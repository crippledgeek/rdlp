//! # rdlp-ffmpeg
//!
//! `FFmpeg` library bindings wrapper for rdlp, providing media operations
//! via `ffmpeg-the-third` (no CLI process spawning).
//!
//! # CLI Usage Policy
//!
//! This crate MUST NOT use `std::process::Command` or spawn external
//! processes. All `FFmpeg` operations use library bindings via
//! `ffmpeg-the-third`. Corrupt input recovery uses `discardcorrupt+genpts`
//! format flags on the input context (library API), not CLI fallback.
//! Violations are caught by CI check: `scripts/check-no-cli.sh`.
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
//! All operations use `tokio::task::spawn_blocking()` internally since `FFmpeg`
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
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::indexing_slicing)]

pub mod error;
pub mod ffmpeg;

// Re-export main types at crate root
pub use error::{CorruptionKind, PostProcessError, Result};
pub use ffmpeg::{
    AudioCodecInfo, AudioEncoderInfo, AudioExtractOptions, AudioNormMode, ChapterEntry,
    FFmpegRunner, FfmpegLogBridge, LogForwarderGuard, LoudnormMeasurements, LoudnormPreset,
    MediaInfo, NormalizeOptions, PeakAnalysis, RemuxOptions, StreamInfo, VideoCodecInfo,
    VideoConvertOptions, VideoEncoderInfo, bridge_ffmpeg_logs, set_verbose,
};
