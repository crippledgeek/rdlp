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

mod ffi_helpers;
mod merge;
mod metadata;
mod probe;
mod remux;
mod thumbnail;
mod transcode;

use std::path::Path;
use std::sync::OnceLock;

use log::debug;

use crate::error::{PostProcessError, Result};

// Re-export public types from submodules
pub use probe::{MediaInfo, StreamInfo};

/// Global initialization state for the FFmpeg library.
/// Ensures `ffmpeg_the_third::init()` is called exactly once.
static FFMPEG_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Initialize the FFmpeg library (idempotent).
///
/// This must be called before any `ffmpeg-the-third` library operations.
/// Safe to call multiple times — only the first call performs initialization.
pub fn ensure_init() -> Result<()> {
    let result = FFMPEG_INIT.get_or_init(|| {
        ffmpeg_the_third::init().map_err(|e| format!("ffmpeg_the_third::init() failed: {e}"))?;
        // Suppress FFmpeg's internal diagnostic messages (e.g. mpegts stream timing warnings).
        // Only show actual errors — we handle logging ourselves.
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

/// Audio codec configuration for extraction/conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioCodecConfig {
    /// FFmpeg encoder name (e.g., "libmp3lame", "aac")
    pub encoder: Option<&'static str>,
    /// Output file extension
    pub extension: &'static str,
    /// Quality scale range (worst, best) for -q:a
    pub quality_scale: Option<(u8, u8)>,
    /// Bitrate range in kbps (min, max) for -b:a
    pub bitrate_range: Option<(u32, u32)>,
}

/// Supported audio codecs and their configurations.
pub static AUDIO_CODECS: &[(&str, AudioCodecConfig)] = &[
    (
        "mp3",
        AudioCodecConfig {
            encoder: Some("libmp3lame"),
            extension: "mp3",
            quality_scale: Some((9, 0)), // VBR quality (0=best, 9=worst)
            bitrate_range: Some((32, 320)),
        },
    ),
    (
        "aac",
        AudioCodecConfig {
            encoder: Some("aac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: Some((32, 512)),
        },
    ),
    (
        "m4a",
        AudioCodecConfig {
            encoder: Some("aac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: Some((32, 512)),
        },
    ),
    (
        "opus",
        AudioCodecConfig {
            encoder: Some("libopus"),
            extension: "opus",
            quality_scale: None,
            bitrate_range: Some((6, 510)),
        },
    ),
    (
        "vorbis",
        AudioCodecConfig {
            encoder: Some("libvorbis"),
            extension: "ogg",
            quality_scale: Some((0, 10)), // Quality (0=worst, 10=best)
            bitrate_range: Some((32, 500)),
        },
    ),
    (
        "flac",
        AudioCodecConfig {
            encoder: None, // Native FLAC encoder
            extension: "flac",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "alac",
        AudioCodecConfig {
            encoder: Some("alac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "wav",
        AudioCodecConfig {
            encoder: None, // PCM
            extension: "wav",
            quality_scale: None,
            bitrate_range: None,
        },
    ),
    (
        "ac3",
        AudioCodecConfig {
            encoder: Some("ac3"),
            extension: "ac3",
            quality_scale: None,
            bitrate_range: Some((64, 640)),
        },
    ),
    (
        "eac3",
        AudioCodecConfig {
            encoder: Some("eac3"),
            extension: "eac3",
            quality_scale: None,
            bitrate_range: Some((32, 6144)),
        },
    ),
    (
        "dts",
        AudioCodecConfig {
            encoder: Some("dca"),
            extension: "dts",
            quality_scale: None,
            bitrate_range: Some((32, 3840)),
        },
    ),
    (
        "mp2",
        AudioCodecConfig {
            encoder: Some("mp2"),
            extension: "mp2",
            quality_scale: None,
            bitrate_range: Some((32, 384)),
        },
    ),
    (
        "wavpack",
        AudioCodecConfig {
            encoder: Some("wavpack"),
            extension: "wv",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "tta",
        AudioCodecConfig {
            encoder: Some("tta"),
            extension: "tta",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
];

/// Get audio codec configuration by name.
#[must_use]
pub fn get_audio_codec(name: &str) -> Option<&'static AudioCodecConfig> {
    AUDIO_CODECS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, config)| config)
}

/// Options for remux and merge operations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemuxOptions {
    /// Enable MP4 faststart (moov atom at beginning of file).
    pub faststart: bool,
    /// Force output format (e.g., "mp4", "mkv").
    pub output_format: Option<String>,
}

/// Options for audio extraction and transcoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioExtractOptions {
    /// Encoder name (e.g., "libmp3lame", "aac", "libopus").
    /// If None, uses the default encoder for the output format.
    pub encoder_name: Option<String>,
    /// If true, copy audio stream without re-encoding.
    pub copy: bool,
    /// Target bitrate in kbps (e.g., 192 for 192kbps).
    pub bitrate_kbps: Option<u32>,
    /// VBR quality scale value (codec-specific).
    /// For MP3: 0 (best) to 9 (worst).
    /// For Vorbis: 0 (worst) to 10 (best).
    pub quality_scale: Option<i32>,
}

/// Options for video conversion/transcoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoConvertOptions {
    /// If true, remux only (stream copy, no re-encoding).
    pub remux_only: bool,
    /// Video encoder name (e.g., "libx264", "libx265", "libvpx-vp9").
    pub video_codec: Option<String>,
    /// Encoder preset (e.g., "medium", "fast", "slow").
    pub preset: Option<String>,
    /// Constant Rate Factor for quality-based encoding.
    pub crf: Option<u32>,
    /// If true, copy audio stream without re-encoding.
    pub audio_copy: bool,
}

/// A chapter entry for metadata embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterEntry {
    /// Chapter ID (unique, typically sequential starting from 0).
    pub id: i64,
    /// Start time in milliseconds.
    pub start_ms: i64,
    /// End time in milliseconds.
    pub end_ms: i64,
    /// Chapter title.
    pub title: String,
}

/// FFmpeg runner.
///
/// Provides media operations via `ffmpeg-the-third` library bindings:
/// probing, remuxing, merging, audio extraction, video conversion,
/// metadata embedding, and thumbnail embedding.
#[derive(Debug, Clone)]
pub struct FFmpegRunner;

impl FFmpegRunner {
    /// Create a new FFmpeg runner.
    ///
    /// Initializes the FFmpeg library (idempotent — safe to call multiple times).
    pub fn new() -> Result<Self> {
        ensure_init()?;
        debug!("FFmpeg library initialized");
        Ok(Self)
    }

    /// Create a new FFmpeg runner with a custom location.
    ///
    /// The `location` parameter is accepted for API compatibility but is
    /// ignored — all operations use `ffmpeg-the-third` library bindings
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
        assert!(flac.encoder.is_none()); // Native codec
        assert!(flac.bitrate_range.is_none()); // Lossless
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
