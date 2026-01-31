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
//! use rdlp_postprocess::ffmpeg::FFmpegRunner;
//!
//! # async fn example() -> rdlp_postprocess::error::Result<()> {
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use log::debug;
use serde::{Deserialize, Serialize};

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
#[must_use]
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

/// A chapter entry for metadata embedding.
#[derive(Debug, Clone)]
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

    /// Probe a media file using the FFmpeg library and return its information.
    pub async fn probe(&self, path: impl AsRef<Path>) -> Result<MediaInfo> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Err(PostProcessError::InputNotFound { path });
        }

        Self::spawn_blocking("probe", move || Self::probe_sync(&path)).await
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
                        let fps = rate.numerator() as f64 / rate.denominator() as f64;
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
                            info.channels = Some(audio.ch_layout().channels() as u8);
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
        Self::spawn_blocking("remux", move || Self::remux_sync(&input, &output, &opts)).await
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
        let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
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
            Self::clear_codec_tag(ost.parameters().as_ptr());
        }

        // Copy format-level metadata
        octx.set_metadata(ictx.metadata().to_owned());

        // Write header (with faststart if requested)
        if opts.faststart {
            let mut dict = ffmpeg_the_third::Dictionary::new();
            dict.set("movflags", "+faststart");
            octx.write_header_with(dict)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                })?;
        } else {
            octx.write_header()
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                })?;
        }

        // Copy packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;
            let ost_time_base = octx
                .stream(ost_idx)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("output stream {ost_idx} not found"))
                })?
                .time_base();
            packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
            packet.set_position(-1);
            packet.set_stream(ost_idx);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
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
        Self::spawn_blocking("merge", move || {
            Self::merge_sync(&video_input, &audio_input, &output, &opts)
        })
        .await
    }

    /// Merge separate video and audio files synchronously (stream copy).
    fn merge_sync(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
        opts: &RemuxOptions,
    ) -> Result<()> {
        ensure_init()?;

        let mut ictx_video = ffmpeg_the_third::format::input(video_input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open video input {}: {e}", video_input.display()),
            }
        })?;

        let mut ictx_audio = ffmpeg_the_third::format::input(audio_input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open audio input {}: {e}", audio_input.display()),
            }
        })?;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
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
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?
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
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "video input stream {video_ist_index} not found"
                    ))
                })?
                .parameters(),
        );
        Self::clear_codec_tag(ost_video.parameters().as_ptr());
        let video_ost_index = ost_video.index();

        // Find best audio stream from audio input
        let audio_ist_index = ictx_audio
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist_time_base = ictx_audio
            .stream(audio_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "audio input stream {audio_ist_index} not found"
                ))
            })?
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
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {audio_ist_index} not found"
                    ))
                })?
                .parameters(),
        );
        Self::clear_codec_tag(ost_audio.parameters().as_ptr());
        let audio_ost_index = ost_audio.index();

        // Write header (with faststart if requested)
        if opts.faststart {
            let mut dict = ffmpeg_the_third::Dictionary::new();
            dict.set("movflags", "+faststart");
            octx.write_header_with(dict)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                })?;
        } else {
            octx.write_header()
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write output header: {e}"),
                })?;
        }

        // Copy video packets
        for result in ictx_video.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read video packet: {e}"),
                })?;
            if stream.index() != video_ist_index {
                continue;
            }
            let ost_time_base = octx
                .stream(video_ost_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "video output stream {video_ost_index} not found"
                    ))
                })?
                .time_base();
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
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read audio packet: {e}"),
                })?;
            if stream.index() != audio_ist_index {
                continue;
            }
            let ost_time_base = octx
                .stream(audio_ost_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio output stream {audio_ost_index} not found"
                    ))
                })?
                .time_base();
            packet.rescale_ts(audio_ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(audio_ost_index);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write audio packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
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
        Self::spawn_blocking("extract_audio", move || {
            Self::extract_audio_sync(&input, &output, &opts)
        })
        .await
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

        let ist_time_base = ictx
            .stream(ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
            })?
            .time_base();

        // Add output stream (stream copy mode)
        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add output stream: {e}"),
            })?;
        ost.set_parameters(
            ictx.stream(ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {ist_index} not found"
                    ))
                })?
                .parameters(),
        );
        Self::clear_codec_tag(ost.parameters().as_ptr());

        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy only audio packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            if stream.index() != ist_index {
                continue;
            }
            let ost_time_base = octx
                .stream(0)
                .ok_or_else(|| PostProcessError::ffmpeg_failed("output stream 0 not found"))?
                .time_base();
            packet.rescale_ts(ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(0);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
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

        let ist_time_base = ictx
            .stream(ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
            })?
            .time_base();

        // Create decoder (bind stream to extend its lifetime for parameters())
        let ist = ictx.stream(ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
        })?;
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
            let ost =
                octx.add_stream(enc_codec)
                    .map_err(|e| PostProcessError::FFmpegLibraryError {
                        message: format!("failed to add output stream: {e}"),
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
        // SAFETY: audio_encoder is a valid pre-open encoder context.
        Self::set_default_channel_layout(unsafe { audio_encoder.as_mut_ptr() }, channels as i32);

        // Set bitrate (CBR)
        if let Some(br_kbps) = opts.bitrate_kbps {
            audio_encoder.set_bit_rate((br_kbps as usize) * 1000);
        }

        // Set VBR quality
        if let Some(quality) = opts.quality_scale {
            // SAFETY: audio_encoder is a valid pre-open encoder context.
            Self::set_vbr_quality(unsafe { audio_encoder.as_mut_ptr() }, quality);
        }

        // Set global header flag if required by output format
        if needs_global_header {
            // SAFETY: audio_encoder is a valid pre-open encoder context.
            Self::set_global_header_flag(unsafe { audio_encoder.as_mut_ptr() });
        }

        // Open encoder
        let mut audio_encoder =
            audio_encoder
                .open_as(enc_codec)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open audio encoder: {e}"),
                })?;

        // Copy encoder parameters back to output stream
        // SAFETY: audio_encoder is a valid opened encoder context.
        Self::copy_encoder_params_to_stream(&mut octx, ost_index, unsafe {
            audio_encoder.as_ptr()
        });

        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Build filter graph for sample format/rate conversion
        let mut filter_graph = Self::build_audio_filter(&decoder, &audio_encoder, ist_time_base)?;

        // Transcode loop: read → decode → filter → encode → write
        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
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
        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_filter_to_encoder(&mut filter_graph, &mut audio_encoder, &mut octx, ost_index)?;

        // Flush encoder
        audio_encoder.send_eof()?;
        Self::drain_encoder_packets(&mut audio_encoder, &mut octx, ost_index)?;

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
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

        let abuffer = ffmpeg_the_third::filter::find("abuffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffer filter not found"))?;
        let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

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

        graph
            .add(&abuffer, "in", &args)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffer filter: {e}"),
            })?;
        graph
            .add(&abuffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffersink filter: {e}"),
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
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)?;
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
        loop {
            let mut out_node = filter
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
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

    /// Embed metadata and chapters into a media file via stream copy (remux).
    ///
    /// Copies all streams without re-encoding, sets format-level metadata via
    /// `Dictionary`, and adds chapters via `add_chapter()`. No temporary
    /// FFMETADATA1 file is needed.
    pub async fn embed_metadata(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        metadata: &HashMap<String, String>,
        chapters: &[ChapterEntry],
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let metadata = metadata.clone();
        let chapters: Vec<ChapterEntry> = chapters.to_vec();
        Self::spawn_blocking("embed_metadata", move || {
            Self::embed_metadata_sync(&input, &output, &metadata, &chapters)
        })
        .await
    }

    /// Embed metadata and chapters synchronously.
    ///
    /// Remuxes (stream copies) the input to output while:
    /// - Setting format-level metadata from the provided `HashMap`
    /// - Adding chapters with millisecond precision (time_base = 1/1000)
    fn embed_metadata_sync(
        input: &Path,
        output: &Path,
        metadata: &HashMap<String, String>,
        chapters: &[ChapterEntry],
    ) -> Result<()> {
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

        // Map all streams (stream copy)
        let stream_count = ictx.streams().count();
        let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
        let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
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
            Self::clear_codec_tag(ost.parameters().as_ptr());
        }

        // Build metadata dictionary from input metadata + provided overrides
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // Copy existing metadata from input first
        for (k, v) in ictx.metadata().iter() {
            dict.set(k, v);
        }

        // Apply provided metadata (overrides existing keys)
        for (k, v) in metadata {
            dict.set(k, v);
        }

        octx.set_metadata(dict);

        // Add chapters (time_base = 1/1000 for millisecond precision)
        for ch in chapters {
            octx.add_chapter(
                ch.id,
                ffmpeg_the_third::Rational(1, 1000),
                ch.start_ms,
                ch.end_ms,
                &ch.title,
            )
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add chapter '{}': {e}", ch.title),
            })?;
        }

        // Write header
        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;
            let ost_time_base = octx
                .stream(ost_idx)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("output stream {ost_idx} not found"))
                })?
                .time_base();
            packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
            packet.set_position(-1);
            packet.set_stream(ost_idx);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Embed a thumbnail image into a media file via stream copy (remux).
    ///
    /// Opens both the media file and thumbnail image, copies all media streams,
    /// and adds the thumbnail as a video stream with `ATTACHED_PIC` disposition.
    /// Container-specific handling for MKV (attachment) and MP3 (ID3v2).
    pub async fn embed_thumbnail(
        &self,
        media: impl AsRef<Path>,
        thumbnail: impl AsRef<Path>,
        output: impl AsRef<Path>,
        container: &str,
    ) -> Result<()> {
        let media = media.as_ref().to_path_buf();
        let thumbnail = thumbnail.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let container = container.to_string();
        Self::spawn_blocking("embed_thumbnail", move || {
            Self::embed_thumbnail_sync(&media, &thumbnail, &output, &container)
        })
        .await
    }

    /// Embed thumbnail synchronously.
    ///
    /// Strategy varies by container:
    /// - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
    /// - **MKV/MKA**: Map all streams + thumbnail as attachment with mimetype metadata
    /// - **MP3**: Map audio only + thumbnail as video with ID3v2 metadata
    /// - **FLAC/OGG/Opus**: Map all streams + thumbnail with `ATTACHED_PIC`
    fn embed_thumbnail_sync(
        media: &Path,
        thumbnail: &Path,
        output: &Path,
        container: &str,
    ) -> Result<()> {
        ensure_init()?;

        // Open media input
        let mut ictx = ffmpeg_the_third::format::input(media).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open media input {}: {e}", media.display()),
            }
        })?;

        // Open thumbnail input
        let mut thumb_ictx = ffmpeg_the_third::format::input(thumbnail).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open thumbnail {}: {e}", thumbnail.display()),
            }
        })?;

        // Create output
        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        let is_mp3 = container.eq_ignore_ascii_case("mp3");
        let is_mkv = matches!(container.to_lowercase().as_str(), "mkv" | "mka");

        // Map media streams to output
        let stream_count = ictx.streams().count();
        let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
        let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
        let mut ost_index: i32 = 0;

        for (ist_index, ist) in ictx.streams().enumerate() {
            let medium = ist.parameters().medium();

            // For MP3: only map audio streams (thumbnail replaces any video)
            if is_mp3 && medium != ffmpeg_the_third::media::Type::Audio {
                continue;
            }

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
            Self::clear_codec_tag(ost.parameters().as_ptr());
        }

        // Add thumbnail stream
        let thumb_ist = thumb_ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .ok_or(PostProcessError::ffmpeg_failed(
                "no video stream found in thumbnail",
            ))?;
        let thumb_ist_index = thumb_ist.index();
        let thumb_ist_time_base = thumb_ist.time_base();
        let thumb_params = thumb_ist.parameters();

        let thumb_ost_index;
        if is_mkv {
            // MKV: add as attachment stream
            let mut ost = octx
                .add_stream(ffmpeg_the_third::encoder::find(
                    ffmpeg_the_third::codec::Id::None,
                ))
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add thumbnail attachment stream: {e}"),
                })?;
            thumb_ost_index = ost.index();
            // Set codec parameters from thumbnail
            ost.set_parameters(thumb_params);
            // Override to attachment type for MKV
            Self::set_stream_as_attachment(ost.parameters().as_ptr());
            // Set attachment metadata
            {
                let mut dict = ffmpeg_the_third::Dictionary::new();
                dict.set("mimetype", Self::thumbnail_mimetype(thumbnail));
                dict.set("filename", "cover.jpg");
                ost.set_metadata(dict);
            }
        } else {
            // All other containers: add as video stream with ATTACHED_PIC
            let mut ost = octx
                .add_stream(ffmpeg_the_third::encoder::find(
                    ffmpeg_the_third::codec::Id::None,
                ))
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add thumbnail stream: {e}"),
                })?;
            thumb_ost_index = ost.index();
            ost.set_parameters(thumb_params);
            // SAFETY: ost is a valid output stream in a live output context.
            Self::set_attached_pic_disposition(unsafe { ost.as_mut_ptr() });

            // For MP3: set ID3v2 metadata on the thumbnail stream
            if is_mp3 {
                let mut dict = ffmpeg_the_third::Dictionary::new();
                dict.set("title", "Album cover");
                dict.set("comment", "Cover (front)");
                ost.set_metadata(dict);
            }
        }

        // Copy format-level metadata from media input
        octx.set_metadata(ictx.metadata().to_owned());

        // Write header
        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy media packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read media packet: {e}"),
                })?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;
            let ost_time_base = octx
                .stream(ost_idx)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("output stream {ost_idx} not found"))
                })?
                .time_base();
            packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
            packet.set_position(-1);
            packet.set_stream(ost_idx);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write media packet: {e}"),
                }
            })?;
        }

        // Copy thumbnail packet(s)
        let thumb_ost_time_base = octx
            .stream(thumb_ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "thumbnail output stream {thumb_ost_index} not found"
                ))
            })?
            .time_base();
        for result in thumb_ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read thumbnail packet: {e}"),
                })?;
            if stream.index() == thumb_ist_index {
                packet.rescale_ts(thumb_ist_time_base, thumb_ost_time_base);
                packet.set_position(-1);
                packet.set_stream(thumb_ost_index);
                packet.write_interleaved(&mut octx).map_err(|e| {
                    PostProcessError::FFmpegLibraryError {
                        message: format!("failed to write thumbnail packet: {e}"),
                    }
                })?;
            }
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Determine MIME type from thumbnail file extension.
    fn thumbnail_mimetype(path: &Path) -> &'static str {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "png" => "image/png",
            "webp" => "image/webp",
            _ => "image/jpeg",
        }
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
        Self::spawn_blocking("convert_video", move || {
            Self::convert_video_sync(&input, &output, &opts)
        })
        .await
    }

    /// Convert video synchronously (dispatches to remux or transcode).
    fn convert_video_sync(input: &Path, output: &Path, opts: &VideoConvertOptions) -> Result<()> {
        if opts.remux_only {
            // Determine if output is MP4/MOV for faststart
            let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("");
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
        let video_ist_time_base = ictx
            .stream(video_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?
            .time_base();
        let video_ist_frame_rate = ictx
            .stream(video_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?
            .avg_frame_rate();
        let audio_ist_time_base = audio_ist_index
            .map(|i| {
                ictx.stream(i).map(|s| s.time_base()).ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("audio input stream {i} not found"))
                })
            })
            .transpose()?;

        // Create video decoder
        let video_ist = ictx.stream(video_ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "video input stream {video_ist_index} not found"
            ))
        })?;
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
        let video_enc_codec = ffmpeg_the_third::encoder::find_by_name(video_codec_name)
            .ok_or_else(|| PostProcessError::UnsupportedCodec {
                codec: video_codec_name.to_string(),
                operation: "video conversion".into(),
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
            // SAFETY: video_encoder is a valid pre-open encoder context.
            Self::set_global_header_flag(unsafe { video_encoder.as_mut_ptr() });
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

        let mut video_encoder = video_encoder
            .open_as_with(video_enc_codec, enc_opts)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to open video encoder: {e}"),
            })?;

        // Copy encoder parameters back to output stream
        // SAFETY: video_encoder is a valid opened encoder context.
        Self::copy_encoder_params_to_stream(&mut octx, video_ost_index, unsafe {
            video_encoder.as_ptr()
        });

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
                    ost.set_parameters(
                        ictx.stream(audio_idx)
                            .ok_or_else(|| {
                                PostProcessError::ffmpeg_failed(format!(
                                    "audio input stream {audio_idx} not found"
                                ))
                            })?
                            .parameters(),
                    );
                    audio_ost_idx = ost.index();
                    Self::clear_codec_tag(ost.parameters().as_ptr());
                }
                Some(audio_ost_idx)
            } else {
                None
            }
        } else {
            None
        };

        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Build video filter graph for pixel format conversion
        let mut filter_graph =
            Self::build_video_filter(&video_decoder, &video_encoder, video_ist_time_base)?;

        // Process packets: video → decode/filter/encode, audio → copy
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
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
                    let ost_time_base = octx
                        .stream(audio_ost_idx)
                        .ok_or_else(|| {
                            PostProcessError::ffmpeg_failed(format!(
                                "audio output stream {audio_ost_idx} not found"
                            ))
                        })?
                        .time_base();
                    let audio_tb = audio_ist_time_base.ok_or_else(|| {
                        PostProcessError::ffmpeg_failed("audio input time base not available")
                    })?;
                    packet.rescale_ts(audio_tb, ost_time_base);
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
        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("video filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_video_filter_to_encoder(
            &mut filter_graph,
            &mut video_encoder,
            &mut octx,
            video_ost_index,
        )?;

        // Flush video encoder
        video_encoder.send_eof()?;
        Self::drain_video_encoder_packets(&mut video_encoder, &mut octx, video_ost_index)?;

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
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

        let buffer = ffmpeg_the_third::filter::find("buffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("buffer filter not found"))?;
        let buffersink = ffmpeg_the_third::filter::find("buffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("buffersink filter not found"))?;

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

        graph
            .add(&buffer, "in", &args)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffer filter: {e}"),
            })?;
        graph
            .add(&buffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffersink filter: {e}"),
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
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("video filter node 'in' not found"))?
                .source()
                .add(&frame)?;
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
        loop {
            let mut out_node = filter.get("out").ok_or_else(|| {
                PostProcessError::ffmpeg_failed("video filter node 'out' not found")
            })?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
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

    // -----------------------------------------------------------------------
    // FFI helper functions
    //
    // These encapsulate all `unsafe` FFI operations that lack safe wrappers
    // in `ffmpeg-the-third`, providing safe call-site signatures. The `unsafe`
    // blocks are limited to these well-documented helpers.
    // -----------------------------------------------------------------------

    /// Reset the codec tag to 0 for container compatibility.
    ///
    /// When remuxing between containers, the source codec tag may not be valid
    /// in the target container. Setting it to 0 lets FFmpeg auto-select.
    fn clear_codec_tag(params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters) {
        // SAFETY: `params_ptr` points to a valid AVCodecParameters allocated by FFmpeg.
        // Setting codec_tag to 0 is always valid — it tells FFmpeg to auto-select.
        unsafe {
            (*(params_ptr as *mut ffmpeg_the_third::ffi::AVCodecParameters)).codec_tag = 0;
        }
    }

    /// Copy encoder parameters back to an output stream.
    ///
    /// After opening an encoder, its parameters (codec, dimensions, sample rate,
    /// etc.) must be copied to the corresponding output stream before writing
    /// the header.
    fn copy_encoder_params_to_stream(
        octx: &mut ffmpeg_the_third::format::context::Output,
        stream_index: usize,
        encoder_ptr: *const ffmpeg_the_third::ffi::AVCodecContext,
    ) {
        // SAFETY: `octx` owns the output context with a valid stream array.
        // `stream_index` was obtained from a stream added to this context.
        // `encoder_ptr` points to a valid, opened encoder context.
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(stream_index);
            ffmpeg_the_third::ffi::avcodec_parameters_from_context(
                (*stream_ptr).codecpar,
                encoder_ptr,
            );
        }
    }

    /// Set the default channel layout for the given number of channels.
    fn set_default_channel_layout(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        channels: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // `av_channel_layout_default` populates the ch_layout field in-place.
        unsafe {
            ffmpeg_the_third::ffi::av_channel_layout_default(
                &mut (*encoder_ptr).ch_layout,
                channels,
            );
        }
    }

    /// Enable VBR (variable bitrate) quality mode on an encoder.
    fn set_vbr_quality(encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext, quality: i32) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // Setting QSCALE flag + global_quality is the standard way to enable VBR.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_QSCALE as i32;
            (*encoder_ptr).global_quality = quality * ffmpeg_the_third::ffi::FF_QP2LAMBDA;
        }
    }

    /// Set the global header flag on an encoder.
    ///
    /// Required when the output format needs codec parameters in the container
    /// header rather than in each packet (e.g., MP4, MKV).
    fn set_global_header_flag(encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // This flag is required by certain container formats.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    /// Configure a stream as an MKV attachment (for embedded thumbnails).
    ///
    /// Sets the codec type to `ATTACHMENT` and clears the codec tag.
    fn set_stream_as_attachment(params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters) {
        // SAFETY: `params_ptr` points to a valid AVCodecParameters for an output stream.
        // Setting codec_type to ATTACHMENT and clearing codec_tag is the standard
        // way to embed attachments in Matroska containers.
        unsafe {
            let codecpar = params_ptr as *mut ffmpeg_the_third::ffi::AVCodecParameters;
            (*codecpar).codec_type = ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_ATTACHMENT;
            (*codecpar).codec_tag = 0;
        }
    }

    /// Configure a stream with `ATTACHED_PIC` disposition (for cover art).
    ///
    /// Sets the stream disposition and clears the codec tag. Used for MP4,
    /// FLAC, OGG, and other containers that embed cover art as a video stream
    /// with special disposition.
    fn set_attached_pic_disposition(stream_ptr: *mut ffmpeg_the_third::ffi::AVStream) {
        // SAFETY: `stream_ptr` is a valid output stream pointer from a live
        // output context. Setting disposition and clearing codec_tag configures
        // the stream as cover art.
        unsafe {
            (*stream_ptr).disposition = ffmpeg_the_third::ffi::AV_DISPOSITION_ATTACHED_PIC;
            (*((*stream_ptr).codecpar)).codec_tag = 0;
        }
    }
}

impl MediaInfo {
    /// Get a resolution string (e.g., "1920x1080").
    #[must_use]
    pub fn resolution_string(&self) -> Option<String> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        }
    }

    /// Get the video stream info.
    #[must_use]
    pub fn video_stream(&self) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.codec_type == "video")
    }

    /// Get the audio stream info.
    #[must_use]
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
    fn test_media_info_resolution() {
        let mut info = MediaInfo::default();
        assert!(info.resolution_string().is_none());

        info.width = Some(1920);
        info.height = Some(1080);
        assert_eq!(info.resolution_string(), Some("1920x1080".to_string()));
    }
}
