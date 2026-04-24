//! Data models for the format listing IPC commands.
//!
//! Contains the request and response types serialised across the
//! Tauri IPC boundary for format selection.

use serde::{Deserialize, Serialize};

/// Frontend-facing format information.
///
/// A simplified projection of [`rdlp_types::Format`] that exposes
/// only the fields the UI needs, without internal download URLs.
#[derive(Debug, Serialize)]
pub struct FormatInfo {
    /// Unique format identifier (e.g. "137", "hls-720").
    pub format_id: String,
    /// File extension (e.g. "mp4", "webm").
    pub ext: String,
    /// Human-readable quality label (e.g. "720p", "1080p").
    pub format_note: Option<String>,
    /// Video width in pixels.
    pub width: Option<u32>,
    /// Video height in pixels.
    pub height: Option<u32>,
    /// Frames per second.
    pub fps: Option<f64>,
    /// Total bitrate in kbps (video + audio).
    pub tbr: Option<f64>,
    /// Video codec name (e.g. "h264", "vp9").
    pub vcodec: Option<String>,
    /// Audio codec name (e.g. "aac", "opus").
    pub acodec: Option<String>,
    /// File size in bytes (exact or approximate).
    pub filesize: Option<u64>,
    /// Video bitrate in kbps.
    pub vbr: Option<f64>,
    /// Audio bitrate in kbps.
    pub abr: Option<f64>,
    /// Audio sampling rate in Hz.
    pub asr: Option<u32>,
    /// Download protocol (e.g. "https", "m3u8_native").
    pub protocol: String,
    /// Whether this format contains a video stream.
    pub has_video: bool,
    /// Whether this format contains an audio stream.
    pub has_audio: bool,
    /// HLS audio-rendition group identifier (EXT-X-MEDIA `GROUP-ID` or the
    /// `AUDIO=` attribute referenced by an EXT-X-STREAM-INF). Lets the UI
    /// visually pair video-only and audio-only rows that share a group.
    /// `None` outside HLS.
    pub audio_group_id: Option<String>,
}

/// Subtitle availability for a single language.
#[derive(Debug, Serialize)]
pub struct SubtitleInfo {
    /// Language code (e.g. "en", "ja").
    pub lang: String,
    /// Subtitle format extensions (kept as strings because extractors
    /// may return unknown formats that do not map to `SubtitleFormat`).
    pub formats: Vec<String>,
}

/// Complete response for the `get_formats` command.
///
/// Contains all metadata the frontend needs to render the format
/// selection UI.
#[derive(Debug, Serialize)]
pub struct FormatListResponse {
    /// Video title.
    pub title: String,
    /// Available download formats.
    pub formats: Vec<FormatInfo>,
    /// Available subtitle languages and their formats.
    pub subtitles: Vec<SubtitleInfo>,
    /// URL of the best thumbnail image.
    pub thumbnail_url: Option<String>,
    /// Video duration in seconds.
    pub duration: Option<f64>,
    /// Whether this URL resolved to a playlist (multiple episodes).
    pub is_playlist: bool,
    /// Playlist metadata — `None` for single videos.
    pub playlist: Option<PlaylistInfo>,
}

/// A single episode entry in a playlist.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    /// 1-based position in the playlist.
    pub index: usize,
    /// Episode title.
    pub title: String,
    /// Full URL with ?ep= parameter for direct single-episode download.
    pub url: String,
    /// Thumbnail URL for this episode.
    pub thumbnail_url: Option<String>,
    /// Duration in seconds.
    pub duration: Option<f64>,
    /// Whether SUB (subtitled) audio is available.
    pub has_sub: bool,
    /// Whether DUB (dubbed) audio is available.
    pub has_dub: bool,
}

/// Playlist metadata returned when a URL resolves to multiple episodes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    /// Playlist/series title.
    pub title: String,
    /// Total number of episodes.
    pub count: usize,
    /// Episode entries with metadata.
    pub entries: Vec<PlaylistEntry>,
}

/// Format metadata sent from the frontend for expression validation.
///
/// Mirrors the fields from [`FormatInfo`] that the format selector
/// uses for filtering and ranking, allowing expressions like
/// `bv[height<=1080]+ba` to match correctly.
#[derive(Debug, Deserialize)]
pub struct FormatData {
    /// Unique format identifier.
    pub(crate) format_id: String,
    /// File extension (e.g. "mp4", "webm").
    pub(crate) ext: String,
    /// Video width in pixels.
    pub(crate) width: Option<u32>,
    /// Video height in pixels.
    pub(crate) height: Option<u32>,
    /// Frames per second.
    pub(crate) fps: Option<f64>,
    /// Total bitrate in kbps.
    pub(crate) tbr: Option<f64>,
    /// Video codec name.
    pub(crate) vcodec: Option<String>,
    /// Audio codec name.
    pub(crate) acodec: Option<String>,
    /// File size in bytes.
    pub(crate) filesize: Option<u64>,
    /// Video bitrate in kbps.
    pub(crate) vbr: Option<f64>,
    /// Audio bitrate in kbps.
    pub(crate) abr: Option<f64>,
    /// Audio sampling rate in Hz.
    pub(crate) asr: Option<u32>,
    /// Download protocol string (e.g. "https", "m3u8_native").
    pub(crate) protocol: String,
}
