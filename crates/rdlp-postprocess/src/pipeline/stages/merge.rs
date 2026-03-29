//! MergeStage — merges separate video and audio streams into a single file.
//!
//! This stage runs first (index 0) when there are 2+ current files.
//! Uses `rdlp_ffmpeg::FFmpegRunner::merge()` via stream copy (no re-encoding).

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Merges separate video + audio streams into a single container.
///
/// `should_run` triggers when `tracker.current_files.len() >= 2`.
/// Uses `FileTracker::temp_path` for output — no naming collisions.
pub struct MergeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl MergeStage {
    /// Create a new `MergeStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Determine the output format from config and input extensions.
    fn determine_output_format(
        config: &rdlp_types::PostProcess,
        video_ext: Option<&str>,
        audio_ext: Option<&str>,
    ) -> &'static str {
        if let Some(format) = config.merge_output_format {
            return format.as_ext();
        }
        match (video_ext, audio_ext) {
            (Some("webm"), _) | (_, Some("webm")) => "mkv",
            _ => {
                debug!("No merge output format configured; defaulting to MP4");
                "mp4"
            }
        }
    }
}

#[async_trait]
impl PipelineStage for MergeStage {
    fn name(&self) -> &str {
        "MergeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.tracker.current_files.len() >= 2
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        let files = &msg.tracker.current_files;

        if files.len() < 2 {
            return Ok(msg);
        }

        info!(
            "MergeStage: merging {} streams into single file",
            files.len()
        );

        // Probe to determine which is video and which is audio.
        let (video_file, audio_file) = if files.len() == 2 {
            let info1 = self
                .ffmpeg
                .probe(&files[0])
                .await
                .context("merge stage: failed to probe first input file")?;
            let info2 = self
                .ffmpeg
                .probe(&files[1])
                .await
                .context("merge stage: failed to probe second input file")?;

            if info1.has_video && !info1.has_audio && info2.has_audio {
                (files[0].clone(), files[1].clone())
            } else if info2.has_video && !info2.has_audio && info1.has_audio {
                (files[1].clone(), files[0].clone())
            } else if info1.has_video {
                (files[0].clone(), files[1].clone())
            } else {
                (files[1].clone(), files[0].clone())
            }
        } else {
            // More than 2 files — assume first is video, second is audio.
            (files[0].clone(), files[1].clone())
        };

        debug!(
            "MergeStage: video={}, audio={}",
            video_file.display(),
            audio_file.display()
        );

        let video_ext = video_file.extension().and_then(|e| e.to_str());
        let audio_ext = audio_file.extension().and_then(|e| e.to_str());
        let output_format = Self::determine_output_format(&msg.config, video_ext, audio_ext);

        // Use tracker.temp_path — no naming collision possible.
        let output_path = msg.tracker.temp_path(&video_file, output_format);

        let opts = RemuxOptions {
            faststart: matches!(output_format, "mp4" | "mov"),
            encoding_tool_override: msg.encoding_tool.clone(),
            ..Default::default()
        };

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
        let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
            Arc::new(move |frac| cb.on_progress(frac))
        });

        self.ffmpeg
            .merge(&video_file, &audio_file, &output_path, &opts, callback)
            .await
            .context("merge stage failed")?;

        info!("MergeStage: merged to {}", output_path.display());

        // Promote output — input files become temps.
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

    use rdlp_types::PostProcess;
    use rdlp_types::InfoDict;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>) -> PipelineMessage {
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
            config: Arc::new(PostProcess::default()),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        }
    }

    #[test]
    fn should_run_requires_two_files() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MergeStage::new(ffmpeg);

        let msg_one = make_msg(vec![PathBuf::from("/tmp/video.mp4")]);
        assert!(!stage.should_run(&msg_one));

        let msg_two = make_msg(vec![
            PathBuf::from("/tmp/video.mp4"),
            PathBuf::from("/tmp/audio.m4a"),
        ]);
        assert!(stage.should_run(&msg_two));
    }

    #[test]
    fn should_not_run_with_zero_files() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MergeStage::new(ffmpeg);
        let msg = make_msg(vec![]);
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn determine_output_format_explicit_config() {
        let config = PostProcess {
            merge_output_format: Some(rdlp_types::ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        assert_eq!(
            MergeStage::determine_output_format(&config, Some("mp4"), Some("m4a")),
            "mkv"
        );
    }

    #[test]
    fn determine_output_format_webm_defaults_to_mkv() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(&config, Some("webm"), Some("opus")),
            "mkv"
        );
    }

    #[test]
    fn determine_output_format_default_mp4() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(&config, Some("mp4"), Some("m4a")),
            "mp4"
        );
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MergeStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }
}
