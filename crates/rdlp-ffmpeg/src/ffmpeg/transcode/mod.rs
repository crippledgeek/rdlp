//! Audio extraction, video conversion, and transcoding.
//!
//! Provides audio extraction (stream copy + transcode), video conversion
//! (remux + transcode), and internal filter graph / encode helpers.

mod audio_extract;
mod audio_pipeline;
mod audio_pipeline_direct;
pub(crate) mod mux_timing;
#[cfg(test)]
mod tests;
mod video_convert;
mod video_pipeline;

// Re-export public(crate) items used by normalize and other modules
pub(crate) use mux_timing::MuxTimingState;
