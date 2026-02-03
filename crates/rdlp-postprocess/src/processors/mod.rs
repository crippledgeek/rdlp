//! Post-processor implementations.
//!
//! This module contains all built-in post-processors:
//!
//! - [`FFmpegMerger`]: Merge video and audio streams
//! - [`FFmpegExtractAudio`]: Extract and convert audio
//! - [`FFmpegRemuxer`]: Remux to different container (MP4/MKV) for better seeking
//! - [`FFmpegVideoConvertor`]: Convert video formats
//! - [`FFmpegMetadata`]: Embed metadata into files
//! - [`EmbedThumbnail`]: Embed thumbnail images

/// Declare an FFmpeg-backed post-processor struct with standard boilerplate.
///
/// Generates:
/// - `pub struct $name { ffmpeg: Arc<FFmpegRunner> }`
/// - `impl $name { pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self }`
/// - `fn name()` and `fn priority()` inside the `PostProcessor` trait impl
///
/// The caller must still provide the `should_run` and `process` methods
/// (and an `#[async_trait]` attribute) — see usage in each processor file.
///
/// # Example
/// ```ignore
/// ffmpeg_processor!(FFmpegRemuxer, "FFmpegRemuxer", 45, "Remuxes containers.");
/// ```
macro_rules! ffmpeg_processor {
    ($name:ident, $display_name:expr, $priority:expr, $doc:expr) => {
        #[doc = $doc]
        pub struct $name {
            ffmpeg: std::sync::Arc<rdlp_ffmpeg::FFmpegRunner>,
        }

        impl $name {
            /// Create a new processor.
            #[must_use]
            pub fn new(ffmpeg: std::sync::Arc<rdlp_ffmpeg::FFmpegRunner>) -> Self {
                Self { ffmpeg }
            }
        }

        // Provide `name()` and `priority()` via a helper trait impl.
        // The file that uses this macro must still write the full
        // `#[async_trait] impl PostProcessor for $name { … }` block
        // containing `should_run` and `process`, and can call
        // `self.processor_name()` / `self.processor_priority()` to
        // delegate.  However, since PostProcessor::name/priority are
        // simple, each file just inlines the constants via the two
        // helper methods below — no extra trait needed.
        impl $name {
            /// Processor display name (used by `PostProcessor::name`).
            #[inline]
            fn processor_name(&self) -> &str {
                $display_name
            }

            /// Processor priority (used by `PostProcessor::priority`).
            #[inline]
            fn processor_priority(&self) -> i32 {
                $priority
            }
        }
    };
}

mod embed_thumbnail;
mod ffmpeg_extract_audio;
mod ffmpeg_merger;
mod ffmpeg_metadata;
mod ffmpeg_remuxer;
mod ffmpeg_video_convertor;

pub use embed_thumbnail::EmbedThumbnail;
pub use ffmpeg_extract_audio::FFmpegExtractAudio;
pub use ffmpeg_merger::FFmpegMerger;
pub use ffmpeg_metadata::FFmpegMetadata;
pub use ffmpeg_remuxer::FFmpegRemuxer;
pub use ffmpeg_video_convertor::FFmpegVideoConvertor;
