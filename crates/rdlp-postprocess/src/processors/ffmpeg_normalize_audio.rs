//! FFmpeg audio normalization post-processor.
//!
//! Normalizes audio levels in media files using either peak/gain mode
//! (volume + alimiter filters) or EBU R128 loudnorm two-pass mode.
//! Video streams are copied without re-encoding.

use std::path::PathBuf;

use async_trait::async_trait;
use log::info;
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use rdlp_ffmpeg::{AudioNormMode, NormalizeOptions, PostProcessError};

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
    fn build_options(config: &PostProcessConfig) -> NormalizeOptions {
        let mode = if config.loudnorm {
            AudioNormMode::Loudnorm
        } else {
            AudioNormMode::Peak
        };

        NormalizeOptions {
            mode,
            target_peak_db: config.audio_gain_target.unwrap_or(-1.0),
            target_i: config.loudnorm_target_i.unwrap_or(-16.0),
            target_tp: config.loudnorm_target_tp.unwrap_or(-1.5),
            target_lra: config.loudnorm_target_lra.unwrap_or(11.0),
            salvage: true,
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
            .unwrap_or("mp4");
        let stem = input_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let output_path = input_file.with_file_name(format!("{stem}.norm.{ext}"));

        self.ffmpeg
            .normalize_audio(input_file, &output_path, &opts)
            .await?;

        info!("Audio normalization complete: {}", output_path.display());

        let temp_files = if config.keep_video {
            Vec::new()
        } else {
            files.clone()
        };

        Ok(PostProcessResult {
            info: info.clone(),
            files: vec![output_path],
            temp_files,
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
        assert!((opts.target_i - (-16.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-1.5)).abs() < f64::EPSILON);
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
    fn test_priority() {
        if let Ok(ffmpeg) = rdlp_ffmpeg::FFmpegRunner::new() {
            let processor = FFmpegNormalizeAudio::new(Arc::new(ffmpeg));
            assert_eq!(processor.priority(), 48);
        }
    }
}
