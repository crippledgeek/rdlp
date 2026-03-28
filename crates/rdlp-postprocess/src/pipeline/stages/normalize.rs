//! NormalizeStage — normalizes audio levels in media files.
//!
//! This stage runs at index 2 when `config.normalize_audio` is true.
//! Supports peak mode and EBU R128 loudnorm two-pass mode.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::{
    AudioNormMode, FFmpegRunner, LoudnormPreset, NormalizeOptions, PostProcessError,
};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Normalizes audio in the primary current file.
///
/// `should_run` triggers when `config.normalize_audio` is true.
pub struct NormalizeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl NormalizeStage {
    /// Create a new `NormalizeStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build normalization options from `PostProcessConfig`.
    pub(crate) fn build_options(config: &rdlp_core::PostProcessConfig) -> NormalizeOptions {
        let mode = if config.loudnorm {
            AudioNormMode::Loudnorm
        } else {
            AudioNormMode::Peak
        };

        let (default_i, default_tp, default_lra) = config
            .loudnorm_preset
            .as_deref()
            .and_then(|s| s.parse::<LoudnormPreset>().ok())
            .unwrap_or(LoudnormPreset::Streaming)
            .targets();

        NormalizeOptions {
            mode,
            target_peak_db: config.audio_gain_target.unwrap_or(-1.0),
            target_i: config.loudnorm_target_i.unwrap_or(default_i),
            target_tp: config.loudnorm_target_tp.unwrap_or(default_tp),
            target_lra: config.loudnorm_target_lra.unwrap_or(default_lra),
            salvage: true,
            force_dynamic: config.loudnorm_dynamic,
            precompress: config.loudnorm_precompress,
            boost_enabled: config.normalize_boost,
            boost_gain_db: config.normalize_boost_db.unwrap_or(12.0),
        }
    }
}

#[async_trait]
impl PipelineStage for NormalizeStage {
    fn name(&self) -> &str {
        "NormalizeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.normalize_audio
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        let media_info = self
            .ffmpeg
            .probe(&input_file)
            .await
            .context("normalize stage: failed to probe input file")?;
        if !media_info.has_audio {
            return Err(PostProcessError::NoAudioStream.into());
        }

        let opts = Self::build_options(&msg.config);
        let mode_name = match opts.mode {
            AudioNormMode::Peak => "peak",
            AudioNormMode::Loudnorm => "loudnorm (EBU R128)",
        };

        info!(
            "NormalizeStage: normalizing audio ({}) for {}",
            mode_name,
            input_file.display()
        );

        let ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_else(|| {
                debug!("NormalizeStage: no extension, defaulting to mp4");
                "mp4"
            });

        let output_path = msg.tracker.temp_path(&input_file, ext);

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
        let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
            Arc::new(move |frac| cb.on_progress(frac))
        });

        self.ffmpeg
            .normalize_audio(&input_file, &output_path, &opts, callback)
            .await
            .context("normalize stage failed")?;

        // Capture the encoding_tool for downstream pass-through stages.
        msg.encoding_tool = Some(format!("normalize ({})", mode_name));

        info!(
            "NormalizeStage: normalization complete: {}",
            output_path.display()
        );

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
    use rdlp_types::InfoDict;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcessConfig) -> PipelineMessage {
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
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        }
    }

    #[test]
    fn should_run_when_normalize_audio() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = NormalizeStage::new(ffmpeg);

        let config = PostProcessConfig {
            normalize_audio: true,
            ..PostProcessConfig::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = NormalizeStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcessConfig::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = NormalizeStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }

    #[test]
    fn build_options_peak_defaults() {
        let config = PostProcessConfig {
            normalize_audio: true,
            ..PostProcessConfig::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Peak);
        assert!((opts.target_peak_db - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn build_options_loudnorm_streaming_preset() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            ..PostProcessConfig::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Loudnorm);
        // Default preset is streaming: I=-14, TP=-1, LRA=11
        assert!((opts.target_i - (-14.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-1.0)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 11.0).abs() < f64::EPSILON);
    }

    #[test]
    fn build_options_loudnorm_broadcast_preset() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_string()),
            ..PostProcessConfig::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert!((opts.target_i - (-23.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-2.0)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn build_options_individual_overrides() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_string()),
            loudnorm_target_i: Some(-16.0),
            loudnorm_target_tp: Some(-1.5),
            ..PostProcessConfig::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert!((opts.target_i - (-16.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-1.5)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 7.0).abs() < f64::EPSILON); // broadcast default
    }

    #[test]
    fn build_options_boost_enabled() {
        let config = PostProcessConfig {
            normalize_audio: true,
            normalize_boost: true,
            normalize_boost_db: Some(8.0),
            ..PostProcessConfig::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert!(opts.boost_enabled);
        assert!((opts.boost_gain_db - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn build_options_boost_default_gain() {
        let config = PostProcessConfig {
            normalize_audio: true,
            normalize_boost: true,
            normalize_boost_db: None,
            ..PostProcessConfig::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert!(opts.boost_enabled);
        assert!((opts.boost_gain_db - 12.0).abs() < f64::EPSILON);
    }
}
