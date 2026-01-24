//! FFmpeg merger post-processor.
//!
//! Merges separate video and audio streams into a single container.
//! This is typically needed when downloading from sites that serve
//! video and audio separately (like HLS/DASH streams).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};
use tracing::{debug, info};

use crate::ffmpeg::FFmpegRunner;

/// Post-processor that merges video and audio streams.
///
/// # Priority
/// This processor has priority 100 (runs first) because other
/// processors need a merged file to work with.
///
/// # When it runs
/// - When there are multiple input files (video + audio)
/// - When `requested_formats` contains multiple formats
pub struct FFmpegMerger {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FFmpegMerger {
    /// Create a new merger processor.
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Determine the output format based on config and input formats.
    fn determine_output_format(
        &self,
        config: &PostProcessConfig,
        video_ext: Option<&str>,
        audio_ext: Option<&str>,
    ) -> &'static str {
        // Use configured merge format if specified
        if let Some(ref format) = config.merge_output_format {
            match format.as_str() {
                "mp4" => return "mp4",
                "mkv" => return "mkv",
                "webm" => return "webm",
                "mov" => return "mov",
                _ => {}
            }
        }

        // Determine based on file extensions
        match (video_ext, audio_ext) {
            // WebM files (VP8/VP9 + Opus/Vorbis) - use MKV for better compatibility
            (Some("webm"), _) | (_, Some("webm")) => "mkv",

            // Default to MP4 for most content (h264/h265 + aac)
            _ => "mp4",
        }
    }

    /// Build FFmpeg arguments for merging.
    fn build_merge_args<'a>(
        &self,
        inputs: &[&'a Path],
        output: &'a Path,
        video_index: usize,
        audio_index: usize,
    ) -> Vec<String> {
        let mut args = Vec::new();

        // Add input files
        for input in inputs {
            args.push("-i".to_string());
            args.push(input.to_string_lossy().to_string());
        }

        // Map video from first input
        args.push("-map".to_string());
        args.push(format!("{video_index}:v:0"));

        // Map audio from second input (or first if only one input)
        args.push("-map".to_string());
        args.push(format!("{audio_index}:a:0"));

        // Copy streams without re-encoding
        args.push("-c".to_string());
        args.push("copy".to_string());

        // Handle AAC in MPEG-TS (HLS) - needs ADTS to ASC conversion
        args.push("-bsf:a".to_string());
        args.push("aac_adtstoasc".to_string());

        // Output file
        args.push(output.to_string_lossy().to_string());

        args
    }
}

#[async_trait]
impl PostProcessor for FFmpegMerger {
    fn name(&self) -> &str {
        "FFmpegMerger"
    }

    fn priority(&self) -> i32 {
        100 // High priority - runs first
    }

    fn should_run(&self, info: &InfoDict, _config: &PostProcessConfig) -> bool {
        // Run if there are multiple requested formats (video + audio separately)
        if let Some(ref formats) = info.requested_formats {
            if formats.len() > 1 {
                // Check if we have both video and audio streams
                let has_video = formats.iter().any(|f| f.has_video());
                let has_audio = formats.iter().any(|f| f.has_audio());
                return has_video && has_audio;
            }
        }
        false
    }

    async fn process(&self, info: &InfoDict, files: Vec<PathBuf>) -> Result<PostProcessResult> {
        if files.len() < 2 {
            // Nothing to merge
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        info!("Merging {} streams into single file", files.len());

        // Determine which file is video and which is audio
        let (video_file, audio_file, video_idx, audio_idx) = if files.len() == 2 {
            // Probe files to determine which is which
            let info1 = self.ffmpeg.probe(&files[0]).await?;
            let info2 = self.ffmpeg.probe(&files[1]).await?;

            if info1.has_video && !info1.has_audio && info2.has_audio {
                (&files[0], &files[1], 0, 1)
            } else if info2.has_video && !info2.has_audio && info1.has_audio {
                (&files[1], &files[0], 1, 0)
            } else if info1.has_video {
                // Both have video, prefer first as video
                (&files[0], &files[1], 0, 1)
            } else {
                (&files[1], &files[0], 1, 0)
            }
        } else {
            // More than 2 files - assume first is video, second is audio
            (&files[0], &files[1], 0, 1)
        };

        debug!(
            "Video file: {}, Audio file: {}",
            video_file.display(),
            audio_file.display()
        );

        // Determine output format
        let video_ext = video_file.extension().and_then(|e| e.to_str());
        let audio_ext = audio_file.extension().and_then(|e| e.to_str());
        let config = PostProcessConfig::default();
        let output_format = self.determine_output_format(&config, video_ext, audio_ext);

        // Create output filename
        let output_path = video_file.with_extension(output_format);

        // Build merge command
        let inputs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
        let args = self.build_merge_args(&inputs, &output_path, video_idx, audio_idx);

        // Run FFmpeg
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.ffmpeg.run(&args_refs).await?;

        info!("Merged output: {}", output_path.display());

        // Return result with merged file and original files as temp
        Ok(PostProcessResult {
            info: info.clone(),
            files: vec![output_path],
            temp_files: files, // Original files can be deleted
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_output_format() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let merger = FFmpegMerger::new(Arc::new(ffmpeg));

            // Default config has merge_output_format=mp4, so always returns mp4
            let config = PostProcessConfig::default();
            assert_eq!(merger.determine_output_format(&config, Some("mp4"), Some("m4a")), "mp4");
            assert_eq!(merger.determine_output_format(&config, Some("webm"), Some("opus")), "mp4");

            // Without explicit format, uses codec-based detection
            let config_no_format = PostProcessConfig {
                merge_output_format: None,
                ..Default::default()
            };
            assert_eq!(merger.determine_output_format(&config_no_format, Some("webm"), Some("opus")), "mkv");
        }
    }

    #[test]
    fn test_should_run() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let merger = FFmpegMerger::new(Arc::new(ffmpeg));
            let config = PostProcessConfig::default();

            // Single format - should not run
            let info = InfoDict::new(
                "test".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                "https://example.com".to_string(),
            );
            assert!(!merger.should_run(&info, &config));
        }
    }
}
