//! `SubtitleStage` — embeds subtitle files into video containers.
//!
//! This stage runs at index 5 when `config.embed_subtitles` is true.
//! Non-fatal: subtitle failure logs a warning and passes through.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, warn};

use rdlp_ffmpeg::FFmpegRunner;

use crate::pipeline::{DiscoveredSidecar, PipelineMessage, PipelineStage, SidecarOwnership};

/// Subtitle file extensions to search for.
const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "vtt", "ass", "ssa", "lrc"];

/// Containers that support subtitle embedding.
const SUBTITLE_CONTAINERS: &[&str] = &["mp4", "m4a", "m4v", "mov", "mkv", "mka", "webm"];

/// Embeds subtitle streams into video containers.
///
/// `should_run` triggers when `config.embed_subtitles` is true.
/// Non-fatal: failures push a warning and pass through unchanged.
pub struct SubtitleStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl SubtitleStage {
    /// Create a new `SubtitleStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Check if the container format supports subtitle embedding.
    fn supports_subtitles(extension: &str) -> bool {
        SUBTITLE_CONTAINERS
            .iter()
            .any(|c| c.eq_ignore_ascii_case(extension))
    }

    /// Get the `FFmpeg` subtitle codec for a container format.
    fn subtitle_codec_for_container(container_ext: &str) -> &'static str {
        if ["mp4", "m4a", "m4v", "mov"]
            .iter()
            .any(|c| c.eq_ignore_ascii_case(container_ext))
        {
            "mov_text"
        } else if ["mkv", "mka"]
            .iter()
            .any(|c| c.eq_ignore_ascii_case(container_ext))
        {
            "srt"
        } else if container_ext.eq_ignore_ascii_case("webm") {
            "webvtt"
        } else {
            "srt"
        }
    }

    /// Find subtitle files alongside a media file using `original_stem` for discovery.
    ///
    /// Searches for `{original_stem}.{lang}.{ext}` patterns.
    ///
    /// Uses `tokio::fs::read_dir` so the async runtime thread isn't stalled
    /// by blocking directory-scan syscalls on slow / network filesystems.
    async fn find_subtitle_files(
        media_file: &Path,
        original_stem: &str,
    ) -> Vec<(String, DiscoveredSidecar)> {
        let Some(parent) = media_file.parent() else {
            return Vec::new();
        };

        // Also try current file stem as fallback.
        let current_stem = media_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let candidates: Vec<&str> = if original_stem != current_stem && !original_stem.is_empty() {
            vec![original_stem, current_stem]
        } else {
            vec![original_stem]
        };

        // Read the directory once — entries are reused across candidate stems.
        let mut entries = match tokio::fs::read_dir(parent).await {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    "SubtitleStage: could not enumerate {} ({e}); no subtitle files will be discovered",
                    parent.display()
                );
                return Vec::new();
            }
        };
        let mut dir_entries: Vec<PathBuf> = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                dir_entries.push(path);
            }
        }

        let mut result = Vec::new();

        for candidate in &candidates {
            for path in &dir_entries {
                let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
                    continue;
                };

                for sub_ext in SUBTITLE_EXTENSIONS {
                    let Some(without_ext) = filename
                        .strip_suffix(sub_ext)
                        .and_then(|s| s.strip_suffix('.'))
                    else {
                        continue;
                    };

                    let Some(lang) = without_ext
                        .strip_prefix(*candidate)
                        .and_then(|s| s.strip_prefix('.'))
                    else {
                        continue;
                    };

                    if !lang.is_empty() {
                        result.push((lang.to_string(), DiscoveredSidecar::new(path.clone())));
                        break;
                    }
                }
            }
        }

        result.sort_by(|a, b| a.0.cmp(&b.0));
        result.dedup_by(|a, b| a.1 == b.1);
        result
    }
}

#[async_trait]
impl PipelineStage for SubtitleStage {
    fn name(&self) -> &'static str {
        "SubtitleStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.embed_subtitles
    }

    fn is_fatal(&self) -> bool {
        false
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let media_file = msg.tracker.primary();
        let extension = media_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if !Self::supports_subtitles(extension) {
            debug!("SubtitleStage: container '{extension}' does not support subtitle embedding");
            return Ok(msg);
        }

        let subtitle_files = Self::find_subtitle_files(&media_file, &msg.original_stem).await;

        if subtitle_files.is_empty() {
            debug!(
                "SubtitleStage: no subtitle files found for {}",
                media_file.display()
            );
            return Ok(msg);
        }

        let codec = Self::subtitle_codec_for_container(extension);
        // FFmpeg subtitle embedding is not yet implemented in rdlp-ffmpeg.
        // Surface this LOUDLY: a user invoking --embed-subtitles must see a
        // warn-level message AND a structured warning on the pipeline so
        // the API event stream (Event::Warning) can flag the failure.
        // Previously this was an info!("would embed …") line that buried the
        // gap; users got a video without subtitles and no error.
        let _ = &self.ffmpeg;
        let langs: Vec<String> = subtitle_files
            .iter()
            .map(|(lang, _)| lang.clone())
            .collect();
        warn!(
            "SubtitleStage: --embed-subtitles is configured but FFmpeg \
             subtitle muxing is not yet implemented. {} subtitle file(s) \
             ({}) were sidecar-written but NOT embedded into {}. \
             Track: https://github.com/crippledgeek/rdlp/issues (subtitle-embedding).",
            subtitle_files.len(),
            langs.join(", "),
            media_file.display()
        );
        for (lang, sub_path) in &subtitle_files {
            debug!(
                "SubtitleStage: pending-implementation. lang={lang} codec={codec} path={}",
                sub_path.path().display()
            );
        }

        msg.warnings.push(format!(
            "Subtitle embedding not implemented; {} subtitle(s) ({}) written \
             alongside the video but not muxed in.",
            subtitle_files.len(),
            langs.join(", "),
        ));

        // Mark subtitle files as temps unless write_subtitles is set — and
        // never when they are the user's own files sitting next to a borrowed
        // input. `find_subtitle_files` discovers them by stem-matching, which
        // cannot tell an rdlp-fetched sidecar from one the user already had;
        // see `SidecarOwnership`. Same defect the thumbnail sidecar had.
        if !msg.config.write_subtitles {
            let ownership = SidecarOwnership::of(&msg);
            for (_, sidecar) in subtitle_files {
                if let Some(path) = sidecar.into_disposable(ownership) {
                    msg.tracker.mark_temp(path);
                }
            }
        }

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcess) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(config),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn should_run_when_embed_subtitles() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = SubtitleStage::new(ffmpeg);

        let config = PostProcess {
            embed_subtitles: true,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = SubtitleStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcess::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_not_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = SubtitleStage::new(ffmpeg);
        assert!(!stage.is_fatal());
    }

    #[test]
    fn supports_subtitle_containers() {
        assert!(SubtitleStage::supports_subtitles("mp4"));
        assert!(SubtitleStage::supports_subtitles("mkv"));
        assert!(SubtitleStage::supports_subtitles("webm"));
        assert!(!SubtitleStage::supports_subtitles("ts"));
        assert!(!SubtitleStage::supports_subtitles("avi"));
    }

    #[test]
    fn subtitle_codec_for_mp4() {
        assert_eq!(
            SubtitleStage::subtitle_codec_for_container("mp4"),
            "mov_text"
        );
    }

    #[test]
    fn subtitle_codec_for_mkv() {
        assert_eq!(SubtitleStage::subtitle_codec_for_container("mkv"), "srt");
    }

    #[test]
    fn subtitle_codec_for_webm() {
        assert_eq!(
            SubtitleStage::subtitle_codec_for_container("webm"),
            "webvtt"
        );
    }

    #[tokio::test]
    async fn find_subtitle_files_returns_empty_for_missing() {
        let result = SubtitleStage::find_subtitle_files(
            &PathBuf::from("/nonexistent/path/video.mp4"),
            "video",
        )
        .await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn process_passes_through_when_no_subtitles() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = SubtitleStage::new(ffmpeg);

        let config = PostProcess {
            embed_subtitles: true,
            ..PostProcess::default()
        };
        let mut msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        msg.original_stem = "video".to_string();

        let result = stage.process(msg).await;
        assert!(result.is_ok());
    }

    /// Negative test for the C1 fix: when subtitle files are discovered but
    /// the `FFmpeg` muxer is not yet implemented, the stage MUST push a
    /// structured warning onto `msg.warnings` so the API event stream
    /// surfaces it as `Event::Warning`. Previously the stage emitted
    /// `info!("would embed subtitle …")` which the user never saw.
    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // test-only fixture I/O
    async fn embed_subtitles_pushes_warning_when_unimplemented() {
        let dir = tempfile::tempdir().unwrap();
        let video_path = dir.path().join("video.mp4");
        std::fs::write(&video_path, b"fake video").unwrap();
        let sub_path = dir.path().join("video.en.srt");
        std::fs::write(&sub_path, b"1\n00:00:00,000 --> 00:00:01,000\nhi\n").unwrap();

        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = SubtitleStage::new(ffmpeg);

        let config = PostProcess {
            embed_subtitles: true,
            ..PostProcess::default()
        };
        let mut msg = make_msg(vec![video_path.clone()], config);
        msg.original_stem = "video".to_string();

        let processed = stage.process(msg).await.unwrap();
        assert!(
            processed
                .warnings
                .iter()
                .any(|w| w.contains("Subtitle embedding not implemented")),
            "expected NOT-IMPLEMENTED warning in msg.warnings, got: {:?}",
            processed.warnings
        );
    }
}
