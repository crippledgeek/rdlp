//! Media file probing via `FFmpeg` library bindings.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` probe values use C integer types (duration as `i64`,
//!   codec properties as `u32`/`u8`). Conversions to `f64`/`u32` are audited:
//!   durations and codec widths/heights are within safe ranges.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use rdlp_types::media_name::CodecName;
use serde::{Deserialize, Serialize};

use crate::error::{PostProcessError, Result};
use crate::ffmpeg::codec_registry::MediaKind;

use super::{FFmpegRunner, ensure_init};

/// Broad category of a probed stream, as `FFmpeg`'s `AVMediaType` classifies
/// it.
///
/// Replaces a bare `codec_type: String` holding one of `"video"` / `"audio"` /
/// `"subtitle"` / `"data"` / `"unknown"` — every comparison against that
/// string was a stringly-typed re-implementation of this same five-way
/// classification (`fixup.rs`'s issue detection, `probe.rs`'s own
/// `video_stream`/`audio_stream` lookups), and a typo in any one of them would
/// have compiled and silently never matched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    /// A video elementary stream.
    Video,
    /// An audio elementary stream.
    Audio,
    /// A subtitle stream.
    Subtitle,
    /// A data (non-media) stream.
    Data,
    /// A stream `FFmpeg` itself could not classify.
    #[default]
    Unknown,
}

impl StreamKind {
    /// Classifies an `FFmpeg` `media::Type`.
    const fn from_medium(medium: ffmpeg_the_third::media::Type) -> Self {
        match medium {
            ffmpeg_the_third::media::Type::Video => Self::Video,
            ffmpeg_the_third::media::Type::Audio => Self::Audio,
            ffmpeg_the_third::media::Type::Subtitle => Self::Subtitle,
            ffmpeg_the_third::media::Type::Data => Self::Data,
            _ => Self::Unknown,
        }
    }

    /// Projects to the audio/video routing kind
    /// `crate::ffmpeg::muxer_defaults::muxer_can_represent` and `Registry`
    /// ask about. `None` for
    /// [`Subtitle`](Self::Subtitle) / [`Data`](Self::Data) /
    /// [`Unknown`](Self::Unknown), which have no muxer-representability
    /// question to answer.
    #[must_use]
    pub const fn as_media_kind(self) -> Option<MediaKind> {
        match self {
            Self::Video => Some(MediaKind::Video),
            Self::Audio => Some(MediaKind::Audio),
            Self::Subtitle | Self::Data | Self::Unknown => None,
        }
    }
}

impl std::fmt::Display for StreamKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Subtitle => "subtitle",
            Self::Data => "data",
            Self::Unknown => "unknown",
        })
    }
}

/// Media file information from `FFprobe`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    /// File path
    pub path: PathBuf,
    /// Duration in seconds
    pub duration: Option<f64>,
    /// Container format (e.g., "mp4", "mkv")
    pub format: Option<String>,
    /// Video codec (e.g., "h264", "vp9")
    pub video_codec: Option<CodecName>,
    /// Audio codec (e.g., "aac", "mp3")
    pub audio_codec: Option<CodecName>,
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
    /// Broad category of this stream.
    pub codec_type: StreamKind,
    /// Codec name
    pub codec_name: Option<CodecName>,
    /// Stream-specific metadata
    pub metadata: HashMap<String, String>,
    /// Stream duration in seconds
    pub duration: Option<f64>,
    /// Sample aspect ratio numerator (video only)
    pub sar_num: Option<i32>,
    /// Sample aspect ratio denominator (video only)
    pub sar_den: Option<i32>,
    /// Number of frames (video only; 0 means unknown)
    pub nb_frames: Option<i64>,
}

impl FFmpegRunner {
    /// Probe a media file using the `FFmpeg` library and return its information.
    ///
    /// # Errors
    ///
    /// Returns [`PostProcessError::InputNotFound`] if the file does not exist,
    /// or a library error if `FFmpeg` fails to open or decode the container.
    pub async fn probe(&self, path: impl AsRef<Path>) -> Result<MediaInfo> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            return Err(PostProcessError::InputNotFound { path });
        }

        Self::spawn_blocking("probe", move || -> Result<MediaInfo> {
            Ok(Self::probe_sync(&path)?)
        })
        .await
    }

    /// Probe a media file synchronously using ffmpeg-the-third library.
    #[allow(clippy::too_many_lines)]
    fn probe_sync(path: &Path) -> anyhow::Result<MediaInfo> {
        ensure_init()?;

        let ictx = ffmpeg_the_third::format::input(path)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to probe input {}", path.display()))?;

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
        info.filesize = {
            // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
            #[allow(clippy::disallowed_methods)]
            let m = std::fs::metadata(path);
            m.ok().map(|m| m.len())
        };

        // Format-level metadata. `collect` into the fresh default map keeps
        // last-wins on duplicate keys exactly as repeated `insert` would.
        info.metadata = ictx
            .metadata()
            .iter()
            .map(|(key, value)| (key.to_lowercase(), value.to_string()))
            .collect();

        // Parse streams
        info.stream_count = ictx.streams().count();

        for stream in ictx.streams() {
            let params = stream.parameters();
            let medium = params.medium();

            // `avcodec_get_name`'s name is documented static, but it is only
            // read once per stream here (not memoised in a static table like
            // the codec registries), so the simpler allocating `new` is used
            // rather than `new_static`. A name this floor would reject (a
            // future libavcodec descriptor with a NUL/control character) is
            // not something this probe should hard-fail on, so an
            // unrepresentable name degrades to `None` rather than aborting
            // the whole probe.
            let codec_name = CodecName::new(params.id().name()).ok();
            let codec_type = StreamKind::from_medium(medium);

            // Stream-level metadata, built directly into the struct below.
            let stream_metadata = stream
                .metadata()
                .iter()
                .map(|(key, value)| (key.to_lowercase(), value.to_string()))
                .collect();

            let mut stream_info = StreamInfo {
                index: stream.index(),
                codec_type,
                codec_name,
                metadata: stream_metadata,
                duration: None,
                sar_num: None,
                sar_den: None,
                nb_frames: None,
            };

            // Per-stream duration
            let stream_duration_ts = stream.duration();
            if stream_duration_ts > 0 {
                let tb = stream.time_base();
                let dur = stream_duration_ts as f64 * f64::from(tb.numerator())
                    / f64::from(tb.denominator());
                stream_info.duration = Some(dur);
            }

            match medium {
                ffmpeg_the_third::media::Type::Video => {
                    info.has_video = true;
                    if info.video_codec.is_none() {
                        info.video_codec.clone_from(&stream_info.codec_name);
                    }

                    if let Ok(codec_ctx) =
                        ffmpeg_the_third::codec::context::Context::from_parameters(params)
                        && let Ok(video) = codec_ctx.decoder().video()
                    {
                        info.width = Some(video.width());
                        info.height = Some(video.height());
                    }

                    // Frame rate from avg_frame_rate
                    let rate = stream.avg_frame_rate();
                    if rate.denominator() > 0 {
                        let fps = f64::from(rate.numerator()) / f64::from(rate.denominator());
                        if fps > 0.0 && fps < 1000.0 {
                            info.fps = Some(fps);
                        }
                    }

                    // Video bitrate from stream metadata
                    if let Some(br_str) = stream_info.metadata.get("bps")
                        && let Ok(bps) = br_str.parse::<u64>()
                    {
                        info.video_bitrate = Some((bps / 1000) as u32);
                    }

                    // SAR (sample aspect ratio) via raw FFI
                    unsafe {
                        let st_ptr = stream.as_ptr();
                        let codecpar = (*st_ptr).codecpar;
                        let sar = (*codecpar).sample_aspect_ratio;
                        if sar.num != 0 && sar.den != 0 {
                            stream_info.sar_num = Some(sar.num);
                            stream_info.sar_den = Some(sar.den);
                        }
                    }

                    // Number of frames via raw FFI
                    unsafe {
                        let st_ptr = stream.as_ptr();
                        let nb = (*st_ptr).nb_frames;
                        if nb > 0 {
                            stream_info.nb_frames = Some(nb);
                        }
                    }
                }
                ffmpeg_the_third::media::Type::Audio => {
                    info.has_audio = true;
                    if info.audio_codec.is_none() {
                        info.audio_codec.clone_from(&stream_info.codec_name);
                    }

                    if let Ok(codec_ctx) =
                        ffmpeg_the_third::codec::context::Context::from_parameters(params)
                        && let Ok(audio) = codec_ctx.decoder().audio()
                    {
                        info.sample_rate = Some(audio.rate());
                        info.channels = Some(audio.ch_layout().channels() as u8);
                    }

                    // Audio bitrate from stream metadata
                    if info.audio_bitrate.is_none()
                        && let Some(br_str) = stream_info.metadata.get("bps")
                        && let Ok(bps) = br_str.parse::<u64>()
                    {
                        info.audio_bitrate = Some((bps / 1000) as u32);
                    }
                }
                _ => {}
            }

            info.streams.push(stream_info);
        }

        Ok(info)
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
        self.streams
            .iter()
            .find(|s| s.codec_type == StreamKind::Video)
    }

    /// Get the audio stream info.
    #[must_use]
    pub fn audio_stream(&self) -> Option<&StreamInfo> {
        self.streams
            .iter()
            .find(|s| s.codec_type == StreamKind::Audio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_info_default_has_none_fields() {
        let si = StreamInfo::default();
        assert_eq!(si.duration, None);
        assert_eq!(si.sar_num, None);
        assert_eq!(si.sar_den, None);
        assert_eq!(si.nb_frames, None);
    }
}
