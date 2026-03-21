//! AudioExtractStage — extracts audio from video files.
//!
//! This stage runs at index 1 when `config.extract_audio` is true.
//! Uses `rdlp_ffmpeg::FFmpegRunner::extract_audio()`.

use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::ffmpeg::{AudioCodecConfig, AudioExtractOptions, get_audio_codec};
use rdlp_ffmpeg::{FFmpegRunner, PostProcessError};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Extracts audio from the primary current file.
///
/// `should_run` triggers when `config.extract_audio` is true.
pub struct AudioExtractStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl AudioExtractStage {
    /// Create a new `AudioExtractStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build extraction options from codec config and quality string.
    pub(crate) fn build_extract_options(
        codec_config: &AudioCodecConfig,
        can_copy: bool,
        quality: Option<&str>,
    ) -> AudioExtractOptions {
        let mut opts = AudioExtractOptions {
            encoder_name: codec_config.encoder.map(String::from),
            copy: can_copy,
            ..Default::default()
        };

        if can_copy {
            return opts;
        }

        if let Some(q) = quality {
            if let Ok(q_num) = q.parse::<u32>() {
                if let Some((min, max)) = codec_config.bitrate_range {
                    opts.bitrate_kbps = Some(q_num.clamp(min, max));
                }
            } else if let Some((worst, best)) = codec_config.quality_scale {
                let scale = if q.eq_ignore_ascii_case("best") || q == "0" {
                    best
                } else if q.eq_ignore_ascii_case("worst") || q == "9" || q == "10" {
                    worst
                } else {
                    q.parse().unwrap_or((worst + best) / 2)
                };
                opts.quality_scale = Some(scale as i32);
            }
        }

        opts
    }
}

#[async_trait]
impl PipelineStage for AudioExtractStage {
    fn name(&self) -> &str {
        "AudioExtractStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.extract_audio
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        // Check if input has audio.
        let media_info = self.ffmpeg.probe(&input_file).await?;
        if !media_info.has_audio {
            return Err(PostProcessError::NoAudioStream.into());
        }

        let target_format = match msg.config.audio_format {
            Some(f) => f.codec_name(),
            None => {
                debug!("AudioExtractStage: no audio format configured; defaulting to MP3");
                "mp3"
            }
        };

        let codec_config =
            get_audio_codec(target_format).ok_or_else(|| PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "audio extraction".to_string(),
            })?;

        let can_copy = media_info
            .audio_codec
            .as_ref()
            .is_some_and(|c| c == target_format || (c == "aac" && target_format == "m4a"));

        if can_copy {
            debug!(
                "AudioExtractStage: audio codec matches target {}, copying stream",
                target_format
            );
        }

        let output_path = msg.tracker.temp_path(&input_file, codec_config.extension);

        let opts = Self::build_extract_options(
            codec_config,
            can_copy,
            msg.config.audio_quality.as_deref(),
        );

        let callback = msg.callback_factory.as_ref().map(|f| f(self.name())).map(
            |cb| -> Arc<dyn Fn(f64) + Send + Sync> { Arc::new(move |frac| cb.on_progress(frac)) },
        );

        self.ffmpeg
            .extract_audio(&input_file, &output_path, &opts, callback)
            .await?;

        info!(
            "AudioExtractStage: audio extracted to {}",
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

    use rdlp_core::{InfoDict, PostProcessConfig};

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
        }
    }

    #[test]
    fn should_run_when_extract_audio() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = AudioExtractStage::new(ffmpeg);

        let config = PostProcessConfig {
            extract_audio: true,
            ..PostProcessConfig::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = AudioExtractStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcessConfig::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = AudioExtractStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }

    #[test]
    fn build_extract_options_mp3_bitrate() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = AudioExtractStage::build_extract_options(codec_config, false, Some("192"));
        assert_eq!(opts.bitrate_kbps, Some(192));
        assert_eq!(opts.quality_scale, None);
        assert!(!opts.copy);
    }

    #[test]
    fn build_extract_options_copy_ignores_quality() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = AudioExtractStage::build_extract_options(codec_config, true, Some("192"));
        assert!(opts.copy);
        assert_eq!(opts.bitrate_kbps, None);
        assert_eq!(opts.quality_scale, None);
    }

    #[test]
    fn build_extract_options_bitrate_clamped() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = AudioExtractStage::build_extract_options(codec_config, false, Some("999"));
        assert_eq!(opts.bitrate_kbps, Some(320)); // clamped to max
        let opts = AudioExtractStage::build_extract_options(codec_config, false, Some("1"));
        assert_eq!(opts.bitrate_kbps, Some(32)); // clamped to min
    }
}
