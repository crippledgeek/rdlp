//! Format selection logic and [`DownloadPlan`] construction

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::{DownloadPlan, Orchestrator, errors::*};
use log::{debug, info, warn};
use rdlp_types::{Format, FormatSelector, InfoDict};

#[cfg(test)]
mod tests;

// Merge download (video + audio muxed via FFmpeg) is fully supported.

impl Orchestrator {
    /// Resolve the effective format selector string based on runtime
    /// capabilities.
    ///
    /// Priority:
    /// 1. Explicit user format (`config.format = Some(...)`) always wins
    /// 2. Otherwise, compute default from:
    ///    - `ffmpeg_available` -- from `pipeline.is_some()`
    ///    - `audio_multistreams` -- from `config.audio_multistreams`
    ///
    /// # Defaults
    ///
    /// | Condition | Default |
    /// |-----------|---------|
    /// | FFmpeg available, no multistreams | `bv*+ba/b` |
    /// | FFmpeg available, multistreams | `bv+ba/b` |
    /// | FFmpeg unavailable | `b/bv+ba` |
    /// | User explicit `-f` | User's value |
    pub(super) fn resolve_effective_selector(&self) -> Cow<'_, str> {
        // 1. Explicit user format always wins
        if let Some(ref user_format) = self.config.format {
            debug!(
                selector = user_format.as_str(),
                reason = "explicit user format";
                "Resolved format selector"
            );
            return Cow::Borrowed(user_format.as_str());
        }

        // 2. Compute dynamic default
        let ffmpeg_available = self.pipeline.is_some();
        let audio_multistreams = self.config.audio_multistreams;

        let selector = if ffmpeg_available && !audio_multistreams {
            // FFmpeg + merge: prefer best video (including combined) + best audio
            "bv*+ba/b"
        } else if ffmpeg_available && audio_multistreams {
            // FFmpeg + merge + multistreams: strict video-only + audio-only
            "bv+ba/b"
        } else {
            // No FFmpeg: prefer combined, fallback to video+audio
            "b/bv+ba"
        };

        debug!(
            selector,
            ffmpeg_available,
            audio_multistreams,
            reason = "dynamic default";
            "Resolved format selector"
        );

        Cow::Borrowed(selector)
    }

    /// Select format from available formats and return a [`DownloadPlan`].
    ///
    /// Uses [`resolve_effective_selector`] to determine the selector
    /// string. When the selector returns a merge pair (video + audio),
    /// returns [`DownloadPlan::Merge`]. Otherwise returns a single
    /// format wrapped in [`DownloadPlan::Single`].
    ///
    /// **Note**: Interactive mode always returns `DownloadPlan::Single`
    /// because the user selects a single format from the grouped table.
    ///
    /// Returns `Ok(Some(plan))` on success, `Ok(None)` if the user
    /// cancelled (interactive mode only).
    ///
    /// # Errors
    /// Returns an error if:
    /// - Format selector string is invalid
    /// - No suitable format is found (automatic mode)
    pub(super) async fn select_format(
        &self,
        info: &InfoDict,
        interactive: bool,
    ) -> Result<Option<DownloadPlan>> {
        let formats = &info.formats;
        if interactive && self.interactive.is_some() {
            let Some(format) = self.select_format_interactive(info).await? else {
                info!("Selection cancelled by user");
                return Ok(None);
            };
            info!(format:?; "Selected format");
            return Ok(Some(DownloadPlan::Single(format)));
        }

        let format = {
            let selector_str = self.resolve_effective_selector();
            let format_selector = FormatSelector::parse(&selector_str)
                .map_err(OrchestratorError::InvalidFormatSelector)?;

            let selected_formats = format_selector.select(formats);
            if selected_formats.is_empty() {
                return Err(OrchestratorError::NoFormat);
            }

            // Selector returned a merge pair: first = video, second = audio
            if selected_formats.len() > 1 {
                let video = selected_formats[0].clone();
                let audio = selected_formats[1].clone();
                info!(
                    video = video.format_id.as_str(),
                    audio = audio.format_id.as_str();
                    "Selected merge pair"
                );
                return Ok(Some(DownloadPlan::Merge { video, audio }));
            } else {
                selected_formats[0].clone()
            }
        };

        info!(format:?; "Selected format");
        Ok(Some(DownloadPlan::Single(format)))
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

        // Delegate to the interactive callback
        let callback = self
            .interactive
            .as_ref()
            .ok_or(OrchestratorError::InteractiveNotConfigured)?;
        let grouped_owned: Vec<Format> = by_key.values().map(|f| (*f).clone()).collect();

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
