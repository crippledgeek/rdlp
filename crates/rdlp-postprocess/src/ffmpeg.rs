//! FFmpeg and FFprobe integration.
//!
//! This module provides utilities for:
//! - Detecting FFmpeg/FFprobe executables
//! - Running FFmpeg commands with proper argument handling
//! - Probing media files for codec and format information
//!
//! # Example
//!
//! ```no_run
//! use rdlp_postprocess::ffmpeg::FFmpegRunner;
//!
//! # async fn example() -> rdlp_postprocess::error::Result<()> {
//! let ffmpeg = FFmpegRunner::new()?;
//!
//! // Probe a media file
//! let info = ffmpeg.probe("video.mp4").await?;
//! println!("Duration: {:?}", info.duration);
//! println!("Video codec: {:?}", info.video_codec);
//!
//! // Run FFmpeg command
//! ffmpeg.run(&["-i", "input.mp4", "-c:v", "copy", "output.mkv"]).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::OnceLock;

use log::{debug, trace};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::{PostProcessError, Result};

/// Global initialization state for the FFmpeg library.
/// Ensures `ffmpeg_the_third::init()` is called exactly once.
static FFMPEG_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Initialize the FFmpeg library (idempotent).
///
/// This must be called before any `ffmpeg-the-third` library operations.
/// Safe to call multiple times — only the first call performs initialization.
pub fn ensure_init() -> Result<()> {
    let result = FFMPEG_INIT.get_or_init(|| {
        ffmpeg_the_third::init().map_err(|e| format!("ffmpeg_the_third::init() failed: {e}"))
    });

    match result {
        Ok(()) => Ok(()),
        Err(msg) => Err(PostProcessError::FFmpegInitFailed {
            message: msg.clone(),
        }),
    }
}

/// Audio codec configuration for extraction/conversion.
#[derive(Debug, Clone)]
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
];

/// Get audio codec configuration by name.
pub fn get_audio_codec(name: &str) -> Option<&'static AudioCodecConfig> {
    AUDIO_CODECS
        .iter()
        .find(|(n, _)| *n == name.to_lowercase())
        .map(|(_, config)| config)
}

/// Options for remux and merge operations.
#[derive(Debug, Clone, Default)]
pub struct RemuxOptions {
    /// Enable MP4 faststart (moov atom at beginning of file).
    pub faststart: bool,
    /// Force output format (e.g., "mp4", "mkv").
    pub output_format: Option<String>,
}

/// Options for audio extraction and transcoding.
#[derive(Debug, Clone, Default)]
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
#[derive(Debug, Clone, Default)]
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

/// Media file information from FFprobe.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    /// File path
    pub path: PathBuf,
    /// Duration in seconds
    pub duration: Option<f64>,
    /// Container format (e.g., "mp4", "mkv")
    pub format: Option<String>,
    /// Video codec (e.g., "h264", "vp9")
    pub video_codec: Option<String>,
    /// Audio codec (e.g., "aac", "mp3")
    pub audio_codec: Option<String>,
    /// Video width in pixels
    pub width: Option<u32>,
    /// Video height in pixels
    pub height: Option<u32>,
    /// Video frame rate
    pub fps: Option<f64>,
    /// Video bitrate in kbps
    pub video_bitrate: Option<u32>,
    /// Audio bitrate in kbps
    pub audio_bitrate: Option<u32>,
    /// Audio sample rate in Hz
    pub sample_rate: Option<u32>,
    /// Number of audio channels
    pub channels: Option<u8>,
    /// Total file bitrate in kbps
    pub bitrate: Option<u32>,
    /// File size in bytes
    pub filesize: Option<u64>,
    /// Number of streams
    pub stream_count: usize,
    /// Whether file has video stream
    pub has_video: bool,
    /// Whether file has audio stream
    pub has_audio: bool,
    /// Raw stream information
    pub streams: Vec<StreamInfo>,
    /// Metadata tags
    pub metadata: HashMap<String, String>,
}

/// Information about a single stream.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamInfo {
    /// Stream index
    pub index: usize,
    /// Codec type ("video", "audio", "subtitle", "data")
    pub codec_type: String,
    /// Codec name
    pub codec_name: Option<String>,
    /// Stream-specific metadata
    pub metadata: HashMap<String, String>,
}

/// FFmpeg runner.
///
/// Provides media probing via `ffmpeg-the-third` library bindings and
/// CLI execution for operations not yet migrated to library calls.
#[derive(Debug, Clone)]
pub struct FFmpegRunner {
    /// Path to FFmpeg executable (used for CLI operations not yet migrated)
    ffmpeg_path: PathBuf,
    /// FFmpeg version string
    version: Option<String>,
}

impl FFmpegRunner {
    /// Create a new FFmpeg runner, auto-detecting executables from PATH.
    pub fn new() -> Result<Self> {
        Self::with_location(None)
    }

    /// Create a new FFmpeg runner with a custom location.
    ///
    /// If `location` is `Some`, it should be either:
    /// - A path to a directory containing ffmpeg
    /// - A path to the ffmpeg executable
    pub fn with_location(location: Option<&Path>) -> Result<Self> {
        let ffmpeg_path = Self::find_ffmpeg(location)?;

        Ok(Self {
            ffmpeg_path,
            version: None,
        })
    }

    /// Find the FFmpeg executable.
    fn find_ffmpeg(location: Option<&Path>) -> Result<PathBuf> {
        let ffmpeg_names = if cfg!(windows) {
            vec!["ffmpeg.exe", "ffmpeg"]
        } else {
            vec!["ffmpeg"]
        };

        let ffmpeg_path = if let Some(loc) = location {
            Self::find_in_location(loc, &ffmpeg_names)?
        } else {
            Self::find_in_path(&ffmpeg_names).ok_or(PostProcessError::FFmpegNotFound)?
        };

        debug!(path:? = ffmpeg_path.display(); "Found FFmpeg");

        Ok(ffmpeg_path)
    }

    /// Find executable in a specific location.
    fn find_in_location(location: &Path, names: &[&str]) -> Result<PathBuf> {
        // If location is a file, check if it's one of the executables
        if location.is_file() {
            if let Some(name) = location.file_name().and_then(|n| n.to_str()) {
                if names
                    .iter()
                    .any(|n| name.contains(n.trim_end_matches(".exe")))
                {
                    return Ok(location.to_path_buf());
                }
            }
            // If it's a file but not the right one, check its directory
            if let Some(dir) = location.parent() {
                for name in names {
                    let path = dir.join(name);
                    if path.exists() {
                        return Ok(path);
                    }
                }
            }
        }

        // If location is a directory, search in it
        if location.is_dir() {
            for name in names {
                let path = location.join(name);
                if path.exists() {
                    return Ok(path);
                }
            }
        }

        Err(PostProcessError::FFmpegNotFound)
    }

    /// Find executable in PATH.
    fn find_in_path(names: &[&str]) -> Option<PathBuf> {
        for name in names {
            if let Ok(path) = which::which(name) {
                return Some(path);
            }
        }
        None
    }

    /// Check if FFmpeg is available.
    pub fn available(&self) -> bool {
        self.ffmpeg_path.exists()
    }

    /// Get the FFmpeg version.
    pub async fn version(&mut self) -> Result<&str> {
        if self.version.is_none() {
            let output = Command::new(&self.ffmpeg_path)
                .arg("-version")
                .output()
                .await
                .map_err(|e| {
                    PostProcessError::ffmpeg_failed_with_source("Failed to get version", e)
                })?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let version = stdout
                .lines()
                .next()
                .and_then(|line| {
                    // Parse "ffmpeg version N.N.N ..." or "ffmpeg version N.N ..."
                    let re = Regex::new(r"ffmpeg version (\S+)").ok()?;
                    re.captures(line).map(|c| c[1].to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());

            self.version = Some(version);
        }

        Ok(self.version.as_ref().unwrap())
    }

    /// Get the path to the FFmpeg executable.
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg_path
    }

    /// Probe a media file using the FFmpeg library and return its information.
    pub async fn probe(&self, path: impl AsRef<Path>) -> Result<MediaInfo> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Err(PostProcessError::InputNotFound { path });
        }

        tokio::task::spawn_blocking(move || Self::probe_sync(&path))
            .await
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("probe task join error: {e}"),
            })?
    }

    /// Probe a media file synchronously using ffmpeg-the-third library.
    fn probe_sync(path: &Path) -> Result<MediaInfo> {
        ensure_init()?;

        let ictx = ffmpeg_the_third::format::input(path).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open {}: {e}", path.display()),
            }
        })?;

        let mut info = MediaInfo {
            path: path.to_path_buf(),
            ..Default::default()
        };

        // Duration (stored in AV_TIME_BASE units)
        let duration_ts = ictx.duration();
        if duration_ts > 0 {
            info.duration =
                Some(duration_ts as f64 / f64::from(ffmpeg_the_third::ffi::AV_TIME_BASE));
        }

        // Format name
        if let Some(fmt) = ictx.format().name().split(',').next() {
            info.format = Some(fmt.to_string());
        }

        // Bit rate
        let bit_rate = ictx.bit_rate();
        if bit_rate > 0 {
            info.bitrate = Some((bit_rate as u64 / 1000) as u32);
        }

        // File size from metadata or filesystem
        info.filesize = std::fs::metadata(path).ok().map(|m| m.len());

        // Format-level metadata
        for (key, value) in ictx.metadata().iter() {
            info.metadata.insert(key.to_lowercase(), value.to_string());
        }

        // Parse streams
        info.stream_count = ictx.streams().count();

        for stream in ictx.streams() {
            let params = stream.parameters();
            let medium = params.medium();

            let codec_name = stream.parameters().id().name().to_string();
            let codec_type_str = match medium {
                ffmpeg_the_third::media::Type::Video => "video",
                ffmpeg_the_third::media::Type::Audio => "audio",
                ffmpeg_the_third::media::Type::Subtitle => "subtitle",
                ffmpeg_the_third::media::Type::Data => "data",
                _ => "unknown",
            };

            let mut stream_info = StreamInfo {
                index: stream.index(),
                codec_type: codec_type_str.to_string(),
                codec_name: Some(codec_name.clone()),
                metadata: HashMap::new(),
            };

            // Stream-level metadata
            for (key, value) in stream.metadata().iter() {
                stream_info
                    .metadata
                    .insert(key.to_lowercase(), value.to_string());
            }

            match medium {
                ffmpeg_the_third::media::Type::Video => {
                    info.has_video = true;
                    if info.video_codec.is_none() {
                        info.video_codec = Some(codec_name);
                    }

                    if let Ok(codec_ctx) =
                        ffmpeg_the_third::codec::context::Context::from_parameters(params)
                    {
                        if let Ok(video) = codec_ctx.decoder().video() {
                            info.width = Some(video.width());
                            info.height = Some(video.height());
                        }
                    }

                    // Frame rate from avg_frame_rate
                    let rate = stream.avg_frame_rate();
                    if rate.denominator() > 0 {
                        let fps =
                            rate.numerator() as f64 / rate.denominator() as f64;
                        if fps > 0.0 && fps < 1000.0 {
                            info.fps = Some(fps);
                        }
                    }

                    // Video bitrate from stream metadata
                    if let Some(br_str) = stream_info.metadata.get("bps") {
                        if let Ok(bps) = br_str.parse::<u64>() {
                            info.video_bitrate = Some((bps / 1000) as u32);
                        }
                    }
                }
                ffmpeg_the_third::media::Type::Audio => {
                    info.has_audio = true;
                    if info.audio_codec.is_none() {
                        info.audio_codec = Some(codec_name);
                    }

                    if let Ok(codec_ctx) =
                        ffmpeg_the_third::codec::context::Context::from_parameters(params)
                    {
                        if let Ok(audio) = codec_ctx.decoder().audio() {
                            info.sample_rate = Some(audio.rate());
                            info.channels =
                                Some(audio.ch_layout().channels() as u8);
                        }
                    }

                    // Audio bitrate from stream metadata
                    if info.audio_bitrate.is_none() {
                        if let Some(br_str) = stream_info.metadata.get("bps") {
                            if let Ok(bps) = br_str.parse::<u64>() {
                                info.audio_bitrate = Some((bps / 1000) as u32);
                            }
                        }
                    }
                }
                _ => {}
            }

            info.streams.push(stream_info);
        }

        Ok(info)
    }

    /// Remux a file (stream copy, no re-encoding) with optional faststart.
    ///
    /// This performs a container-level copy without transcoding, useful for:
    /// - Moving the moov atom to the start of MP4 files (faststart)
    /// - Fixing timestamps and container structure
    /// - Converting between container formats
    pub async fn remux(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        tokio::task::spawn_blocking(move || Self::remux_sync(&input, &output, &opts))
            .await
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("remux task join error: {e}"),
            })?
    }

    /// Remux a single input file synchronously (stream copy).
    fn remux_sync(input: &Path, output: &Path, opts: &RemuxOptions) -> Result<()> {
        ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        let stream_count = ictx.streams().count();
        let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
        let mut ist_time_bases =
            vec![ffmpeg_the_third::Rational(0, 1); stream_count];
        let mut ost_index: i32 = 0;

        for (ist_index, ist) in ictx.streams().enumerate() {
            let medium = ist.parameters().medium();
            if medium != ffmpeg_the_third::media::Type::Video
                && medium != ffmpeg_the_third::media::Type::Audio
            {
                continue;
            }

            stream_mapping[ist_index] = ost_index;
            ist_time_bases[ist_index] = ist.time_base();
            ost_index += 1;

            let mut ost = octx
                .add_stream(ffmpeg_the_third::encoder::find(
                    ffmpeg_the_third::codec::Id::None,
                ))
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add output stream: {e}"),
                })?;
            ost.set_parameters(ist.parameters());
            // Reset codec tag for container compatibility
            unsafe {
                (*(ost.parameters().as_ptr() as *mut ffmpeg_the_third::ffi::AVCodecParameters))
                    .codec_tag = 0;
            }
        }

        // Copy format-level metadata
        octx.set_metadata(ictx.metadata().to_owned());

        // Write header (with faststart if requested)
        if opts.faststart {
            let mut dict = ffmpeg_the_third::Dictionary::new();
            dict.set("movflags", "+faststart");
            octx.write_header_with(dict).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                }
            })?;
        } else {
            octx.write_header().map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                }
            })?;
        }

        // Copy packets
        for result in ictx.packets() {
            let (stream, mut packet) = result.map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                }
            })?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;
            let ost_time_base = octx.stream(ost_idx).unwrap().time_base();
            packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
            packet.set_position(-1);
            packet.set_stream(ost_idx);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output trailer: {e}"),
        })?;

        Ok(())
    }

    /// Merge separate video and audio files into a single container (stream copy).
    ///
    /// Takes two input files (one containing video, one containing audio) and
    /// combines them into a single output file without re-encoding.
    /// The MP4 muxer automatically handles AAC ADTS→ASC conversion when needed.
    pub async fn merge(
        &self,
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
    ) -> Result<()> {
        let video_input = video_input.as_ref().to_path_buf();
        let audio_input = audio_input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        tokio::task::spawn_blocking(move || {
            Self::merge_sync(&video_input, &audio_input, &output, &opts)
        })
        .await
        .map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("merge task join error: {e}"),
        })?
    }

    /// Merge separate video and audio files synchronously (stream copy).
    fn merge_sync(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
        opts: &RemuxOptions,
    ) -> Result<()> {
        ensure_init()?;

        let mut ictx_video =
            ffmpeg_the_third::format::input(video_input).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "failed to open video input {}: {e}",
                        video_input.display()
                    ),
                }
            })?;

        let mut ictx_audio =
            ffmpeg_the_third::format::input(audio_input).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "failed to open audio input {}: {e}",
                        audio_input.display()
                    ),
                }
            })?;

        let mut octx =
            ffmpeg_the_third::format::output(output).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "failed to create output {}: {e}",
                        output.display()
                    ),
                }
            })?;

        // Find best video stream from video input
        let video_ist_index = ictx_video
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoVideoStream)?;

        let video_ist_time_base = ictx_video
            .stream(video_ist_index)
            .unwrap()
            .time_base();

        let mut ost_video = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add video output stream: {e}"),
            })?;
        ost_video.set_parameters(
            ictx_video
                .stream(video_ist_index)
                .unwrap()
                .parameters(),
        );
        unsafe {
            (*(ost_video.parameters().as_ptr() as *mut ffmpeg_the_third::ffi::AVCodecParameters))
                    .codec_tag = 0;
        }
        let video_ost_index = ost_video.index();

        // Find best audio stream from audio input
        let audio_ist_index = ictx_audio
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist_time_base = ictx_audio
            .stream(audio_ist_index)
            .unwrap()
            .time_base();

        let mut ost_audio = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add audio output stream: {e}"),
            })?;
        ost_audio.set_parameters(
            ictx_audio
                .stream(audio_ist_index)
                .unwrap()
                .parameters(),
        );
        unsafe {
            (*(ost_audio.parameters().as_ptr() as *mut ffmpeg_the_third::ffi::AVCodecParameters))
                    .codec_tag = 0;
        }
        let audio_ost_index = ost_audio.index();

        // Write header (with faststart if requested)
        if opts.faststart {
            let mut dict = ffmpeg_the_third::Dictionary::new();
            dict.set("movflags", "+faststart");
            octx.write_header_with(dict).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                }
            })?;
        } else {
            octx.write_header().map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                }
            })?;
        }

        // Copy video packets
        for result in ictx_video.packets() {
            let (stream, mut packet) = result.map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read video packet: {e}"),
                }
            })?;
            if stream.index() != video_ist_index {
                continue;
            }
            let ost_time_base =
                octx.stream(video_ost_index).unwrap().time_base();
            packet.rescale_ts(video_ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(video_ost_index);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write video packet: {e}"),
                }
            })?;
        }

        // Copy audio packets
        for result in ictx_audio.packets() {
            let (stream, mut packet) = result.map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read audio packet: {e}"),
                }
            })?;
            if stream.index() != audio_ist_index {
                continue;
            }
            let ost_time_base =
                octx.stream(audio_ost_index).unwrap().time_base();
            packet.rescale_ts(audio_ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(audio_ost_index);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write audio packet: {e}"),
                }
            })?;
        }

        octx.write_trailer().map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            }
        })?;

        Ok(())
    }

    /// Extract audio from a media file, either by stream copy or transcoding.
    ///
    /// Uses `opts.copy` to determine whether to copy or transcode.
    /// For transcoding, supports bitrate (CBR) and quality scale (VBR) modes.
    pub async fn extract_audio(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &AudioExtractOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        tokio::task::spawn_blocking(move || Self::extract_audio_sync(&input, &output, &opts))
            .await
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("extract_audio task join error: {e}"),
            })?
    }

    /// Extract audio synchronously (dispatches to copy or transcode).
    fn extract_audio_sync(input: &Path, output: &Path, opts: &AudioExtractOptions) -> Result<()> {
        if opts.copy {
            Self::extract_audio_copy_sync(input, output)
        } else {
            Self::extract_audio_transcode_sync(input, output, opts)
        }
    }

    /// Extract audio by stream copy (no re-encoding).
    ///
    /// Maps only the best audio stream from input to output without transcoding.
    fn extract_audio_copy_sync(input: &Path, output: &Path) -> Result<()> {
        ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Find best audio stream
        let ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let ist_time_base = ictx.stream(ist_index).unwrap().time_base();

        // Add output stream (stream copy mode)
        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add output stream: {e}"),
            })?;
        ost.set_parameters(ictx.stream(ist_index).unwrap().parameters());
        unsafe {
            (*(ost.parameters().as_ptr() as *mut ffmpeg_the_third::ffi::AVCodecParameters))
                .codec_tag = 0;
        }

        octx.write_header().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output header: {e}"),
        })?;

        // Copy only audio packets
        for result in ictx.packets() {
            let (stream, mut packet) = result.map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                }
            })?;
            if stream.index() != ist_index {
                continue;
            }
            let ost_time_base = octx.stream(0).unwrap().time_base();
            packet.rescale_ts(ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(0);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output trailer: {e}"),
        })?;

        Ok(())
    }

    /// Extract audio by transcoding to a target codec.
    ///
    /// Decodes the input audio, optionally converts sample format/rate through
    /// a filter graph, and encodes to the target codec.
    fn extract_audio_transcode_sync(
        input: &Path,
        output: &Path,
        opts: &AudioExtractOptions,
    ) -> Result<()> {
        ensure_init()?;

        // Open input and find audio stream
        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let ist_time_base = ictx.stream(ist_index).unwrap().time_base();

        // Create decoder (bind stream to extend its lifetime for parameters())
        let ist = ictx.stream(ist_index).unwrap();
        let decoder_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(ist.parameters())?;
        let mut decoder = decoder_ctx.decoder().audio()?;

        // Open output
        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Find encoder codec
        let enc_codec = if let Some(ref name) = opts.encoder_name {
            ffmpeg_the_third::encoder::find_by_name(name).ok_or_else(|| {
                PostProcessError::UnsupportedCodec {
                    codec: name.clone(),
                    operation: "audio extraction".into(),
                }
            })?
        } else {
            let codec_id = octx
                .format()
                .codec(output, ffmpeg_the_third::media::Type::Audio);
            ffmpeg_the_third::encoder::find(codec_id).ok_or_else(|| {
                PostProcessError::ffmpeg_failed("no default encoder for output format")
            })?
        };

        // Check global header flag BEFORE taking mutable stream borrow
        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        // Add output stream and create encoder context (scoped to release octx borrow)
        let ost_index;
        let enc_context;
        {
            let ost = octx.add_stream(enc_codec).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add output stream: {e}"),
                }
            })?;
            ost_index = ost.index();
            enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }
        // ost dropped — octx no longer mutably borrowed

        // Configure encoder
        let mut audio_encoder = enc_context.encoder().audio()?;

        let target_format = Self::pick_audio_sample_format(&enc_codec, decoder.format());
        audio_encoder.set_format(target_format);
        audio_encoder.set_rate(decoder.rate() as i32);
        audio_encoder.set_time_base(ffmpeg_the_third::Rational(1, decoder.rate() as i32));

        // Set channel layout from decoder (default layout matching channel count)
        let channels = decoder.ch_layout().channels();
        unsafe {
            ffmpeg_the_third::ffi::av_channel_layout_default(
                &mut (*audio_encoder.as_mut_ptr()).ch_layout,
                channels as i32,
            );
        }

        // Set bitrate (CBR)
        if let Some(br_kbps) = opts.bitrate_kbps {
            audio_encoder.set_bit_rate((br_kbps as usize) * 1000);
        }

        // Set VBR quality
        if let Some(quality) = opts.quality_scale {
            unsafe {
                let ctx = audio_encoder.as_mut_ptr();
                (*ctx).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_QSCALE as i32;
                (*ctx).global_quality = quality * ffmpeg_the_third::ffi::FF_QP2LAMBDA;
            }
        }

        // Set global header flag if required by output format
        if needs_global_header {
            unsafe {
                (*audio_encoder.as_mut_ptr()).flags |=
                    ffmpeg_the_third::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }

        // Open encoder
        let mut audio_encoder = audio_encoder.open_as(enc_codec).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open audio encoder: {e}"),
            }
        })?;

        // Copy encoder parameters back to output stream via FFI
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(ost_index);
            ffmpeg_the_third::ffi::avcodec_parameters_from_context(
                (*stream_ptr).codecpar,
                audio_encoder.as_ptr(),
            );
        }

        octx.write_header().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output header: {e}"),
        })?;

        // Build filter graph for sample format/rate conversion
        let mut filter_graph =
            Self::build_audio_filter(&decoder, &audio_encoder, ist_time_base)?;

        // Transcode loop: read → decode → filter → encode → write
        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                }
            })?;
            if stream.index() != ist_index {
                continue;
            }
            decoder.send_packet(&packet)?;
            Self::receive_and_process_audio(
                &mut decoder,
                &mut filter_graph,
                &mut audio_encoder,
                &mut octx,
                ost_index,
            )?;
        }

        // Flush decoder
        decoder.send_eof()?;
        Self::receive_and_process_audio(
            &mut decoder,
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            ost_index,
        )?;

        // Flush filter graph (signal EOF to source)
        filter_graph.get("in").unwrap().source().flush()?;
        Self::drain_filter_to_encoder(
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            ost_index,
        )?;

        // Flush encoder
        audio_encoder.send_eof()?;
        Self::drain_encoder_packets(&mut audio_encoder, &mut octx, ost_index)?;

        octx.write_trailer().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output trailer: {e}"),
        })?;

        Ok(())
    }

    /// Pick a sample format supported by the encoder, preferring the decoder's format.
    fn pick_audio_sample_format(
        codec: &ffmpeg_the_third::Codec,
        preferred: ffmpeg_the_third::format::Sample,
    ) -> ffmpeg_the_third::format::Sample {
        // Check codec's supported sample formats
        unsafe {
            let ptr = codec.as_ptr();
            let sample_fmts = (*ptr).sample_fmts;
            if sample_fmts.is_null() {
                // Codec accepts any format
                return preferred;
            }

            let mut i = 0;
            let mut first = None;
            loop {
                let fmt = *sample_fmts.offset(i);
                if fmt == ffmpeg_the_third::ffi::AVSampleFormat::AV_SAMPLE_FMT_NONE {
                    break;
                }
                let sample = ffmpeg_the_third::format::Sample::from(fmt);
                if first.is_none() {
                    first = Some(sample);
                }
                if sample == preferred {
                    return preferred;
                }
                i += 1;
            }

            first.unwrap_or(preferred)
        }
    }

    /// Build an audio filter graph for sample format/rate/channel conversion.
    ///
    /// Uses `abuffer` → `anull` → `abuffersink` to let FFmpeg handle any
    /// necessary sample format, sample rate, or channel layout conversions.
    fn build_audio_filter(
        decoder: &ffmpeg_the_third::decoder::Audio,
        encoder: &ffmpeg_the_third::encoder::audio::Audio,
        ist_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let mut graph = ffmpeg_the_third::filter::Graph::new();

        let abuffer = ffmpeg_the_third::filter::find("abuffer").ok_or_else(|| {
            PostProcessError::ffmpeg_failed("abuffer filter not found")
        })?;
        let abuffersink = ffmpeg_the_third::filter::find("abuffersink").ok_or_else(|| {
            PostProcessError::ffmpeg_failed("abuffersink filter not found")
        })?;

        // Build abuffer args with decoder's output parameters
        let channels = decoder.ch_layout().channels();
        let args = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:chlayout={}c",
            ist_time_base.numerator(),
            ist_time_base.denominator(),
            decoder.rate(),
            decoder.format().name(),
            channels,
        );

        graph.add(&abuffer, "in", &args).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffer filter: {e}"),
            }
        })?;
        graph.add(&abuffersink, "out", "").map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffersink filter: {e}"),
            }
        })?;

        // Build aformat spec to convert to encoder's expected format
        let enc_channels = encoder.ch_layout().channels();
        let aformat_spec = format!(
            "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}c",
            encoder.format().name(),
            encoder.rate(),
            enc_channels,
        );

        graph
            .output("out", 0)?
            .input("in", 0)?
            .parse(&aformat_spec)?;
        graph.validate()?;

        Ok(graph)
    }

    /// Receive decoded frames from decoder, push through filter, encode, and write.
    fn receive_and_process_audio(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter.get("in").unwrap().source().add(&frame)?;
            Self::drain_filter_to_encoder(filter, encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Pull filtered frames from filter graph, encode, and write.
    fn drain_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        while filter
            .get("out")
            .unwrap()
            .sink()
            .frame(&mut filtered)
            .is_ok()
        {
            encoder.send_frame(&filtered)?;
            Self::drain_encoder_packets(encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Receive encoded packets from encoder and write to output.
    fn drain_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.write_interleaved(octx)?;
        }
        Ok(())
    }

    /// Convert a video file, either by remuxing or transcoding.
    ///
    /// Uses `opts.remux_only` to determine whether to stream-copy or transcode.
    /// For transcoding, encodes video with the specified codec while optionally
    /// copying the audio stream unchanged.
    pub async fn convert_video(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &VideoConvertOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        tokio::task::spawn_blocking(move || Self::convert_video_sync(&input, &output, &opts))
            .await
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("convert_video task join error: {e}"),
            })?
    }

    /// Convert video synchronously (dispatches to remux or transcode).
    fn convert_video_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
    ) -> Result<()> {
        if opts.remux_only {
            // Determine if output is MP4/MOV for faststart
            let ext = output
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let remux_opts = RemuxOptions {
                faststart: ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov"),
                ..Default::default()
            };
            Self::remux_sync(input, output, &remux_opts)
        } else {
            Self::convert_video_transcode_sync(input, output, opts)
        }
    }

    /// Transcode video to a target codec, optionally copying audio.
    ///
    /// Decodes video frames, converts pixel format through a filter graph,
    /// and encodes with the target video codec. Audio is stream-copied if
    /// `opts.audio_copy` is true.
    fn convert_video_transcode_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
    ) -> Result<()> {
        ensure_init()?;

        // Open input
        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        // Find video and audio stream indices
        let video_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoVideoStream)?;

        let audio_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index());

        // Capture stream time bases before any mutable borrows
        let video_ist_time_base = ictx.stream(video_ist_index).unwrap().time_base();
        let video_ist_frame_rate = ictx.stream(video_ist_index).unwrap().avg_frame_rate();
        let audio_ist_time_base =
            audio_ist_index.map(|i| ictx.stream(i).unwrap().time_base());

        // Create video decoder
        let video_ist = ictx.stream(video_ist_index).unwrap();
        let video_dec_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(video_ist.parameters())?;
        let mut video_decoder = video_dec_ctx.decoder().video()?;

        // Open output
        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Find video encoder
        let video_codec_name = opts.video_codec.as_deref().unwrap_or("libx264");
        let video_enc_codec =
            ffmpeg_the_third::encoder::find_by_name(video_codec_name).ok_or_else(|| {
                PostProcessError::UnsupportedCodec {
                    codec: video_codec_name.to_string(),
                    operation: "video conversion".into(),
                }
            })?;

        // Check global header flag before mutable stream borrows
        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        // Add video output stream (scoped to release octx borrow)
        let video_ost_index;
        let video_enc_context;
        {
            let ost = octx.add_stream(video_enc_codec).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add video output stream: {e}"),
                }
            })?;
            video_ost_index = ost.index();
            video_enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }

        // Configure video encoder
        let mut video_encoder = video_enc_context.encoder().video()?;
        video_encoder.set_width(video_decoder.width());
        video_encoder.set_height(video_decoder.height());

        let target_pix_fmt =
            Self::pick_video_pixel_format(&video_enc_codec, video_decoder.format());
        video_encoder.set_format(target_pix_fmt);

        // Set time base from frame rate (inverse of fps)
        if video_ist_frame_rate.numerator() > 0 && video_ist_frame_rate.denominator() > 0 {
            video_encoder.set_time_base(ffmpeg_the_third::Rational(
                video_ist_frame_rate.denominator(),
                video_ist_frame_rate.numerator(),
            ));
        } else {
            video_encoder.set_time_base(video_ist_time_base);
        }

        // Set frame rate
        video_encoder.set_frame_rate(Some(video_ist_frame_rate));

        if needs_global_header {
            unsafe {
                (*video_encoder.as_mut_ptr()).flags |=
                    ffmpeg_the_third::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }

        // Open encoder with preset/CRF options
        let mut enc_opts = ffmpeg_the_third::Dictionary::new();
        if let Some(ref preset) = opts.preset {
            enc_opts.set("preset", preset);
        }
        if let Some(crf) = opts.crf {
            enc_opts.set("crf", &crf.to_string());
        }

        // For VP9: set bitrate to 0 for pure CRF mode
        if video_codec_name.contains("vpx") && opts.crf.is_some() {
            video_encoder.set_bit_rate(0);
        }

        let mut video_encoder = video_encoder.open_as_with(video_enc_codec, enc_opts).map_err(
            |e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to open video encoder: {e}"),
            },
        )?;

        // Copy encoder parameters back to output stream
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(video_ost_index);
            ffmpeg_the_third::ffi::avcodec_parameters_from_context(
                (*stream_ptr).codecpar,
                video_encoder.as_ptr(),
            );
        }

        // Add audio output stream (stream copy) if audio exists and copy requested
        let audio_ost_index = if opts.audio_copy {
            if let Some(audio_idx) = audio_ist_index {
                let audio_ost_idx;
                {
                    let mut ost = octx
                        .add_stream(ffmpeg_the_third::encoder::find(
                            ffmpeg_the_third::codec::Id::None,
                        ))
                        .map_err(|e| PostProcessError::FFmpegLibraryError {
                            message: format!("failed to add audio output stream: {e}"),
                        })?;
                    ost.set_parameters(ictx.stream(audio_idx).unwrap().parameters());
                    audio_ost_idx = ost.index();
                    unsafe {
                        (*(ost.parameters().as_ptr()
                            as *mut ffmpeg_the_third::ffi::AVCodecParameters))
                            .codec_tag = 0;
                    }
                }
                Some(audio_ost_idx)
            } else {
                None
            }
        } else {
            None
        };

        octx.write_header().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output header: {e}"),
        })?;

        // Build video filter graph for pixel format conversion
        let mut filter_graph =
            Self::build_video_filter(&video_decoder, &video_encoder, video_ist_time_base)?;

        // Process packets: video → decode/filter/encode, audio → copy
        for result in ictx.packets() {
            let (stream, mut packet) = result.map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                }
            })?;
            let ist_index = stream.index();

            if ist_index == video_ist_index {
                // Video: decode → filter → encode → write
                video_decoder.send_packet(&packet)?;
                Self::receive_and_process_video(
                    &mut video_decoder,
                    &mut filter_graph,
                    &mut video_encoder,
                    &mut octx,
                    video_ost_index,
                )?;
            } else if Some(ist_index) == audio_ist_index {
                // Audio: stream copy
                if let Some(audio_ost_idx) = audio_ost_index {
                    let ost_time_base = octx.stream(audio_ost_idx).unwrap().time_base();
                    packet.rescale_ts(audio_ist_time_base.unwrap(), ost_time_base);
                    packet.set_position(-1);
                    packet.set_stream(audio_ost_idx);
                    packet.write_interleaved(&mut octx).map_err(|e| {
                        PostProcessError::FFmpegLibraryError {
                            message: format!("failed to write audio packet: {e}"),
                        }
                    })?;
                }
            }
        }

        // Flush video decoder
        video_decoder.send_eof()?;
        Self::receive_and_process_video(
            &mut video_decoder,
            &mut filter_graph,
            &mut video_encoder,
            &mut octx,
            video_ost_index,
        )?;

        // Flush video filter graph
        filter_graph.get("in").unwrap().source().flush()?;
        Self::drain_video_filter_to_encoder(
            &mut filter_graph,
            &mut video_encoder,
            &mut octx,
            video_ost_index,
        )?;

        // Flush video encoder
        video_encoder.send_eof()?;
        Self::drain_video_encoder_packets(&mut video_encoder, &mut octx, video_ost_index)?;

        octx.write_trailer().map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to write output trailer: {e}"),
        })?;

        Ok(())
    }

    /// Pick a pixel format supported by the video encoder, preferring the decoder's format.
    fn pick_video_pixel_format(
        codec: &ffmpeg_the_third::Codec,
        preferred: ffmpeg_the_third::format::Pixel,
    ) -> ffmpeg_the_third::format::Pixel {
        unsafe {
            let ptr = codec.as_ptr();
            let pix_fmts = (*ptr).pix_fmts;
            if pix_fmts.is_null() {
                return preferred;
            }

            let mut i = 0;
            let mut first = None;
            loop {
                let fmt = *pix_fmts.offset(i);
                if fmt == ffmpeg_the_third::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    break;
                }
                let pixel = ffmpeg_the_third::format::Pixel::from(fmt);
                if first.is_none() {
                    first = Some(pixel);
                }
                if pixel == preferred {
                    return preferred;
                }
                i += 1;
            }

            first.unwrap_or(preferred)
        }
    }

    /// Build a video filter graph for pixel format conversion.
    ///
    /// Uses `buffer` → `format` → `buffersink` to convert pixel format
    /// from decoder output to encoder input format.
    fn build_video_filter(
        decoder: &ffmpeg_the_third::decoder::Video,
        encoder: &ffmpeg_the_third::encoder::video::Video,
        ist_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let mut graph = ffmpeg_the_third::filter::Graph::new();

        let buffer = ffmpeg_the_third::filter::find("buffer").ok_or_else(|| {
            PostProcessError::ffmpeg_failed("buffer filter not found")
        })?;
        let buffersink = ffmpeg_the_third::filter::find("buffersink").ok_or_else(|| {
            PostProcessError::ffmpeg_failed("buffersink filter not found")
        })?;

        // Pixel aspect ratio (default 1:1 if unknown)
        let sar = decoder.aspect_ratio();
        let sar_num = if sar.numerator() > 0 {
            sar.numerator()
        } else {
            1
        };
        let sar_den = if sar.denominator() > 0 {
            sar.denominator()
        } else {
            1
        };

        let args = format!(
            "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
            decoder.width(),
            decoder.height(),
            decoder.format() as i32,
            ist_time_base.numerator(),
            ist_time_base.denominator(),
            sar_num,
            sar_den,
        );

        graph.add(&buffer, "in", &args).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffer filter: {e}"),
            }
        })?;
        graph.add(&buffersink, "out", "").map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffersink filter: {e}"),
            }
        })?;

        // Convert pixel format to match encoder's requirement
        let enc_pix_fmt_name = encoder
            .format()
            .descriptor()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|| "yuv420p".to_string());

        let format_spec = format!("format=pix_fmts={enc_pix_fmt_name}");

        graph
            .output("out", 0)?
            .input("in", 0)?
            .parse(&format_spec)?;
        graph.validate()?;

        Ok(graph)
    }

    /// Receive decoded video frames, push through filter, encode, and write.
    fn receive_and_process_video(
        decoder: &mut ffmpeg_the_third::decoder::Video,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter.get("in").unwrap().source().add(&frame)?;
            Self::drain_video_filter_to_encoder(filter, encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Pull filtered video frames from filter graph, encode, and write.
    fn drain_video_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Video::empty();
        while filter
            .get("out")
            .unwrap()
            .sink()
            .frame(&mut filtered)
            .is_ok()
        {
            encoder.send_frame(&filtered)?;
            Self::drain_video_encoder_packets(encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Receive encoded video packets from encoder and write to output.
    fn drain_video_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.write_interleaved(octx)?;
        }
        Ok(())
    }

    /// Run FFmpeg with the given arguments.
    ///
    /// This method automatically adds `-y` (overwrite) and sets an appropriate loglevel.
    pub async fn run(&self, args: &[&str]) -> Result<Output> {
        self.run_with_options(args, true, "error").await
    }

    /// Run FFmpeg with custom options.
    pub async fn run_with_options(
        &self,
        args: &[&str],
        overwrite: bool,
        loglevel: &str,
    ) -> Result<Output> {
        let mut cmd = Command::new(&self.ffmpeg_path);

        if overwrite {
            cmd.arg("-y");
        }

        cmd.args(["-loglevel", loglevel]);
        cmd.args(args);

        trace!(
            "Running FFmpeg: {} {}",
            self.ffmpeg_path.display(),
            args.join(" ")
        );

        let output = cmd
            .output()
            .await
            .map_err(|e| PostProcessError::ffmpeg_failed_with_source("Failed to run FFmpeg", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PostProcessError::FFmpegExitCode {
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.to_string(),
            });
        }

        Ok(output)
    }

    /// Run FFmpeg with input and output file paths.
    ///
    /// Handles special characters in filenames by using the `file:` protocol.
    pub async fn run_with_files(
        &self,
        inputs: &[&Path],
        output: &Path,
        opts: &[&str],
    ) -> Result<Output> {
        let mut args = Vec::new();

        // Add input files
        for input in inputs {
            args.push("-i".to_string());
            args.push(Self::filename_arg(input));
        }

        // Add options
        args.extend(opts.iter().map(|s| s.to_string()));

        // Add output
        args.push(Self::filename_arg(output));

        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&args_refs).await
    }

    /// Create a filename argument with proper escaping.
    fn filename_arg(path: &Path) -> String {
        let path_str = path.to_string_lossy();

        // Use file: protocol for paths that might contain special characters
        if path_str.starts_with('-') || path_str.contains(':') && !path_str.starts_with("file:") {
            format!("file:{path_str}")
        } else {
            path_str.to_string()
        }
    }

    /// Get the audio codec from a file.
    pub async fn get_audio_codec(&self, path: impl AsRef<Path>) -> Result<Option<String>> {
        let info = self.probe(path).await?;
        Ok(info.audio_codec)
    }

    /// Check if a file has an audio stream.
    pub async fn has_audio(&self, path: impl AsRef<Path>) -> Result<bool> {
        let info = self.probe(path).await?;
        Ok(info.has_audio)
    }

    /// Check if a file has a video stream.
    pub async fn has_video(&self, path: impl AsRef<Path>) -> Result<bool> {
        let info = self.probe(path).await?;
        Ok(info.has_video)
    }
}

impl MediaInfo {
    /// Get a resolution string (e.g., "1920x1080").
    pub fn resolution_string(&self) -> Option<String> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        }
    }

    /// Get the video stream info.
    pub fn video_stream(&self) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.codec_type == "video")
    }

    /// Get the audio stream info.
    pub fn audio_stream(&self) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.codec_type == "audio")
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
    fn test_filename_arg() {
        // Normal path
        assert_eq!(
            FFmpegRunner::filename_arg(Path::new("video.mp4")),
            "video.mp4"
        );

        // Path starting with dash
        assert_eq!(
            FFmpegRunner::filename_arg(Path::new("-output.mp4")),
            "file:-output.mp4"
        );
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
