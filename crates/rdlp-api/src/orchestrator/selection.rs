//! Format selection logic

use std::collections::BTreeMap;

use super::{Orchestrator, errors::*};
use log::{debug, info, warn};
use rdlp_core::{Format, FormatSelector, InfoDict};

impl Orchestrator {
    /// Resolve the effective format selector string based on runtime
    /// capabilities.
    ///
    /// Priority:
    /// 1. Explicit user format (`config.format = Some(...)`) always wins
    /// 2. Otherwise, compute default from:
    ///    - `ffmpeg_available` -- from `postprocessor_registry.is_some()`
    ///    - `merge_supported` -- false in Phase 1 (no merge download yet)
    ///    - `audio_multistreams` -- from `config.audio_multistreams`
    ///
    /// # Phase 1 defaults
    ///
    /// | Condition | Default |
    /// |-----------|---------|
    /// | FFmpeg available, merge not supported | `b/bv*+ba` |
    /// | FFmpeg unavailable | `b/bv+ba` |
    /// | audio_multistreams enabled | `b/bv+ba` |
    /// | User explicit `-f` | User's value |
    #[allow(dead_code)] // Wired in Task 4
    pub(super) fn resolve_effective_selector(&self) -> String {
        // 1. Explicit user format always wins
        if let Some(ref user_format) = self.config.format {
            debug!(
                selector = user_format.as_str(),
                reason = "explicit user format";
                "Resolved format selector"
            );
            return user_format.clone();
        }

        // 2. Compute dynamic default
        let ffmpeg_available = self.postprocessor_registry.is_some();
        let merge_supported = false; // Phase 2 will set this to true
        let audio_multistreams = self.config.audio_multistreams;

        let selector = if merge_supported && ffmpeg_available && !audio_multistreams {
            // Phase 2+: full merge with star
            // (video+audio combined included)
            "bv*+ba/b"
        } else if merge_supported && ffmpeg_available && audio_multistreams {
            // Phase 2+: merge without star
            // (strict video-only + audio-only)
            "bv+ba/b"
        } else if ffmpeg_available && !audio_multistreams {
            // Phase 1: FFmpeg available but no merge -- prefer combined,
            // fallback to video-star + audio (star allows combined
            // formats as video candidate)
            "b/bv*+ba"
        } else {
            // No FFmpeg OR multistreams -- only combined or strict
            // video+audio
            "b/bv+ba"
        };

        debug!(
            selector,
            ffmpeg_available,
            merge_supported,
            audio_multistreams,
            reason = "dynamic default";
            "Resolved format selector"
        );

        selector.to_string()
    }

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

#[cfg(test)]
mod tests {
    use crate::events::Event;
    use crate::handle::DownloadId;
    use crate::orchestrator::Orchestrator;
    use rdlp_core::Config;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// Create orchestrator with specific config for testing.
    fn orchestrator_with_config(config: Config) -> Orchestrator {
        let (tx, _rx) = mpsc::channel::<Event>(64);
        Orchestrator::new(
            config,
            tx,
            DownloadId::next(),
            CancellationToken::new(),
            None,
        )
    }

    #[test]
    fn test_resolve_no_ffmpeg_returns_b_bv_plus_ba() {
        let config = Config::default();
        let orch = orchestrator_with_config(config);
        // Without FFmpeg the postprocessor_registry is None
        if orch.postprocessor_registry.is_none() {
            let selector = orch.resolve_effective_selector();
            assert_eq!(selector, "b/bv+ba");
        }
    }

    #[test]
    fn test_resolve_with_ffmpeg_no_merge_returns_b_bvstar_ba() {
        let config = Config::default();
        let orch = orchestrator_with_config(config);
        // When FFmpeg is available the postprocessor_registry is Some
        if orch.postprocessor_registry.is_some() {
            let selector = orch.resolve_effective_selector();
            assert_eq!(selector, "b/bv*+ba");
        }
    }

    #[test]
    fn test_resolve_explicit_format_always_wins() {
        let config = Config {
            format: Some("worst".to_string()),
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);
        let selector = orch.resolve_effective_selector();
        assert_eq!(selector, "worst");
    }

    #[test]
    fn test_resolve_audio_multistreams_with_ffmpeg() {
        let config = Config {
            audio_multistreams: true,
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);
        let selector = orch.resolve_effective_selector();
        // With multistreams, always "b/bv+ba" regardless of FFmpeg
        assert_eq!(selector, "b/bv+ba");
    }

    #[test]
    fn test_resolve_explicit_format_ignores_multistreams() {
        let config = Config {
            format: Some("bestvideo*+bestaudio/best".to_string()),
            audio_multistreams: true,
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);
        let selector = orch.resolve_effective_selector();
        assert_eq!(selector, "bestvideo*+bestaudio/best");
    }
}
