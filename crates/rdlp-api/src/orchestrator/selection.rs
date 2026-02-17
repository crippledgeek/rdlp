//! Format selection logic

use std::collections::BTreeMap;

use super::{Orchestrator, errors::*};
use log::{info, warn};
use rdlp_core::{Format, FormatSelector, InfoDict};

impl Orchestrator {
    /// Select format from available formats
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (interactive mode only)
    ///
    /// # Errors
    /// Returns an error if:
    /// - Format selector string is invalid
    /// - No suitable format is found (automatic mode)
    pub(super) async fn select_format(
        &self,
        info: &InfoDict,
        interactive: bool,
    ) -> Result<Option<Format>> {
        let formats = &info.formats;
        let format = if interactive && self.interactive.is_some() {
            let Some(format) = self.select_format_interactive(info).await? else {
                info!("Selection cancelled by user");
                return Ok(None);
            };
            format
        } else {
            let selector_str = self
                .config
                .format
                .as_deref()
                .unwrap_or("bestvideo*+bestaudio/best");
            let format_selector = FormatSelector::parse(selector_str)
                .map_err(OrchestratorError::InvalidFormatSelector)?;

            let selected_formats = format_selector.select(formats);
            if selected_formats.is_empty() {
                return Err(OrchestratorError::NoFormat);
            }

            // If merge (2 formats), pick the first one for now.
            // Full merge download (download both + FFmpeg merge) is a future enhancement.
            if selected_formats.len() > 1 {
                info!(
                    video = selected_formats[0].format_id.as_str(),
                    audio = selected_formats[1].format_id.as_str();
                    "Merge requested — downloading primary format (merge not yet supported)"
                );
            }

            selected_formats[0].clone()
        };

        info!(format:?; "Selected format");
        Ok(Some(format))
    }

    /// Interactive format selection via [`InteractiveCallback`]
    ///
    /// Groups available formats and delegates the actual UI to the
    /// frontend-provided callback.
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled
    pub(super) async fn select_format_interactive(
        &self,
        info: &InfoDict,
    ) -> Result<Option<Format>> {
        if info.formats.is_empty() {
            return Err(OrchestratorError::NoFormat);
        }

        // Warn about live streams before showing format table
        if info.is_live.unwrap_or(false) {
            warn!("LIVE stream detected — download may be incomplete");
        }

        // Group formats by (height, is_hls, language), picking the best per group.
        // Language is included so SUB/DUB tracks at the same resolution are separate options.
        let mut by_key: BTreeMap<(u32, bool, Option<&str>), &Format> = BTreeMap::new();
        for fmt in &info.formats {
            let h = fmt.height.unwrap_or(0);
            let hls = fmt.is_hls();
            let lang = fmt.language.as_deref();
            let entry = by_key.entry((h, hls, lang)).or_insert(fmt);
            if pick_better(fmt, entry) {
                *entry = fmt;
            }
        }

        let grouped: Vec<&Format> = by_key.values().copied().collect();

        // Delegate to the interactive callback
        let callback = self.interactive.as_ref().unwrap();
        let grouped_owned: Vec<Format> = grouped.iter().map(|f| (*f).clone()).collect();

        let selection = callback.select_format(&grouped_owned, info).await;

        Ok(selection.map(|index| grouped_owned[index].clone()))
    }
}

/// Returns true if `candidate` is a better pick than `current` within the same group.
///
/// Prefer formats with audio over video-only, then higher quality, then larger filesize.
fn pick_better(candidate: &Format, current: &Format) -> bool {
    // Prefer formats that have audio over video-only
    match (candidate.acodec.is_some(), current.acodec.is_some()) {
        (true, false) => return true,
        (false, true) => return false,
        _ => {}
    }

    // Higher quality score wins
    if candidate.quality != current.quality {
        return candidate.quality > current.quality;
    }

    // Larger filesize wins (more likely to be higher quality)
    matches!(
        (candidate.filesize, current.filesize),
        (Some(a), Some(b)) if a > b
    )
}
