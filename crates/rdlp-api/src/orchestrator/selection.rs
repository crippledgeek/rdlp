//! Format selection logic and [`DownloadPlan`] construction

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::{DownloadPlan, Orchestrator, errors::*};
use log::{debug, info, warn};
use rdlp_core::{Format, FormatSelector, InfoDict};

// Merge download (video + audio muxed via FFmpeg) is fully supported.

impl Orchestrator {
    /// Resolve the effective format selector string based on runtime
    /// capabilities.
    ///
    /// Priority:
    /// 1. Explicit user format (`config.format = Some(...)`) always wins
    /// 2. Otherwise, compute default from:
    ///    - `ffmpeg_available` -- from `postprocessor_registry.is_some()`
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
        let ffmpeg_available = self.postprocessor_registry.is_some();
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
            .expect("interactive callback is set for interactive mode");
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

#[cfg(test)]
mod tests {
    use crate::events::Event;
    use crate::handle::DownloadId;
    use crate::orchestrator::{DownloadPlan, Orchestrator};
    use rdlp_core::{Config, DownloadProtocol, Format, InfoDict};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// Unwrap a `DownloadPlan::Single`, panicking on `Merge`.
    fn unwrap_single(plan: DownloadPlan) -> Format {
        match plan {
            DownloadPlan::Single(f) => f,
            other => panic!("Expected Single, got {other}"),
        }
    }

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

    fn make_combined(id: &str, height: u32, quality: i32) -> Format {
        let mut f = Format::new(id, format!("url_{id}"), "mp4", DownloadProtocol::Https);
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("aac".to_string());
        f.height = Some(height);
        f.quality = Some(quality);
        f.tbr = Some(height as f64 * 2.0);
        f
    }

    fn make_video_only(id: &str, height: u32) -> Format {
        let mut f = Format::new(id, format!("url_{id}"), "mp4", DownloadProtocol::Https);
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("none".to_string());
        f.height = Some(height);
        f.vbr = Some(height as f64 * 1.5);
        f
    }

    fn make_audio_only(id: &str, abr: f64) -> Format {
        let mut f = Format::new(id, format!("url_{id}"), "m4a", DownloadProtocol::Https);
        f.vcodec = Some("none".to_string());
        f.acodec = Some("aac".to_string());
        f.abr = Some(abr);
        f
    }

    fn test_info_with_formats(formats: Vec<Format>) -> InfoDict {
        let mut info = InfoDict::new(
            "test_id",
            "Test Video",
            "TestExtractor",
            "https://example.com/video",
        );
        info.formats = formats;
        info
    }

    #[tokio::test]
    async fn test_merge_selector_returns_merge_plan() {
        let config = Config {
            format: Some("bv+ba".to_string()),
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);

        let formats = vec![
            make_combined("c720", 720, 2),
            make_combined("c1080", 1080, 3),
            make_video_only("v1440", 1440),
            make_audio_only("a256", 256.0),
        ];
        let info = test_info_with_formats(formats);

        let plan = orch.select_format(&info, false).await.unwrap().unwrap();
        // Should return Merge plan for bv+ba selector
        match plan {
            DownloadPlan::Merge { video, audio } => {
                assert!(video.has_video(), "Video format should have video");
                assert!(audio.has_audio(), "Audio format should have audio");
            }
            DownloadPlan::Single(f) => {
                panic!("Expected Merge plan, got Single({})", f.format_id);
            }
        }
    }

    #[tokio::test]
    async fn test_default_selector_used_when_format_is_none() {
        let config = Config::default(); // format: None
        let orch = orchestrator_with_config(config);

        let formats = vec![
            make_combined("c720", 720, 2),
            make_combined("c1080", 1080, 3),
        ];
        let info = test_info_with_formats(formats);

        // Should succeed using the dynamic default selector
        let plan = orch.select_format(&info, false).await.unwrap().unwrap();
        let result = unwrap_single(plan);
        // Best combined should be selected (c1080)
        assert_eq!(result.format_id, "c1080");
    }

    #[test]
    fn test_resolve_default_selector_matches_environment() {
        // The resolved selector depends on FFmpeg availability,
        // which varies by environment. Assert the correct value
        // for whichever environment we're in.
        let config = Config::default();
        let orch = orchestrator_with_config(config);
        let selector = orch.resolve_effective_selector();
        if orch.postprocessor_registry.is_some() {
            assert_eq!(
                selector, "bv*+ba/b",
                "FFmpeg available + merge supported should give bv*+ba/b"
            );
        } else {
            assert_eq!(selector, "b/bv+ba", "No FFmpeg should give b/bv+ba");
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
        if orch.postprocessor_registry.is_some() {
            assert_eq!(selector, "bv+ba/b");
        } else {
            assert_eq!(selector, "b/bv+ba");
        }
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

    /// Verify the selector truth table (merge supported):
    ///
    /// | Condition                              | Default          |
    /// |----------------------------------------|------------------|
    /// | FFmpeg available, no multistreams       | `bv*+ba/b`       |
    /// | FFmpeg available, multistreams          | `bv+ba/b`        |
    /// | FFmpeg unavailable                      | `b/bv+ba`        |
    /// | User explicit `-f`                      | User's value     |
    #[test]
    fn test_selector_truth_table() {
        // Case 1: FFmpeg available, merge supported
        // (depends on environment -- tested via unit logic)
        let config = Config::default();
        let orch = orchestrator_with_config(config);
        let selector = orch.resolve_effective_selector();
        if orch.postprocessor_registry.is_some() {
            assert_eq!(
                selector, "bv*+ba/b",
                "FFmpeg available + merge supported should give bv*+ba/b"
            );
        } else {
            assert_eq!(selector, "b/bv+ba", "No FFmpeg should give b/bv+ba");
        }

        // Case 2: audio_multistreams with FFmpeg gives bv+ba/b,
        // without FFmpeg gives b/bv+ba
        let config = Config {
            audio_multistreams: true,
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);
        if orch.postprocessor_registry.is_some() {
            assert_eq!(
                orch.resolve_effective_selector(),
                "bv+ba/b",
                "multistreams + FFmpeg should give bv+ba/b"
            );
        } else {
            assert_eq!(
                orch.resolve_effective_selector(),
                "b/bv+ba",
                "multistreams + no FFmpeg should give b/bv+ba"
            );
        }

        // Case 3: Explicit format always wins
        let config = Config {
            format: Some("worst".to_string()),
            audio_multistreams: true,
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);
        assert_eq!(
            orch.resolve_effective_selector(),
            "worst",
            "Explicit format should always win"
        );

        // Case 4: Another explicit format
        let config = Config {
            format: Some("bv[height<=720]+ba".to_string()),
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);
        assert_eq!(
            orch.resolve_effective_selector(),
            "bv[height<=720]+ba",
            "Explicit complex format should be preserved exactly"
        );
    }

    #[tokio::test]
    async fn test_merge_plan_returned_for_explicit_merge_selector() {
        // Force selector to request merge (bv+ba)
        let config = Config {
            format: Some("bv+ba".to_string()),
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);

        let formats = vec![
            make_combined("c720", 720, 2),
            make_video_only("v1080", 1080),
            make_audio_only("a256", 256.0),
        ];
        let info = test_info_with_formats(formats);

        let plan = orch.select_format(&info, false).await.unwrap().unwrap();
        match plan {
            DownloadPlan::Merge { video, audio } => {
                assert!(video.has_video(), "Merge video format should have video");
                assert!(audio.has_audio(), "Merge audio format should have audio");
            }
            DownloadPlan::Single(f) => {
                panic!("Expected Merge plan, got Single({})", f.format_id);
            }
        }
    }

    #[tokio::test]
    async fn test_single_format_returns_single_plan() {
        let config = Config {
            format: Some("b".to_string()),
            ..Default::default()
        };
        let orch = orchestrator_with_config(config);

        let formats = vec![
            make_combined("c720", 720, 2),
            make_combined("c1080", 1080, 3),
        ];
        let info = test_info_with_formats(formats);

        let plan = orch.select_format(&info, false).await.unwrap().unwrap();
        let format = unwrap_single(plan);
        assert_eq!(
            format.format_id, "c1080",
            "Best single format should be c1080"
        );
    }

    #[tokio::test]
    async fn test_default_selector_returns_merge_with_ffmpeg() {
        // Default (no explicit format) with FFmpeg should use bv*+ba/b
        // which prefers merge when better video-only exists
        let config = Config::default();
        let orch = orchestrator_with_config(config);

        if orch.postprocessor_registry.is_none() {
            // Skip if no FFmpeg
            return;
        }

        let formats = vec![
            make_combined("c720", 720, 2),
            make_video_only("v1080", 1080),
            make_audio_only("a256", 256.0),
        ];
        let info = test_info_with_formats(formats);

        let plan = orch.select_format(&info, false).await.unwrap().unwrap();
        assert!(
            matches!(plan, DownloadPlan::Merge { .. }),
            "Default selector with FFmpeg should prefer merge when better quality available"
        );
    }
}
