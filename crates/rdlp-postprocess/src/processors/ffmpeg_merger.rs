//! FFmpeg merger post-processor.
//!
//! Merges separate video and audio streams into a single container
//! using `ffmpeg-the-third` library bindings (no CLI process spawning).
//! This is typically needed when downloading from sites that serve
//! video and audio separately (like HLS/DASH streams).

use std::path::PathBuf;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::ffmpeg::RemuxOptions;

ffmpeg_processor!(
    FFmpegMerger,
    "FFmpegMerger",
    100,
    "Post-processor that merges video and audio streams.\n\n\
     # Priority\n\
     This processor has priority 100 (runs first) because other\n\
     processors need a merged file to work with.\n\n\
     # When it runs\n\
     - When there are multiple input files (video + audio)\n\
     - When `requested_formats` contains multiple formats"
);

impl FFmpegMerger {
    /// Determine the output format based on config and input formats.
    fn determine_output_format(
        &self,
        config: &PostProcessConfig,
        video_ext: Option<&str>,
        audio_ext: Option<&str>,
    ) -> &'static str {
        // Use configured merge format if specified
        if let Some(format) = config.merge_output_format {
            return format.as_ext();
        }

        // Determine based on file extensions
        match (video_ext, audio_ext) {
            // WebM files (VP8/VP9 + Opus/Vorbis) - use MKV for better compatibility
            (Some("webm"), _) | (_, Some("webm")) => "mkv",

            // Default to MP4 for most content (h264/h265 + aac)
            _ => "mp4",
        }
    }
}

#[async_trait]
impl PostProcessor for FFmpegMerger {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
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

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
    ) -> Result<PostProcessResult> {
        if files.len() < 2 {
            // Nothing to merge
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        info!(streams = files.len(); "Merging streams into single file");

        // Determine which file is video and which is audio
        let (video_file, audio_file) = if files.len() == 2 {
            // Probe files to determine which is which
            let info1 = self.ffmpeg.probe(&files[0]).await?;
            let info2 = self.ffmpeg.probe(&files[1]).await?;

            if info1.has_video && !info1.has_audio && info2.has_audio {
                (&files[0], &files[1])
            } else if info2.has_video && !info2.has_audio && info1.has_audio {
                (&files[1], &files[0])
            } else if info1.has_video {
                // Both have video, prefer first as video
                (&files[0], &files[1])
            } else {
                (&files[1], &files[0])
            }
        } else {
            // More than 2 files - assume first is video, second is audio
            (&files[0], &files[1])
        };

        debug!(
            video:? = video_file.display(),
            audio:? = audio_file.display();
            "Identified input streams"
        );

        // Determine output format
        let video_ext = video_file.extension().and_then(|e| e.to_str());
        let audio_ext = audio_file.extension().and_then(|e| e.to_str());
        let output_format = self.determine_output_format(config, video_ext, audio_ext);

        // Create output filename
        let output_path = video_file.with_extension(output_format);

        // Merge using library bindings (stream copy, no re-encoding).
        // The MP4 muxer automatically handles AAC ADTS→ASC conversion,
        // so we no longer need the unconditional aac_adtstoasc BSF.
        let opts = RemuxOptions {
            faststart: matches!(output_format, "mp4" | "mov"),
            ..Default::default()
        };
        self.ffmpeg
            .merge(video_file, audio_file, &output_path, &opts)
            .await?;

        info!(output:? = output_path.display(); "Merged output");

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
    use std::sync::Arc;

    use super::*;
    use crate::ffmpeg::FFmpegRunner;

    #[test]
    fn test_determine_output_format() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let merger = FFmpegMerger::new(Arc::new(ffmpeg));

            // Default config has merge_output_format=mp4, so always returns mp4
            let config = PostProcessConfig::default();
            assert_eq!(
                merger.determine_output_format(&config, Some("mp4"), Some("m4a")),
                "mp4"
            );
            assert_eq!(
                merger.determine_output_format(&config, Some("webm"), Some("opus")),
                "mp4"
            );

            // Without explicit format, uses codec-based detection
            let config_no_format = PostProcessConfig {
                merge_output_format: None,
                ..PostProcessConfig::default()
            };
            assert_eq!(
                merger.determine_output_format(&config_no_format, Some("webm"), Some("opus")),
                "mkv"
            );
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
