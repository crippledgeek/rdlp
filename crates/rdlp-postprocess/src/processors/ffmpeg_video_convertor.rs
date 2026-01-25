//! FFmpeg video conversion/remuxing post-processor.
//!
//! Converts video files to different formats or remuxes them to different
//! containers without re-encoding.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::error::PostProcessError;
use crate::ffmpeg::FFmpegRunner;

/// Supported video container formats.
const SUPPORTED_CONTAINERS: &[&str] = &["mp4", "mkv", "webm", "mov", "avi", "flv", "ts"];

/// Supported video codecs for transcoding.
const VIDEO_CODECS: &[(&str, &str)] = &[
    ("h264", "libx264"),
    ("h265", "libx265"),
    ("hevc", "libx265"),
    ("vp9", "libvpx-vp9"),
    ("vp8", "libvpx"),
    ("av1", "libaom-av1"),
];

/// Post-processor that converts or remuxes video files.
///
/// # Priority
/// This processor has priority 40 (runs after merging, before metadata).
///
/// # When it runs
/// - When `recode_video` is specified in config (full transcode)
/// - When container change is needed
pub struct FFmpegVideoConvertor {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FFmpegVideoConvertor {
    /// Create a new video converter processor.
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Check if the format is a supported container.
    fn is_supported_container(format: &str) -> bool {
        SUPPORTED_CONTAINERS.contains(&format.to_lowercase().as_str())
    }

    /// Get the FFmpeg encoder for a codec.
    fn get_encoder(codec: &str) -> Option<&'static str> {
        VIDEO_CODECS
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(codec))
            .map(|(_, e)| *e)
    }

    /// Determine if we can remux (copy) or need to transcode.
    fn can_remux(input_ext: &str, output_ext: &str, video_codec: Option<&str>) -> bool {
        // Check if the codec is compatible with the target container
        match (output_ext.to_lowercase().as_str(), video_codec) {
            // MP4 supports H.264, H.265, MPEG-4
            ("mp4", Some(c)) => matches!(
                c.to_lowercase().as_str(),
                "h264" | "avc" | "h265" | "hevc" | "mpeg4"
            ),
            // MKV supports almost everything
            ("mkv", _) => true,
            // WebM supports VP8, VP9, AV1
            ("webm", Some(c)) => matches!(c.to_lowercase().as_str(), "vp8" | "vp9" | "av1"),
            // Same container - can copy
            _ if input_ext.eq_ignore_ascii_case(output_ext) => true,
            _ => false,
        }
    }
}

#[async_trait]
impl PostProcessor for FFmpegVideoConvertor {
    fn name(&self) -> &str {
        "FFmpegVideoConvertor"
    }

    fn priority(&self) -> i32 {
        40 // After merging
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        // Run if recode_video is specified
        config.recode_video.is_some()
    }

    async fn process(&self, info: &InfoDict, files: Vec<PathBuf>) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let input_file = &files[0];
        let config = PostProcessConfig::default();

        let target_format = config.recode_video.as_deref().unwrap_or("mp4");

        // Validate target format
        if !Self::is_supported_container(target_format) {
            return Err(PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "video conversion".to_string(),
            }
            .into());
        }

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Skip if already in target format
        if input_ext.eq_ignore_ascii_case(target_format) {
            debug!(format:? = target_format; "File already in target format, skipping conversion");
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        info!(
            file:? = input_file.display(),
            from:? = input_ext,
            to:? = target_format;
            "Converting format"
        );

        // Probe input to determine codecs
        let media_info = self.ffmpeg.probe(input_file).await?;

        // Determine if we can remux or need to transcode
        let can_remux =
            Self::can_remux(input_ext, target_format, media_info.video_codec.as_deref());

        // Build output path
        let output_path = input_file.with_extension(target_format);

        // Build FFmpeg arguments
        let mut args = vec!["-i".to_string(), input_file.to_string_lossy().to_string()];

        if can_remux {
            debug!("Remuxing (stream copy)");
            args.push("-c".to_string());
            args.push("copy".to_string());
        } else {
            debug!("Transcoding video");

            // Get appropriate encoder
            let target_codec = match target_format {
                "webm" => "vp9",
                "mp4" | "mov" | "mkv" => "h264",
                _ => "h264",
            };

            if let Some(encoder) = Self::get_encoder(target_codec) {
                args.push("-c:v".to_string());
                args.push(encoder.to_string());

                // Add reasonable default encoding settings
                match target_codec {
                    "h264" | "h265" | "hevc" => {
                        args.push("-preset".to_string());
                        args.push("medium".to_string());
                        args.push("-crf".to_string());
                        args.push("23".to_string());
                    }
                    "vp9" => {
                        args.push("-crf".to_string());
                        args.push("30".to_string());
                        args.push("-b:v".to_string());
                        args.push("0".to_string());
                    }
                    _ => {}
                }
            }

            // Copy audio stream
            args.push("-c:a".to_string());
            args.push("copy".to_string());
        }

        // Add any custom FFmpeg args
        args.extend(config.ffmpeg_args.iter().cloned());

        args.push(output_path.to_string_lossy().to_string());

        // Run FFmpeg
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.ffmpeg.run(&args_refs).await?;

        info!(output:? = output_path.display(); "Converted");

        // Keep or delete original
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
    fn test_is_supported_container() {
        assert!(FFmpegVideoConvertor::is_supported_container("mp4"));
        assert!(FFmpegVideoConvertor::is_supported_container("MKV"));
        assert!(FFmpegVideoConvertor::is_supported_container("webm"));
        assert!(!FFmpegVideoConvertor::is_supported_container("xyz"));
    }

    #[test]
    fn test_get_encoder() {
        assert_eq!(FFmpegVideoConvertor::get_encoder("h264"), Some("libx264"));
        assert_eq!(FFmpegVideoConvertor::get_encoder("vp9"), Some("libvpx-vp9"));
        assert_eq!(FFmpegVideoConvertor::get_encoder("unknown"), None);
    }

    #[test]
    fn test_can_remux() {
        // H.264 to MP4 - can remux
        assert!(FFmpegVideoConvertor::can_remux("mkv", "mp4", Some("h264")));

        // VP9 to MP4 - cannot remux (needs transcode)
        assert!(!FFmpegVideoConvertor::can_remux("webm", "mp4", Some("vp9")));

        // VP9 to WebM - can remux
        assert!(FFmpegVideoConvertor::can_remux("mkv", "webm", Some("vp9")));

        // Anything to MKV - can remux
        assert!(FFmpegVideoConvertor::can_remux("mp4", "mkv", Some("h264")));
        assert!(FFmpegVideoConvertor::can_remux("webm", "mkv", Some("vp9")));
    }
}
