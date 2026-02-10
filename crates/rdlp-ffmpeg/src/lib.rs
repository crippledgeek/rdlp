//! # rdlp-ffmpeg
//!
//! FFmpeg library bindings wrapper for rdlp, providing media operations
//! via `ffmpeg-the-third` (no CLI process spawning).
//!
//! This crate provides:
//! - **Media probing**: Extract stream info, codecs, duration, resolution
//! - **Remuxing**: Container conversion without re-encoding (stream copy)
//! - **Stream merging**: Combine separate video and audio streams
//! - **Audio extraction**: Extract and transcode audio to various formats
//! - **Video conversion**: Transcode video with codec selection
//! - **Metadata embedding**: Write title, artist, chapters into containers
//! - **Thumbnail embedding**: Cover art via `attached_pic` disposition
//!
//! All operations use `tokio::task::spawn_blocking()` internally since FFmpeg
//! library calls are synchronous.
//!
//! ## Quick Start
//!
//! ```no_run
//! use rdlp_ffmpeg::FFmpegRunner;
//!
//! # async fn example() -> rdlp_ffmpeg::Result<()> {
//! let ffmpeg = FFmpegRunner::new()?;
//!
//! // Probe a media file
//! let info = ffmpeg.probe("video.mp4").await?;
//! println!("Duration: {:?}s", info.duration);
//! println!("Video codec: {:?}", info.video_codec);
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod error;
pub mod ffmpeg;

// Re-export main types at crate root
pub use error::{CorruptionKind, PostProcessError, Result};
pub use ffmpeg::{
    AudioExtractOptions, AudioNormMode, ChapterEntry, FFmpegRunner, LoudnormMeasurements,
    LoudnormPreset, MediaInfo, NormalizeOptions, PeakAnalysis, RemuxOptions, StreamInfo,
    VideoConvertOptions, set_verbose,
};
