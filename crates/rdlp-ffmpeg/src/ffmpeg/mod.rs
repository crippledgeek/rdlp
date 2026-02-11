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
pub(crate) mod log_capture;
mod merge;
mod metadata;
mod normalize;
mod probe;
mod remux;
pub(crate) mod salvage;
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

/// Audio normalization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioNormMode {
    /// Peak/gain normalization: analyze peak/RMS via astats, apply volume + alimiter.
    Peak,
    /// EBU R128 two-pass loudness normalization via loudnorm filter.
    Loudnorm,
}

/// Loudnorm target presets for common use cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoudnormPreset {
    /// Broadcast standard: I=-23 LUFS, TP=-2 dBTP, LRA=7 LU
    Broadcast,
    /// Streaming standard: I=-14 LUFS, TP=-1 dBTP, LRA=11 LU
    Streaming,
    /// Loud master: I=-11 LUFS, TP=-1 dBTP, LRA=11 LU
    Loud,
}

impl LoudnormPreset {
    /// Returns `(target_i, target_tp, target_lra)` for this preset.
    #[must_use]
    pub fn targets(self) -> (f64, f64, f64) {
        match self {
            Self::Broadcast => (-23.0, -2.0, 7.0),
            Self::Streaming => (-14.0, -1.0, 11.0),
            Self::Loud => (-11.0, -1.0, 11.0),
        }
    }
}

impl std::str::FromStr for LoudnormPreset {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "broadcast" => Ok(Self::Broadcast),
            "streaming" => Ok(Self::Streaming),
            "loud" => Ok(Self::Loud),
            _ => Err(format!(
                "unknown loudnorm preset '{s}': expected broadcast, streaming, or loud"
            )),
        }
    }
}

impl std::fmt::Display for LoudnormPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broadcast => write!(f, "broadcast"),
            Self::Streaming => write!(f, "streaming"),
            Self::Loud => write!(f, "loud"),
        }
    }
}

/// Options for audio normalization.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizeOptions {
    /// Normalization mode (peak or loudnorm).
    pub mode: AudioNormMode,
    /// Target peak level in dBFS (Mode A, default -1.0).
    pub target_peak_db: f64,
    /// Target integrated loudness in LUFS (Mode B, default -16.0).
    pub target_i: f64,
    /// Target true peak in dBTP (Mode B, default -1.5).
    pub target_tp: f64,
    /// Target loudness range in LU (Mode B, default 11.0).
    pub target_lra: f64,
    /// Automatically salvage corrupt Matroska/WebM containers before processing.
    ///
    /// When enabled (default), corrupt inputs are detected via EBML log analysis
    /// and automatically remuxed to a clean temporary file before normalization.
    /// Disable for strict mode where corruption should be a hard error.
    pub salvage: bool,
    /// Force dynamic (per-frame compression) mode in loudnorm pass 2.
    ///
    /// By default, loudnorm uses `linear=true` (letting FFmpeg fall back to
    /// dynamic internally if needed). This flag forces `linear=false` for users
    /// who explicitly want dynamic compression. Corresponds to `--loudnorm-dynamic`.
    pub force_dynamic: bool,
    /// Prepend a mild acompressor before loudnorm in pass 2.
    ///
    /// Tames extreme peaks before loudnorm, allowing linear mode to apply more
    /// gain without hitting the TP ceiling. Uses a conservative preset:
    /// `threshold=-18dB, ratio=3:1, attack=20ms, release=200ms, makeup=2dB, knee=6dB`.
    /// Corresponds to `--loudnorm-precompress`.
    pub precompress: bool,
    /// Enable limiter-boost fallback for over-compressed content.
    ///
    /// When enabled and loudnorm pass 1 shows shortfall > 6 LU, skips
    /// loudnorm pass 2 and applies a fixed gain with hard limiter instead.
    /// Corresponds to `--normalize-boost`.
    pub boost_enabled: bool,
    /// Gain in dB for limiter-boost fallback (default 12.0).
    ///
    /// Only used when `boost_enabled` is true and shortfall exceeds threshold.
    /// Corresponds to `--normalize-boost-db`.
    pub boost_gain_db: f64,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        let (i, tp, lra) = LoudnormPreset::Streaming.targets();
        Self {
            mode: AudioNormMode::Peak,
            target_peak_db: -1.0,
            target_i: i,
            target_tp: tp,
            target_lra: lra,
            salvage: true,
            force_dynamic: false,
            precompress: false,
            boost_enabled: false,
            boost_gain_db: 12.0,
        }
    }
}

/// Results from peak/RMS audio analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct PeakAnalysis {
    /// Peak level in dBFS.
    pub peak_db: f64,
    /// RMS level in dBFS.
    pub rms_db: f64,
    /// Computed gain adjustment in dB.
    pub gain_db: f64,
}

/// Measurements from EBU R128 loudnorm first pass.
#[derive(Debug, Clone, PartialEq)]
pub struct LoudnormMeasurements {
    /// Measured integrated loudness (LUFS).
    pub input_i: f64,
    /// Measured true peak (dBTP).
    pub input_tp: f64,
    /// Measured loudness range (LU).
    pub input_lra: f64,
    /// Measured loudness threshold (LUFS).
    pub input_thresh: f64,
    /// Target offset (LU).
    pub target_offset: f64,
}

impl LoudnormMeasurements {
    /// Predict the gain (dB) that linear mode would apply.
    ///
    /// Linear mode applies a constant gain capped by the true-peak headroom:
    /// `min(target_i - measured_i, target_tp - measured_tp)`.
    #[must_use]
    pub fn predict_linear_gain(&self, target_i: f64, target_tp: f64) -> f64 {
        let desired = target_i - self.input_i;
        let tp_headroom = target_tp - self.input_tp;
        desired.min(tp_headroom)
    }

    /// Compute the shortfall (LU) when using linear mode.
    ///
    /// Returns `target_i - (measured_i + predicted_linear_gain)`.
    /// A value <= 0 means linear mode fully reaches the target.
    #[must_use]
    pub fn linear_shortfall(&self, target_i: f64, target_tp: f64) -> f64 {
        let gain = self.predict_linear_gain(target_i, target_tp);
        target_i - (self.input_i + gain)
    }

    /// Returns `true` if linear mode can reach the target within 0.5 LU.
    #[must_use]
    pub fn linear_sufficient(&self, target_i: f64, target_tp: f64) -> bool {
        self.linear_shortfall(target_i, target_tp) <= 0.5
    }
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
        // Source: I=-24, TP=-5 → target: I=-14, TP=-1
        // desired=10, tp_headroom=4 → gain=4 → shortfall=-14-(-24+4)=6
        // Wait, let me recalculate: shortfall = -14 - (-24 + 4) = -14 - (-20) = 6
        // Actually: desired = -14 - (-24) = 10, tp_headroom = -1 - (-5) = 4
        // gain = min(10, 4) = 4
        // shortfall = -14 - (-24 + 4) = -14 + 20 = 6
        let m = LoudnormMeasurements {
            input_i: -20.0,
            input_tp: -7.0,
            input_lra: 8.0,
            input_thresh: -30.0,
            target_offset: 0.0,
        };
        // desired = -14 - (-20) = 6, tp_headroom = -1 - (-7) = 6
        // gain = min(6, 6) = 6, shortfall = -14 - (-20 + 6) = -14 + 14 = 0
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
        // desired = -14 - (-24) = 10, tp_headroom = -1 - (-3) = 2
        // gain = min(10, 2) = 2, shortfall = -14 - (-24 + 2) = -14 + 22 = 8
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
        // desired = -14 - (-30) = 16, tp_headroom = -1 - (-1) = 0
        // gain = min(16, 0) = 0, shortfall = -14 - (-30 + 0) = -14 + 30 = 16
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
