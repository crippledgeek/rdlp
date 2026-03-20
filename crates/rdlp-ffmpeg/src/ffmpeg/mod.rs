//! FFmpeg integration via `ffmpeg-the-third` library bindings.
//!
//! This module provides utilities for:
//! - Probing media files for codec and format information
//! - Remuxing and merging streams (stream copy)
//! - Audio extraction and transcoding
//! - Video conversion and transcoding
//! - Metadata and chapter embedding
//! - Thumbnail embedding (container-specific strategies)
//!
//! All FFmpeg operations use direct library calls (no CLI process spawning).
//!
//! # Example
//!
//! ```no_run
//! use rdlp_ffmpeg::FFmpegRunner;
//!
//! # async fn example() -> rdlp_ffmpeg::Result<()> {
//! let ffmpeg = FFmpegRunner::new()?;
//!
//! // Probe a media file
//! let info = ffmpeg.probe("video.mp4").await?;
//! println!("Duration: {:?}", info.duration);
//! println!("Video codec: {:?}", info.video_codec);
//! println!("Resolution: {:?}", info.resolution_string());
//! # Ok(())
//! # }
//! ```

mod audio_codecs;
mod ffi_helpers;
pub(crate) mod log_capture;
mod merge;
mod metadata;
mod normalize;
mod normalize_types;
mod options;
mod probe;
mod remux;
pub(crate) mod salvage;
mod thumbnail;
mod transcode;
pub mod video_codecs;

use std::path::Path;
use std::sync::OnceLock;

use log::debug;

use crate::error::{PostProcessError, Result};

// Re-export public types from submodules
pub use audio_codecs::{AUDIO_CODECS, AudioCodecConfig, get_audio_codec};
pub use normalize_types::{
    AudioNormMode, LoudnormMeasurements, LoudnormPreset, NormalizeOptions, PeakAnalysis,
};
pub use options::{AudioExtractOptions, ChapterEntry, RemuxOptions, VideoConvertOptions};
pub use probe::{MediaInfo, StreamInfo};
pub use video_codecs::{
    VideoCodecInfo, VideoEncoderInfo, available_encoders_for_codec, is_encoder_available,
    list_available_codecs, preferred_video_encoder, resolve_encoder,
};

/// Global initialization state for the FFmpeg library.
/// Ensures `ffmpeg_the_third::init()` is called exactly once.
static FFMPEG_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Initialize the FFmpeg library (idempotent).
///
/// This must be called before any `ffmpeg-the-third` library operations.
/// Safe to call multiple times -- only the first call performs initialization.
pub fn ensure_init() -> Result<()> {
    let result = FFMPEG_INIT.get_or_init(|| {
        ffmpeg_the_third::init().map_err(|e| format!("ffmpeg_the_third::init() failed: {e}"))?;
        // Suppress FFmpeg's internal diagnostic messages (e.g. mpegts stream timing warnings).
        // Only show actual errors -- we handle logging ourselves.
        ffmpeg_the_third::log::set_level(ffmpeg_the_third::log::Level::Error);
        Ok(())
    });

    match result {
        Ok(()) => Ok(()),
        Err(msg) => Err(PostProcessError::FFmpegInitFailed {
            message: msg.clone(),
        }),
    }
}

/// Set FFmpeg library log level based on verbose mode.
///
/// Call after `ensure_init()` to enable FFmpeg trace logging when `-v` is passed.
/// - `verbose=true`: Show FFmpeg trace/debug messages
/// - `verbose=false`: Only show FFmpeg errors (default)
pub fn set_verbose(verbose: bool) {
    let level = if verbose {
        ffmpeg_the_third::log::Level::Trace
    } else {
        ffmpeg_the_third::log::Level::Error
    };
    ffmpeg_the_third::log::set_level(level);
}

/// FFmpeg runner.
///
/// Provides media operations via `ffmpeg-the-third` library bindings:
/// probing, remuxing, merging, audio extraction, video conversion,
/// metadata embedding, thumbnail embedding, and audio normalization.
#[derive(Debug, Clone)]
pub struct FFmpegRunner;

impl FFmpegRunner {
    /// Create a new FFmpeg runner.
    ///
    /// Initializes the FFmpeg library (idempotent -- safe to call multiple times).
    pub fn new() -> Result<Self> {
        ensure_init()?;
        debug!("FFmpeg library initialized");
        Ok(Self)
    }

    /// Create a new FFmpeg runner with a custom location.
    ///
    /// The `location` parameter is accepted for API compatibility but is
    /// ignored -- all operations use `ffmpeg-the-third` library bindings
    /// which link against system FFmpeg shared libraries.
    pub fn with_location(_location: Option<&Path>) -> Result<Self> {
        Self::new()
    }

    /// Run a blocking FFmpeg operation on a background thread.
    ///
    /// All FFmpeg library calls are synchronous and must not run on the
    /// Tokio runtime. This helper wraps `tokio::task::spawn_blocking`
    /// with uniform error mapping.
    async fn spawn_blocking<F, T>(task_name: &str, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let name = task_name.to_string();
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("{name} task join error: {e}"),
            })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_audio_codec() {
        assert!(get_audio_codec("mp3").is_some());
        assert!(get_audio_codec("MP3").is_some()); // Case insensitive
        assert!(get_audio_codec("aac").is_some());
        assert!(get_audio_codec("unknown_codec").is_none());
    }

    #[test]
    fn test_audio_codec_config() {
        let mp3 = get_audio_codec("mp3").unwrap();
        assert_eq!(mp3.encoder, Some("libmp3lame"));
        assert_eq!(mp3.extension, "mp3");
        assert_eq!(mp3.quality_scale, Some((9, 0)));

        let flac = get_audio_codec("flac").unwrap();
        assert_eq!(flac.encoder, Some("flac")); // Native FLAC encoder
        assert!(flac.bitrate_range.is_none()); // Lossless
    }

    #[test]
    fn test_loudnorm_preset_from_str() {
        assert_eq!(
            "broadcast".parse::<LoudnormPreset>().unwrap(),
            LoudnormPreset::Broadcast
        );
        assert_eq!(
            "Streaming".parse::<LoudnormPreset>().unwrap(),
            LoudnormPreset::Streaming
        );
        assert_eq!(
            "LOUD".parse::<LoudnormPreset>().unwrap(),
            LoudnormPreset::Loud
        );
        assert!("unknown".parse::<LoudnormPreset>().is_err());
    }

    #[test]
    fn test_loudnorm_preset_display() {
        assert_eq!(LoudnormPreset::Broadcast.to_string(), "broadcast");
        assert_eq!(LoudnormPreset::Streaming.to_string(), "streaming");
        assert_eq!(LoudnormPreset::Loud.to_string(), "loud");
    }

    #[test]
    fn test_loudnorm_preset_targets() {
        let (i, tp, lra) = LoudnormPreset::Broadcast.targets();
        assert!((i - (-23.0)).abs() < f64::EPSILON);
        assert!((tp - (-2.0)).abs() < f64::EPSILON);
        assert!((lra - 7.0).abs() < f64::EPSILON);

        let (i, tp, lra) = LoudnormPreset::Streaming.targets();
        assert!((i - (-14.0)).abs() < f64::EPSILON);
        assert!((tp - (-1.0)).abs() < f64::EPSILON);
        assert!((lra - 11.0).abs() < f64::EPSILON);

        let (i, tp, lra) = LoudnormPreset::Loud.targets();
        assert!((i - (-11.0)).abs() < f64::EPSILON);
        assert!((tp - (-1.0)).abs() < f64::EPSILON);
        assert!((lra - 11.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_normalize_options_default_uses_streaming() {
        let opts = NormalizeOptions::default();
        assert!((opts.target_i - (-14.0)).abs() < f64::EPSILON);
        assert!((opts.target_tp - (-1.0)).abs() < f64::EPSILON);
        assert!((opts.target_lra - 11.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_normalize_options_default_boost_disabled() {
        let opts = NormalizeOptions::default();
        assert!(!opts.boost_enabled);
        assert!((opts.boost_gain_db - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_linear_shortfall_no_gap() {
        let m = LoudnormMeasurements {
            input_i: -20.0,
            input_tp: -7.0,
            input_lra: 8.0,
            input_thresh: -30.0,
            target_offset: 0.0,
        };
        assert!((m.linear_shortfall(-14.0, -1.0)).abs() < f64::EPSILON);
        assert!(m.linear_sufficient(-14.0, -1.0));
    }

    #[test]
    fn test_linear_shortfall_small_gap() {
        let m = LoudnormMeasurements {
            input_i: -24.0,
            input_tp: -3.0,
            input_lra: 8.0,
            input_thresh: -34.0,
            target_offset: 0.0,
        };
        assert!((m.linear_shortfall(-14.0, -1.0) - 8.0).abs() < f64::EPSILON);
        assert!(!m.linear_sufficient(-14.0, -1.0));
    }

    #[test]
    fn test_linear_shortfall_large_gap() {
        let m = LoudnormMeasurements {
            input_i: -30.0,
            input_tp: -1.0,
            input_lra: 12.0,
            input_thresh: -40.0,
            target_offset: 0.0,
        };
        assert!((m.linear_shortfall(-14.0, -1.0) - 16.0).abs() < f64::EPSILON);
        assert!(!m.linear_sufficient(-14.0, -1.0));
    }

    #[test]
    fn test_media_info_resolution() {
        let mut info = MediaInfo::default();
        assert!(info.resolution_string().is_none());

        info.width = Some(1920);
        info.height = Some(1080);
        assert_eq!(info.resolution_string(), Some("1920x1080".to_string()));
    }
}
