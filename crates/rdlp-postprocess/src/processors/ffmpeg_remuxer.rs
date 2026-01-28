//! FFmpeg container remuxer post-processor.
//!
//! Remuxes video files to different containers (MP4, MKV) using stream copy
//! (no re-encoding). Improves seeking for formats like MPEG-TS which lack
//! a centralized index.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::error::PostProcessError;
use crate::ffmpeg::{FFmpegRunner, RemuxOptions};

/// Supported remux target containers.
const SUPPORTED_CONTAINERS: &[&str] = &["mp4", "mkv", "webm", "mov", "avi", "ts"];

/// Post-processor that remuxes video files to a different container.
///
/// This performs stream copy (no re-encoding) for fast container conversion.
/// Primary use case: remux MPEG-TS (.ts) to MP4 or MKV for better seeking.
///
/// # Priority
/// This processor has priority 45 (runs after merging, before video conversion).
///
/// # When it runs
/// When `remux_container` is specified in config.
///
/// # Supported Containers
/// - **MP4**: Best compatibility, faststart for streaming
/// - **MKV**: Supports all codecs, efficient cues index
/// - **WebM**: Web-optimized (VP8/VP9/AV1 + Opus/Vorbis)
/// - **MOV**: Apple QuickTime, good for editing
/// - **AVI**: Legacy format, wide support
/// - **TS**: MPEG-TS for broadcast/streaming
pub struct FFmpegRemuxer {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FFmpegRemuxer {
    /// Create a new remuxer processor.
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Check if the format is a supported container for remuxing.
    fn is_supported_container(format: &str) -> bool {
        SUPPORTED_CONTAINERS.contains(&format.to_lowercase().as_str())
    }
}

#[async_trait]
impl PostProcessor for FFmpegRemuxer {
    fn name(&self) -> &str {
        "FFmpegRemuxer"
    }

    fn priority(&self) -> i32 {
        45 // After merging (100), before video conversion (40)
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.remux_container.is_some()
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
        let target_container = config
            .remux_container
            .as_deref()
            .unwrap_or("mp4")
            .to_lowercase();

        // Validate target container
        if !Self::is_supported_container(&target_container) {
            return Err(PostProcessError::UnsupportedFormat {
                format: target_container,
                operation: "remux".to_string(),
            }
            .into());
        }

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Skip if already in target container
        if input_ext == target_container {
            debug!(
                container = target_container.as_str();
                "File already in target container, skipping remux"
            );
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        info!(
            file = input_file.display().to_string().as_str(),
            from = input_ext.as_str(),
            to = target_container.as_str();
            "Remuxing to improve seeking"
        );

        // Build output path
        let output_path = input_file.with_extension(&target_container);

        // Configure remux options
        // MP4/MOV: enable faststart for progressive playback (moov atom at beginning)
        // MKV: cues written at end, but seeking is still excellent
        // WebM: similar to MKV (Matroska-based)
        let opts = RemuxOptions {
            faststart: matches!(target_container.as_str(), "mp4" | "mov"),
            output_format: Some(target_container.clone()),
        };

        // Perform remux via library bindings
        self.ffmpeg.remux(input_file, &output_path, &opts).await?;

        info!(
            output = output_path.display().to_string().as_str();
            "Remuxed successfully"
        );

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
    fn test_supported_containers() {
        // All supported containers
        assert!(FFmpegRemuxer::is_supported_container("mp4"));
        assert!(FFmpegRemuxer::is_supported_container("MP4"));
        assert!(FFmpegRemuxer::is_supported_container("mkv"));
        assert!(FFmpegRemuxer::is_supported_container("MKV"));
        assert!(FFmpegRemuxer::is_supported_container("webm"));
        assert!(FFmpegRemuxer::is_supported_container("mov"));
        assert!(FFmpegRemuxer::is_supported_container("avi"));
        assert!(FFmpegRemuxer::is_supported_container("ts"));

        // Unsupported
        assert!(!FFmpegRemuxer::is_supported_container("flv"));
        assert!(!FFmpegRemuxer::is_supported_container("wmv"));
    }

    #[test]
    fn test_priority() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let remuxer = FFmpegRemuxer::new(Arc::new(ffmpeg));
            // Should run after merger (100) but before video convertor (40)
            assert_eq!(remuxer.priority(), 45);
        }
    }
}
