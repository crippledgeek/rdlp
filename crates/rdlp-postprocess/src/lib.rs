//! # rdlp-postprocess
//!
//! Post-processing pipeline for rdlp, providing FFmpeg-based media transformations.
//!
//! This crate provides post-processors for:
//! - **Stream merging**: Combine separate video and audio streams
//! - **Audio extraction**: Extract and convert audio to various formats
//! - **Video conversion**: Remux or transcode video files
//! - **Metadata embedding**: Embed title, artist, chapters, etc.
//! - **Thumbnail embedding**: Embed cover art into media files
//!
//! ## Architecture
//!
//! The post-processing system follows a pipeline architecture where multiple
//! processors can be chained together. Each processor:
//!
//! 1. Checks if it should run based on configuration
//! 2. Processes the input files
//! 3. Returns output files and any temporary files for cleanup
//!
//! Processors are executed in priority order (highest priority first).
//!
//! ## Quick Start
//!
//! ```no_run
//! use rdlp_postprocess::{PostProcessorRegistry, PostProcessorRegistryTrait};
//! use rdlp_core::{InfoDict, PostProcessConfig};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create registry (auto-detects FFmpeg)
//! let registry = PostProcessorRegistry::new()?;
//!
//! // Create info and config
//! let info = InfoDict::new(
//!     "video123".to_string(),
//!     "My Video".to_string(),
//!     "TestExtractor".to_string(),
//!     "https://example.com/video".to_string(),
//! );
//!
//! let mut config = PostProcessConfig::default();
//! config.embed_metadata = true;
//!
//! // Process files
//! let files = vec![PathBuf::from("video.mp4")];
//! let result = registry.process(&info, files, &config).await?;
//!
//! println!("Output: {:?}", result.files);
//! # Ok(())
//! # }
//! ```
//!
//! ## Custom Post-Processors
//!
//! You can create custom post-processors by implementing the `PostProcessor` trait:
//!
//! ```no_run
//! use async_trait::async_trait;
//! use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};
//! use std::path::PathBuf;
//!
//! struct MyProcessor;
//!
//! #[async_trait]
//! impl PostProcessor for MyProcessor {
//!     fn name(&self) -> &str {
//!         "MyProcessor"
//!     }
//!
//!     fn should_run(&self, info: &InfoDict, config: &PostProcessConfig) -> bool {
//!         // Custom condition
//!         true
//!     }
//!
//!     async fn process(&self, info: &InfoDict, files: Vec<PathBuf>, _config: &PostProcessConfig) -> Result<PostProcessResult> {
//!         // Custom processing logic
//!         Ok(PostProcessResult::new(info.clone(), files))
//!     }
//! }
//! ```
//!
//! ## FFmpeg Integration
//!
//! The crate provides a comprehensive FFmpeg wrapper:
//!
//! ```no_run
//! use rdlp_postprocess::ffmpeg::FFmpegRunner;
//!
//! # async fn example() -> rdlp_postprocess::error::Result<()> {
//! let ffmpeg = FFmpegRunner::new()?;
//!
//! // Probe a media file
//! let info = ffmpeg.probe("video.mp4").await?;
//! println!("Duration: {:?}s", info.duration);
//! println!("Video codec: {:?}", info.video_codec);
//! println!("Audio codec: {:?}", info.audio_codec);
//! println!("Resolution: {:?}", info.resolution_string());
//!
//! // Run custom FFmpeg command
//! ffmpeg.run(&["-i", "input.mp4", "-c:v", "copy", "-an", "video_only.mp4"]).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Built-in Processors
//!
//! | Processor | Priority | Description |
//! |-----------|----------|-------------|
//! | `FFmpegMerger` | 100 | Merge video + audio streams |
//! | `FFmpegExtractAudio` | 50 | Extract and convert audio |
//! | `FFmpegVideoConvertor` | 40 | Convert/remux video formats |
//! | `FFmpegMetadata` | 30 | Embed metadata and chapters |
//! | `EmbedThumbnail` | 20 | Embed cover art/thumbnails |
//!
//! ## Error Handling
//!
//! The crate uses a dedicated error type for post-processing errors:
//!
//! ```no_run
//! use rdlp_postprocess::error::{PostProcessError, Result};
//!
//! fn example() -> Result<()> {
//!     // Errors are descriptive and actionable
//!     Err(PostProcessError::FFmpegNotFound)
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod error;
pub mod ffmpeg;
pub mod processors;
pub mod registry;

// Re-export main types
pub use error::{PostProcessError, Result};
pub use ffmpeg::{FFmpegRunner, MediaInfo, StreamInfo};
pub use registry::{PostProcessorRegistry, PostProcessorRegistryTrait};

// Re-export processor types
pub use processors::{
    EmbedThumbnail, FFmpegExtractAudio, FFmpegMerger, FFmpegMetadata, FFmpegVideoConvertor,
};

// Re-export core types for convenience
pub use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor};
