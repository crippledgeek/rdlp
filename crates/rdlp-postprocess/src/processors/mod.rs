//! Post-processor implementations.
//!
//! This module contains all built-in post-processors:
//!
//! - [`FFmpegMerger`]: Merge video and audio streams
//! - [`FFmpegExtractAudio`]: Extract and convert audio
//! - [`FFmpegVideoConvertor`]: Convert video formats
//! - [`FFmpegMetadata`]: Embed metadata into files
//! - [`EmbedThumbnail`]: Embed thumbnail images

mod embed_thumbnail;
mod ffmpeg_extract_audio;
mod ffmpeg_merger;
mod ffmpeg_metadata;
mod ffmpeg_video_convertor;

pub use embed_thumbnail::EmbedThumbnail;
pub use ffmpeg_extract_audio::FFmpegExtractAudio;
pub use ffmpeg_merger::FFmpegMerger;
pub use ffmpeg_metadata::FFmpegMetadata;
pub use ffmpeg_video_convertor::FFmpegVideoConvertor;
