//! # rdlp-postprocess
//!
//! Channel-based post-processing pipeline for rdlp.
//!
//! This crate provides a 9-stage pipeline for FFmpeg-based media transformations:
//! - **`MergeStage`**: Combine separate video and audio streams
//! - **`AudioExtractStage`**: Extract and convert audio to various formats
//! - **`NormalizeStage`**: Peak / EBU R128 loudnorm audio normalization
//! - **`RemuxStage`**: Remux to a target container (HLS TS → MP4, etc.)
//! - **`RecodeStage`**: Transcode video to a different container/codec
//! - **`SubtitleStage`**: Embed subtitle files into video containers
//! - **`MetadataStage`**: Embed title, artist, chapters, etc.
//! - **`ThumbnailStage`**: Embed cover art (`FFmpeg` + MP4 covr atom)
//! - **`FixupStage`**: Detect and repair container/codec issues
//!
//! ## Quick Start
//!
//! ```no_run
//! use rdlp_postprocess::{Pipeline, PipelineRunOptions, TempRegistry};
//! use rdlp_postprocess::pipeline::PipelineStage;
//! use rdlp_postprocess::{MergeStage, RemuxStage, MetadataStage, ThumbnailStage};
//! use rdlp_postprocess::FFmpegRunner;
//! use rdlp_postprocess::PostProcess;
//! use rdlp_types::InfoDict;
//! use std::path::PathBuf;
//! use std::sync::Arc;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let ffmpeg = Arc::new(FFmpegRunner::new()?);
//! let temp_registry = Arc::new(TempRegistry::new());
//!
//! let stages: Vec<Arc<dyn PipelineStage>> = vec![
//!     Arc::new(MergeStage::new(Arc::clone(&ffmpeg))),
//!     Arc::new(RemuxStage::new(Arc::clone(&ffmpeg))),
//!     Arc::new(MetadataStage::new(Arc::clone(&ffmpeg))),
//!     Arc::new(ThumbnailStage::new(ffmpeg)),
//! ];
//!
//! let pipeline = Arc::new(Pipeline::new(stages, temp_registry, 2));
//!
//! let info = InfoDict::new(
//!     "video123".to_string(),
//!     "My Video".to_string(),
//!     "TestExtractor".to_string(),
//!     "https://example.com/video".to_string(),
//! );
//!
//! let config = Arc::new(PostProcess::default());
//! let files = vec![PathBuf::from("video.mp4")];
//! let opts = PipelineRunOptions { keep_inputs: false, is_hls: false, verbose: false };
//!
//! let output = pipeline.run(info, files, opts, config, "video".to_string(), None, None).await?;
//! println!("Output: {output:?}");
//! # Ok(())
//! # }
//! ```
//!
//! ## `FFmpeg` Integration
//!
//! `FFmpeg` operations are provided by the [`rdlp_ffmpeg`] crate. Use `rdlp_postprocess::FFmpegRunner`
//! (re-exported) or `rdlp_ffmpeg::FFmpegRunner` directly:
//!
//! ```no_run
//! use rdlp_postprocess::FFmpegRunner;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let ffmpeg = FFmpegRunner::new()?;
//!
//! // Probe a media file
//! let info = ffmpeg.probe("video.mp4").await?;
//! println!("Duration: {:?}s", info.duration);
//! println!("Video codec: {:?}", info.video_codec);
//! println!("Resolution: {:?}", info.resolution_string());
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::indexing_slicing)]

pub mod pipeline;

// Re-export pipeline types for use by rdlp-api
pub use pipeline::stages::{
    AudioExtractStage, FinalizeMetadataStage, FixupStage, MergeStage, MetadataStage,
    NormalizeStage, RecodeStage, RemuxStage, SubtitleStage, ThumbnailStage,
};
pub use pipeline::{BatchInput, Pipeline, PipelineError, PipelineRunOptions, TempRegistry};

// Re-export FFmpegRunner so callers don't need a direct rdlp-ffmpeg dependency
pub use rdlp_ffmpeg::FFmpegRunner;

// Re-export core types for convenience
pub use rdlp_types::PostProcess;
