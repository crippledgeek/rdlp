//! RemuxStage — remuxes to a target container format.
//!
//! This stage runs at index 3. It handles:
//! - Explicit remux via `config.remux_container`
//! - HLS auto-remux via `msg.is_hls` (replaces the old `ffmpeg_remux()` hack)
//!
//! Uses stream copy (no re-encoding). Skipped when audio extraction is active.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Remuxes the current file to a different container.
///
/// Triggers when `config.remux_container.is_some() || msg.is_hls`.
/// Skipped when `config.extract_audio` is true (audio extract produces a
/// standalone audio file that must not be remuxed into a video container).
pub struct RemuxStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl RemuxStage {
    /// Create a new `RemuxStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Determine the target extension for remuxing.
    ///
    /// Returns `None` if the file is already in the target container.
    fn target_ext(msg: &PipelineMessage, input_ext: &str) -> Option<&'static str> {
        if let Some(container) = msg.config.remux_container {
            let ext = container.as_ext();
            if input_ext.eq_ignore_ascii_case(ext) {
                return None; // already in target container
            }
            return Some(ext);
        }
        // HLS auto-remux: .ts → .mp4
        if msg.is_hls && !input_ext.eq_ignore_ascii_case("mp4") {
            return Some("mp4");
        }
        None
    }
}

#[async_trait]
impl PipelineStage for RemuxStage {
    fn name(&self) -> &str {
        "RemuxStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        // Skip when audio extraction is active — extract_audio produces a
        // standalone audio file that should not be remuxed into a video container.
        if msg.config.extract_audio {
            return false;
        }
        msg.config.remux_container.is_some() || msg.is_hls
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();
        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let Some(target_ext) = Self::target_ext(&msg, input_ext) else {
            debug!(
                "RemuxStage: file already in target container ({}), skipping",
                input_ext
            );
            return Ok(msg);
        };

        info!(
            "RemuxStage: remuxing {} → {}",
            input_file.display(),
            target_ext
        );

        let output_path = msg.tracker.temp_path(&input_file, target_ext);

        let opts = RemuxOptions {
            faststart: matches!(target_ext, "mp4" | "mov"),
            output_format: Some(
                msg.config
                    .remux_container
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "mp4".to_string()),
            ),
            encoding_tool_override: msg.encoding_tool.clone(),
        };

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
        let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
            Arc::new(move |frac| cb.on_progress(frac))
        });

        self.ffmpeg
            .remux(&input_file, &output_path, &opts, callback)
            .await
            .context("remux stage failed")?;

        debug!("RemuxStage: remuxed to {}", output_path.display());

        msg.tracker.replace(vec![output_path]);

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_core::PostProcessConfig;
    use rdlp_types::{ContainerFormat, InfoDict};

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg_with_config(
        files: Vec<PathBuf>,
        config: PostProcessConfig,
        is_hls: bool,
    ) -> PipelineMessage {
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
            is_hls,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        }
    }

    #[test]
    fn should_run_with_remux_container() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let config = PostProcessConfig {
            remux_container: Some(ContainerFormat::Mp4),
            ..PostProcessConfig::default()
        };
        let msg = make_msg_with_config(vec![PathBuf::from("/tmp/video.ts")], config, false);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_run_when_is_hls() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let msg = make_msg_with_config(
            vec![PathBuf::from("/tmp/video.ts")],
            PostProcessConfig::default(),
            true,
        );
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_without_remux_or_hls() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let msg = make_msg_with_config(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcessConfig::default(),
            false,
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_when_extract_audio() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let config = PostProcessConfig {
            remux_container: Some(ContainerFormat::Mkv),
            extract_audio: true,
            ..PostProcessConfig::default()
        };
        let msg = make_msg_with_config(vec![PathBuf::from("/tmp/video.ts")], config, false);
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn target_ext_already_in_target_returns_none() {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        let msg = PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "T".to_string(),
                "T".to_string(),
                "https://x.com".to_string(),
            ),
            tracker: FileTracker::new(vec![PathBuf::from("/tmp/v.mp4")], reg),
            config: Arc::new(PostProcessConfig {
                remux_container: Some(ContainerFormat::Mp4),
                ..PostProcessConfig::default()
            }),
            original_stem: "v".into(),
            is_hls: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        };
        assert!(RemuxStage::target_ext(&msg, "mp4").is_none());
    }

    #[test]
    fn target_ext_hls_ts_returns_mp4() {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        let msg = PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "T".to_string(),
                "T".to_string(),
                "https://x.com".to_string(),
            ),
            tracker: FileTracker::new(vec![PathBuf::from("/tmp/v.ts")], reg),
            config: Arc::new(PostProcessConfig::default()),
            original_stem: "v".into(),
            is_hls: true,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        };
        assert_eq!(RemuxStage::target_ext(&msg, "ts"), Some("mp4"));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }
}
