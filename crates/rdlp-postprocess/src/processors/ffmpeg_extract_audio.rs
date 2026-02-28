//! FFmpeg audio extraction post-processor.
//!
//! Extracts and optionally converts audio from video files using
//! `ffmpeg-the-third` library bindings (no CLI process spawning).
//! Supports various output formats including MP3, AAC, Opus, FLAC, etc.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessCallback, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use rdlp_ffmpeg::PostProcessError;
use rdlp_ffmpeg::ffmpeg::{AudioCodecConfig, AudioExtractOptions, get_audio_codec};

ffmpeg_processor!(
    FFmpegExtractAudio,
    "FFmpegExtractAudio",
    50,
    "Post-processor that extracts audio from video files.\n\n\
     # Priority\n\
     This processor has priority 50 (runs after merging).\n\n\
     # When it runs\n\
     - When `extract_audio` is true in config\n\
     - Optionally converts to specified `audio_format`"
);

impl FFmpegExtractAudio {
    /// Build extraction options from codec config and quality string.
    fn build_extract_options(
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
            // Try to parse as numeric bitrate (e.g., "192" for 192kbps)
            if let Ok(q_num) = q.parse::<u32>() {
                if let Some((min, max)) = codec_config.bitrate_range {
                    opts.bitrate_kbps = Some(q_num.clamp(min, max));
                }
            } else if let Some((worst, best)) = codec_config.quality_scale {
                // Non-numeric = VBR quality string (e.g., "best", "worst")
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
impl PostProcessor for FFmpegExtractAudio {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.extract_audio
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

        // Check if input has audio
        let media_info = self.ffmpeg.probe(input_file).await?;
        if !media_info.has_audio {
            return Err(PostProcessError::NoAudioStream.into());
        }

        // Determine target format
        let target_format = match config.audio_format {
            Some(f) => f.codec_name(),
            None => {
                debug!("No audio format configured; defaulting to MP3");
                "mp3"
            }
        };
        let codec_config =
            get_audio_codec(target_format).ok_or_else(|| PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "audio extraction".to_string(),
            })?;

        debug!(
            "Extracting audio from {} to {} format",
            input_file.display(),
            target_format
        );

        // Determine if we can copy or need to transcode
        let can_copy = media_info
            .audio_codec
            .as_ref()
            .is_some_and(|c| c == target_format || (c == "aac" && target_format == "m4a"));

        if can_copy {
            debug!(format:? = target_format; "Audio codec matches target, copying stream");
        }

        // Build output path
        let output_path = input_file.with_extension(codec_config.extension);

        // Build extraction options
        let opts =
            Self::build_extract_options(codec_config, can_copy, config.audio_quality.as_deref());

        // Extract audio via library bindings
        let progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>> =
            callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                Arc::new(move |frac| cb.on_progress(frac))
            });
        self.ffmpeg
            .extract_audio(input_file, &output_path, &opts, progress_fn)
            .await?;

        info!(output:? = output_path.display(); "Audio extracted");

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

    #[test]
    fn test_build_extract_options_mp3_bitrate() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, false, Some("192"));
        assert_eq!(opts.bitrate_kbps, Some(192));
        assert_eq!(opts.quality_scale, None);
        assert!(!opts.copy);
    }

    #[test]
    fn test_build_extract_options_mp3_vbr() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, false, Some("best"));
        assert_eq!(opts.quality_scale, Some(0)); // Best quality for MP3 = 0
        assert_eq!(opts.bitrate_kbps, None);
    }

    #[test]
    fn test_build_extract_options_copy() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, true, Some("192"));
        assert!(opts.copy);
        // Quality settings should be ignored when copying
        assert_eq!(opts.bitrate_kbps, None);
        assert_eq!(opts.quality_scale, None);
    }

    #[test]
    fn test_build_extract_options_opus_bitrate() {
        let codec_config = get_audio_codec("opus").unwrap();
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, false, Some("128"));
        assert_eq!(opts.bitrate_kbps, Some(128));
        assert_eq!(opts.quality_scale, None); // Opus has no quality scale
    }

    #[test]
    fn test_build_extract_options_bitrate_clamping() {
        let codec_config = get_audio_codec("mp3").unwrap();
        // Over max (320)
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, false, Some("999"));
        assert_eq!(opts.bitrate_kbps, Some(320));
        // Under min (32)
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, false, Some("1"));
        assert_eq!(opts.bitrate_kbps, Some(32));
    }

    #[test]
    fn test_build_extract_options_no_quality() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = FFmpegExtractAudio::build_extract_options(codec_config, false, None);
        assert_eq!(opts.bitrate_kbps, None);
        assert_eq!(opts.quality_scale, None);
        assert_eq!(opts.encoder_name, Some("libmp3lame".to_string()));
    }
}
