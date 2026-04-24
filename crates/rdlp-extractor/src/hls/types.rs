//! HLS data types and utility functions
//!
//! Contains the public types used across the HLS module: `HlsStreamFlags`,
//! `HlsInfo`, `HlsVariantInfo`, and the internal `MediaPlaylistInfo`.

/// Stream-level flags aggregated from HLS format detection
///
/// These flags represent properties of the entire stream, not individual formats.
/// They are aggregated during `detect_format_sizes()` and can be used to set
/// `InfoDict.is_live` or warn users about encrypted content.
#[derive(Debug, Clone, Default)]
pub struct HlsStreamFlags {
    /// True if any HLS format is a live stream (no EXT-X-ENDLIST tag)
    pub is_live: bool,
    /// True if any HLS format uses encryption (EXT-X-KEY)
    pub has_any_drm: bool,
}

/// Information about an HLS stream
#[derive(Debug, Clone)]
pub struct HlsInfo {
    /// Total size in bytes (sum of all segment sizes) - None if not detected
    pub total_size: Option<u64>,
    /// Number of segments in the playlist
    pub segment_count: usize,
    /// Total duration in seconds (sum of segment durations)
    pub total_duration: Option<f64>,
    /// Video resolution (width, height) from master playlist variant
    pub resolution: Option<(u64, u64)>,
    /// Parsed video codec name (e.g., "h264", "hevc", "vp9")
    pub video_codec: Option<String>,
    /// Parsed audio codec name (e.g., "aac", "ac3", "opus")
    pub audio_codec: Option<String>,
    /// Frame rate from master playlist variant
    pub frame_rate: Option<f64>,
    /// Peak bandwidth in bits per second from variant
    pub bandwidth: Option<u64>,
    /// Average bandwidth in bits per second from variant
    pub average_bandwidth: Option<u64>,
    /// Whether the stream is live (no EXT-X-ENDLIST tag)
    pub is_live: bool,
    /// Whether any segment uses encryption (EXT-X-KEY)
    pub has_encryption: bool,
    /// Detected segment container format (e.g., "ts", "mp4", "m4s")
    pub segment_container: Option<String>,
}

/// Per-variant information from an HLS master playlist.
///
/// Each variant represents a specific quality level (e.g., 720p, 1080p)
/// with its own media playlist URL and metadata.
///
/// Entries can be produced from two sources:
/// - `EXT-X-STREAM-INF` — a combined or video-only variant (default)
/// - `EXT-X-MEDIA TYPE=AUDIO` — an audio-only rendition group member;
///   these are flagged via `is_audio_only = true` and have no resolution
///   or video codec.
#[derive(Debug, Clone)]
pub struct HlsVariantInfo {
    /// Resolved absolute URL to this variant's media playlist
    pub media_playlist_url: String,
    /// Video resolution (width, height)
    pub resolution: Option<(u64, u64)>,
    /// Parsed video codec name (e.g., "h264", "av1")
    pub video_codec: Option<String>,
    /// Parsed audio codec name (e.g., "aac", "opus")
    pub audio_codec: Option<String>,
    /// Frame rate
    pub frame_rate: Option<f64>,
    /// Peak bandwidth in bits per second
    pub bandwidth: u64,
    /// Average bandwidth in bits per second
    pub average_bandwidth: Option<u64>,
    /// True if this entry was derived from an `EXT-X-MEDIA TYPE=AUDIO`
    /// rendition group rather than an `EXT-X-STREAM-INF`. Audio-only
    /// entries carry `video_codec=None` and no `resolution`.
    pub is_audio_only: bool,
    /// BCP-47 language tag from `LANGUAGE` attribute (EXT-X-MEDIA only)
    pub language: Option<String>,
    /// Audio-rendition `GROUP-ID` (EXT-X-MEDIA only). Matches the
    /// `AUDIO=` attribute on paired `EXT-X-STREAM-INF` variants.
    pub audio_group_id: Option<String>,
    /// Rendition `NAME` attribute, e.g. "English", "Stereo" (EXT-X-MEDIA only)
    pub rendition_name: Option<String>,
    // Shared fields (from one media playlist, applied to all variants):
    /// Number of segments
    pub segment_count: usize,
    /// Total duration in seconds
    pub total_duration: Option<f64>,
    /// Whether the stream is live
    pub is_live: bool,
    /// Whether segments use encryption
    pub has_encryption: bool,
    /// Detected segment container format
    pub segment_container: Option<String>,
}

/// Media playlist metadata extracted without additional HTTP requests
pub(super) struct MediaPlaylistInfo {
    pub segment_count: usize,
    pub total_duration: f64,
    pub is_live: bool,
    pub has_encryption: bool,
    pub segment_container: Option<String>,
}

/// Detect container format from segment URL extension
pub(super) fn detect_segment_container(segment_uri: &str) -> Option<String> {
    let path = segment_uri.split('?').next()?;
    let ext = path[path.rfind('.')? + 1..].to_lowercase();
    if ext.is_empty() { None } else { Some(ext) }
}
