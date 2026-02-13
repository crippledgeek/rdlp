//! Format selection logic

use std::collections::BTreeMap;

use super::{Orchestrator, errors::*};
use dialoguer::{Select, theme::ColorfulTheme};
use log::{info, warn};
use rdlp_core::{Format, FormatSelector, InfoDict};
use rdlp_table;

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
        let format = if interactive {
            let Some(format) = self.select_format_interactive(info).await? else {
                info!("Selection cancelled by user");
                return Ok(None);
            };
            format
        } else {
            let format_selector = FormatSelector::parse(&self.config.format)
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

    /// Interactive format selection menu
    ///
    /// Displays a menu of available formats and lets the user select one.
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (ESC pressed)
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

        // Compute dynamic size column width from actual content
        let size_width = grouped
            .iter()
            .map(|f| f.size_text().len())
            .max()
            .unwrap_or(4)
            .max(4);

        // Build menu items with format details
        let items: Vec<String> = grouped.iter().map(|f| f.table_row(size_width)).collect();

        info!("Available formats:");
        info!(
            "{:<qw$} | {:<rw$} | {:<size_width$} | {:<tw$} | Codecs",
            "Quality",
            "Resolution",
            "Size",
            "Type",
            qw = rdlp_table::QUALITY_WIDTH,
            rw = rdlp_table::RESOLUTION_WIDTH,
            tw = rdlp_table::TYPE_WIDTH,
        );
        info!("{}", "-".repeat(rdlp_table::separator_width(size_width)));

        let selection = tokio::task::spawn_blocking(move || {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select a format to download (ESC to cancel)")
                .items(&items)
                .default(0)
                .interact_opt()
        })
        .await
        .map_err(|e| OrchestratorError::Io(std::io::Error::other(e)))?
        .map_err(|e| OrchestratorError::Io(e.into()))?;

        Ok(selection.map(|index| grouped[index].clone()))
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
