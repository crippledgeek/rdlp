//! FFmpeg audio normalization post-processor.
//!
//! Normalizes audio levels in media files using either peak/gain mode
//! (volume + alimiter filters) or EBU R128 loudnorm two-pass mode.
//! Video streams are copied without re-encoding.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessCallback, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use rdlp_ffmpeg::{AudioNormMode, LoudnormPreset, NormalizeOptions, PostProcessError};

ffmpeg_processor!(
    FFmpegNormalizeAudio,
    "FFmpegNormalizeAudio",
    48,
    "Post-processor that normalizes audio levels in media files.\n\n\
     Supports two modes:\n\
     - **Peak**: Analyzes peak/RMS via astats, applies volume + alimiter\n\
     - **Loudnorm**: EBU R128 two-pass normalization\n\n\
     # Priority\n\
     This processor has priority 48 (runs after audio extraction, before remuxing).\n\n\
     # When it runs\n\
     When `normalize_audio` is true in config."
);

impl FFmpegNormalizeAudio {
    /// Build normalization options from PostProcessConfig.
    ///
    /// Resolution order: preset first (defaults to streaming), then individual overrides.
    fn build_options(config: &PostProcessConfig) -> NormalizeOptions {
        let mode = if config.loudnorm {
            AudioNormMode::Loudnorm
        } else {
            AudioNormMode::Peak
        };

        // Resolve preset → base targets, then allow individual overrides
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
impl PostProcessor for FFmpegNormalizeAudio {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.normalize_audio
    }

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
        callback: Option<Arc<dyn PostProcessCallback>>,
    ) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let input_file = &files[0];

        // Verify audio exists
        let media_info = self.ffmpeg.probe(input_file).await?;
        if !media_info.has_audio {
            return Err(PostProcessError::NoAudioStream.into());
        }

        let opts = Self::build_options(config);
        let mode_name = match opts.mode {
            AudioNormMode::Peak => "peak",
            AudioNormMode::Loudnorm => "loudnorm (EBU R128)",
        };

        info!(
            "Normalizing audio ({mode_name}) for {}",
            input_file.display()
        );

        // Build temp output path
        let ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_else(|| {
                debug!(
                    file:? = input_file.display();
                    "No file extension detected, defaulting to mp4"
                );
                "mp4"
            });
        let stem = input_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let output_path = input_file.with_file_name(format!("{stem}.norm.{ext}"));

        let progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>> =
            callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                Arc::new(move |frac| cb.on_progress(frac))
            });
        self.ffmpeg
            .normalize_audio(input_file, &output_path, &opts, progress_fn)
            .await?;

        info!("Audio normalization complete: {}", output_path.display());

        Ok(PostProcessResult {
            info: info.clone(),
            files: vec![output_path],
            temp_files: if config.keep_video { Vec::new() } else { files },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_build_options_peak_defaults() {
        let config = PostProcessConfig {
            normalize_audio: true,
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Peak);
        assert!((opts.target_peak_db - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_options_loudnorm() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Loudnorm);
        // Default preset is streaming: I=-14, TP=-1, LRA=11
        assert!((opts.target_i - (-14.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-1.0)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 11.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_options_custom_target() {
        let config = PostProcessConfig {
            normalize_audio: true,
            audio_gain_target: Some(-3.0),
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert!((opts.target_peak_db - (-3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_options_loudnorm_broadcast_preset() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_string()),
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Loudnorm);
        assert!((opts.target_i - (-23.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-2.0)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_options_individual_overrides() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_string()),
            loudnorm_target_i: Some(-16.0),
            loudnorm_target_tp: Some(-1.5),
            // lra not overridden → should use broadcast default (7.0)
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert!((opts.target_i - (-16.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-1.5)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_options_boost_enabled() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            normalize_boost: true,
            normalize_boost_db: Some(8.0),
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert!(opts.boost_enabled);
        assert!((opts.boost_gain_db - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_options_boost_default_gain() {
        let config = PostProcessConfig {
            normalize_audio: true,
            loudnorm: true,
            normalize_boost: true,
            normalize_boost_db: None,
            ..PostProcessConfig::default()
        };
        let opts = FFmpegNormalizeAudio::build_options(&config);
        assert!(opts.boost_enabled);
        assert!((opts.boost_gain_db - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_priority() {
        if let Ok(ffmpeg) = rdlp_ffmpeg::FFmpegRunner::new() {
            let processor = FFmpegNormalizeAudio::new(Arc::new(ffmpeg));
            assert_eq!(processor.priority(), 48);
        }
    }
}
