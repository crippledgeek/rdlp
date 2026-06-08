//! Audio extraction, video conversion, and transcoding.
//!
//! Provides audio extraction (stream copy + transcode), video conversion
//! (remux + transcode), and internal filter graph / encode helpers.

mod audio_extract;
mod audio_pipeline;
mod audio_pipeline_direct;
mod audio_recode_pipeline;
mod cancel;
pub mod encoder_options;
pub mod mux_timing;
#[cfg(test)]
mod tests;
pub mod thread_resolve;
mod video_convert;
mod video_pipeline;
mod video_transcode_context;
mod video_transcode_phases;

// Re-export public(crate) items used by normalize and other modules
pub use cancel::check_cancelled;
pub use mux_timing::MuxTimingState;
