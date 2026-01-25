//! FFmpeg audio extraction post-processor.
//!
//! Extracts and optionally converts audio from video files.
//! Supports various output formats including MP3, AAC, Opus, FLAC, etc.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::error::PostProcessError;
use crate::ffmpeg::{FFmpegRunner, get_audio_codec};

/// Post-processor that extracts audio from video files.
///
/// # Priority
/// This processor has priority 50 (runs after merging).
///
/// # When it runs
/// - When `extract_audio` is true in config
/// - Optionally converts to specified `audio_format`
pub struct FFmpegExtractAudio {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FFmpegExtractAudio {
    /// Create a new audio extraction processor.
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build quality arguments for the specified codec.
    fn build_quality_args(&self, codec: &str, quality: Option<&str>) -> Vec<String> {
        let mut args = Vec::new();

        let codec_config = match get_audio_codec(codec) {
            Some(c) => c,
            None => return args,
        };

        let quality = match quality {
            Some(q) => q,
            None => return args,
        };

        // Parse quality as number
        if let Ok(q_num) = quality.parse::<u32>() {
            // If it's a bitrate (e.g., "192" for 192kbps)
            if let Some((min, max)) = codec_config.bitrate_range {
                let bitrate = q_num.clamp(min, max);
                args.push("-b:a".to_string());
                args.push(format!("{bitrate}k"));
            }
        } else if let Some((worst, best)) = codec_config.quality_scale {
            // Quality scale (VBR)
            // Map quality string to scale
            let scale = match quality.to_lowercase().as_str() {
                "best" | "0" => best,
                "worst" | "9" | "10" => worst,
                _ => {
                    // Try to parse as scale value
                    quality.parse().unwrap_or((worst + best) / 2)
                }
            };
            args.push("-q:a".to_string());
            args.push(scale.to_string());
        }

        args
    }
}

#[async_trait]
impl PostProcessor for FFmpegExtractAudio {
    fn name(&self) -> &str {
        "FFmpegExtractAudio"
    }

    fn priority(&self) -> i32 {
        50 // After merging
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.extract_audio
    }

    async fn process(&self, info: &InfoDict, files: Vec<PathBuf>) -> Result<PostProcessResult> {
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
        let config = PostProcessConfig::default();
        let target_format = config.audio_format.as_deref().unwrap_or("mp3");
        let codec_config =
            get_audio_codec(target_format).ok_or_else(|| PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "audio extraction".to_string(),
            })?;

        info!(
            "Extracting audio from {} to {} format",
            input_file.display(),
            target_format
        );

        // Determine if we can copy or need to transcode
        let can_copy = media_info
            .audio_codec
            .as_ref()
            .is_some_and(|c| c == target_format || (c == "aac" && target_format == "m4a"));

        // Build output path
        let output_path = input_file.with_extension(codec_config.extension);

        // Build FFmpeg arguments
        let mut args = vec![
            "-i".to_string(),
            input_file.to_string_lossy().to_string(),
            "-vn".to_string(), // No video
        ];

        if can_copy {
            debug!("Audio codec matches target, copying stream");
            args.push("-c:a".to_string());
            args.push("copy".to_string());
        } else {
            // Transcode
            if let Some(encoder) = codec_config.encoder {
                args.push("-c:a".to_string());
                args.push(encoder.to_string());
            }

            // Add quality arguments
            let quality_args =
                self.build_quality_args(target_format, config.audio_quality.as_deref());
            args.extend(quality_args);
        }

        args.push(output_path.to_string_lossy().to_string());

        // Run FFmpeg
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.ffmpeg.run(&args_refs).await?;

        info!("Audio extracted: {}", output_path.display());

        // Determine if we should keep original
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

    #[test]
    fn test_build_quality_args_mp3() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let extractor = FFmpegExtractAudio::new(Arc::new(ffmpeg));

            // Bitrate
            let args = extractor.build_quality_args("mp3", Some("192"));
            assert!(args.contains(&"-b:a".to_string()));
            assert!(args.contains(&"192k".to_string()));

            // VBR quality
            let args = extractor.build_quality_args("mp3", Some("best"));
            assert!(args.contains(&"-q:a".to_string()));
            assert!(args.contains(&"0".to_string())); // Best quality for MP3
        }
    }

    #[test]
    fn test_build_quality_args_opus() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let extractor = FFmpegExtractAudio::new(Arc::new(ffmpeg));

            // Opus uses bitrate only
            let args = extractor.build_quality_args("opus", Some("128"));
            assert!(args.contains(&"-b:a".to_string()));
            assert!(args.contains(&"128k".to_string()));
        }
    }
}
