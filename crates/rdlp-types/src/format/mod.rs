//! Format types for video/audio streams

mod codec;
mod display;
pub mod selector;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use crate::protocol::DownloadProtocol;

pub use codec::Codec;
pub use selector::{FormatSelectError, FormatSelector, FormatSorter, format_select};

/// Video/audio format information
///
/// Represents a single downloadable stream (video, audio, or combined).
/// Sites often provide multiple formats with different qualities, codecs, and containers.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    /// Unique format identifier
    pub format_id: String,

    /// Download URL for this format
    pub url: String,

    // === Quality indicators ===
    /// Quality rating (higher is better, site-specific scale)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<i32>,

    /// Video width in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,

    /// Video height in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,

    /// Frames per second
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,

    /// Total bitrate (video + audio) in kbps
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tbr: Option<f64>,

    /// Video bitrate in kbps
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vbr: Option<f64>,

    /// Audio bitrate in kbps
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abr: Option<f64>,

    /// Audio sampling rate in Hz
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr: Option<u32>,

    // === Codec information ===
    /// Video codec (e.g., `Codec::Present("h264")`, `Codec::Present("vp9")`,
    /// `Codec::Absent` for audio-only streams). The `"none"` sentinel
    /// (and the empty string) is normalised to [`Codec::Absent`] at every
    /// entry point so call-sites need only check `is_absent` / `is_present`.
    #[serde(default, skip_serializing_if = "Codec::is_absent")]
    pub vcodec: Codec,

    /// Audio codec (e.g., `Codec::Present("aac")`, `Codec::Present("opus")`,
    /// `Codec::Absent` for video-only streams).
    #[serde(default, skip_serializing_if = "Codec::is_absent")]
    pub acodec: Codec,

    /// File extension (e.g., "mp4", "webm", "m4a")
    pub ext: String,

    /// Human-readable format note
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_note: Option<String>,

    // === Protocol ===
    /// Download protocol
    pub protocol: DownloadProtocol,

    /// Container format (e.g., "mp4", "webm", "`mp4_dash`")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,

    // === File size ===
    /// Exact file size in bytes (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,

    /// Approximate file size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize_approx: Option<u64>,

    // === Fragment information ===
    /// List of fragments for segmented downloads (HLS, DASH, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragments: Option<Vec<Fragment>>,

    /// Fragment base URL (for relative fragment URLs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fragment_base_url: Option<String>,

    // === HTTP headers ===
    /// HTTP headers required for download
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_headers: Option<HashMap<String, String>>,

    // === Additional metadata ===
    /// Format language (for audio tracks with multiple languages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// HLS audio-rendition group identifier.
    ///
    /// - On a video-only variant derived from an `EXT-X-STREAM-INF` that
    ///   carries an `AUDIO="group-id"` attribute, this is the **referenced**
    ///   group — the audio rendition the variant expects to be paired with.
    /// - On an audio-only variant derived from an `EXT-X-MEDIA TYPE=AUDIO`
    ///   rendition, this is the **owned** group identifier.
    /// - `None` for any other source (muxed HLS variants, direct progressive
    ///   downloads, extractors that don't surface groups).
    ///
    /// Lets UIs visually pair video-only and audio-only rows when a user
    /// hand-picks without the preset — matching `audio_group_id` values
    /// indicate a compatible pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_group_id: Option<String>,

    /// Dynamic range (e.g., "SDR", "HDR10", "HDR10+")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_range: Option<String>,

    /// Cached format description (lazy initialization)
    #[serde(skip)]
    cached_description: OnceLock<String>,

    /// Whether format has DRM protection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_drm: Option<bool>,

    /// Total duration in seconds (for HLS: sum of segment durations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    /// Fallback download URLs (alternative CDNs), tried in order if primary fails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_urls: Option<Vec<String>>,
}

impl fmt::Debug for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Format");

        // Always show required fields
        d.field("format_id", &self.format_id);
        d.field("url", &self.url);
        d.field("ext", &self.ext);
        d.field("protocol", &self.protocol.as_str());

        // Only show optional fields that have values
        if let Some(v) = &self.quality {
            d.field("quality", v);
        }
        if let Some(v) = &self.width {
            d.field("width", v);
        }
        if let Some(v) = &self.height {
            d.field("height", v);
        }
        if let Some(v) = &self.fps {
            d.field("fps", v);
        }
        if let Some(v) = &self.tbr {
            d.field("tbr", v);
        }
        if let Some(v) = &self.vbr {
            d.field("vbr", v);
        }
        if let Some(v) = &self.abr {
            d.field("abr", v);
        }
        if let Some(v) = &self.asr {
            d.field("asr", v);
        }
        if let Some(v) = self.vcodec.as_str() {
            d.field("vcodec", &v);
        }
        if let Some(v) = self.acodec.as_str() {
            d.field("acodec", &v);
        }
        if let Some(v) = &self.format_note {
            d.field("format_note", v);
        }
        if let Some(v) = &self.container {
            d.field("container", v);
        }
        if let Some(v) = &self.filesize {
            d.field("filesize", v);
        }
        if let Some(v) = &self.filesize_approx {
            d.field("filesize_approx", v);
        }
        if let Some(v) = &self.fragments {
            d.field("fragments", &format!("[{} fragments]", v.len()));
        }
        if let Some(v) = &self.fragment_base_url {
            d.field("fragment_base_url", v);
        }
        if let Some(v) = &self.http_headers {
            d.field("http_headers", v);
        }
        if let Some(v) = &self.language {
            d.field("language", v);
        }
        if let Some(v) = &self.audio_group_id {
            d.field("audio_group_id", v);
        }
        if let Some(v) = &self.dynamic_range {
            d.field("dynamic_range", v);
        }
        if let Some(v) = &self.has_drm {
            d.field("has_drm", v);
        }
        if let Some(v) = &self.duration {
            d.field("duration", v);
        }
        if let Some(v) = &self.fallback_urls {
            d.field("fallback_urls", &format!("[{} URLs]", v.len()));
        }

        // cached_description is an internal lazy cache; intentionally excluded.
        d.finish_non_exhaustive()
    }
}

impl Format {
    /// Create a new format with required fields
    #[must_use]
    pub fn new(
        format_id: impl Into<String>,
        url: impl Into<String>,
        ext: impl Into<String>,
        protocol: DownloadProtocol,
    ) -> Self {
        Self {
            format_id: format_id.into(),
            url: url.into(),
            quality: None,
            width: None,
            height: None,
            fps: None,
            tbr: None,
            vbr: None,
            abr: None,
            asr: None,
            vcodec: Codec::Absent,
            acodec: Codec::Absent,
            ext: ext.into(),
            format_note: None,
            protocol,
            container: None,
            filesize: None,
            filesize_approx: None,
            fragments: None,
            fragment_base_url: None,
            http_headers: None,
            language: None,
            audio_group_id: None,
            dynamic_range: None,
            cached_description: OnceLock::new(),
            has_drm: None,
            duration: None,
            fallback_urls: None,
        }
    }

    /// Check if this format has video
    pub const fn has_video(&self) -> bool {
        self.vcodec.is_present()
    }

    /// Check if this format has audio
    pub const fn has_audio(&self) -> bool {
        self.acodec.is_present()
    }

    /// Get resolution as string (e.g., "1920x1080")
    pub fn resolution_string(&self) -> Option<String> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        }
    }

    /// Get file size (exact or approximate)
    pub fn get_filesize(&self) -> Option<u64> {
        self.filesize.or(self.filesize_approx)
    }

    /// Format file size as human-readable string
    /// Returns exact size if known, approximate size with ~ prefix, or "Unknown"
    pub fn filesize_string(&self) -> String {
        self.filesize.map_or_else(
            || {
                self.filesize_approx.map_or_else(
                    || "Unknown".to_string(),
                    |size| format!("~{}", format_bytes(size)),
                )
            },
            format_bytes,
        )
    }

    /// Check if this is a DASH format
    pub fn is_dash(&self) -> bool {
        self.protocol.is_dash() || self.container.as_ref().is_some_and(|c| c.contains("dash"))
    }

    /// Check if this is an HLS format
    ///
    /// Checks all three signals: protocol enum, URL pattern, and file extension.
    /// Some extractors set `protocol` as `Https` even for m3u8 URLs, so the URL
    /// and extension checks act as fallbacks.
    pub fn is_hls(&self) -> bool {
        self.protocol.is_hls() || self.url.contains(".m3u8") || self.ext == "hls"
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Compact format for general use
        write!(f, "{}", self.format_id)?;

        if let Some(note) = &self.format_note {
            write!(f, " ({note})")?;
        }

        if let Some(res) = self.resolution_string() {
            write!(f, " {res}")?;
        }

        // Show exact size or approximate size with ~ prefix
        // u64 as f64: MB display tolerates precision loss on very large sizes
        #[allow(clippy::cast_precision_loss)]
        if let Some(size) = self.filesize {
            let mb = size as f64 / (1024.0 * 1024.0);
            write!(f, " {mb:.1}MB")?;
        } else if let Some(size) = self.filesize_approx {
            let mb = size as f64 / (1024.0 * 1024.0);
            write!(f, " ~{mb:.0}MB")?;
        }

        Ok(())
    }
}

/// Pre-resolved fragment (HLS segment / DASH segment) with optional
/// byte-range subrange and per-fragment init reference.
///
/// Byte-range tuple convention: `(start, end_exclusive)`, matching yt-dlp's
/// `byte_range` object `{start, end}`. To convert to an `FFmpeg` `url_offset`/`size`
/// pair: `url_offset = start`, `size = end_exclusive - start`. The HTTP `Range`
/// header uses an inclusive end, so subtract 1 when emitting it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fragment {
    /// Absolute fragment URL (HLS segment URI or DASH segment URI).
    pub url: String,

    /// Subrange of `url` to fetch via HTTP `Range:` header.
    /// `(start, end_exclusive)`. `None` = whole resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_range: Option<(u64, u64)>,

    /// `EXT-X-MAP` init segment URI for THIS fragment (HLS) or
    /// `<Initialization>` URI (DASH). Per-fragment so multi-init streams
    /// (RFC 8216 §4.4.2.5) work correctly: downloader refetches only
    /// when this URI changes between consecutive fragments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_url: Option<String>,

    /// Subrange of `init_url`. Same `(start, end_exclusive)` convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_byte_range: Option<(u64, u64)>,

    /// Segment duration in seconds (from `#EXTINF` or DASH `@duration`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    /// Optional pre-known segment size in bytes (rarely populated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
}

/// Format bytes as a human-readable string using base-1024 (binary)
/// math with the colloquial "KB"/"MB"/"GB"/"TB" labels — matches the
/// convention every other downloader (yt-dlp, curl, browsers) uses for
/// terminal output, and matches the `Format::size_text` helper that
/// renders the same value in the CLI table.
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    // u64 as f64: precision loss acceptable for display; bytes ≤ u64::MAX ≈ 1.8×10¹⁹
    #[allow(clippy::cast_precision_loss)]
    let bytes_f = bytes as f64;
    // log2 / 10 == log_1024. The floor is finite and non-negative for bytes > 0.
    // cast to usize: exponent is at most 4 (≤ UNITS.len() - 1), never wraps.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let exponent = ((bytes_f.log2() / 10.0).floor() as usize).min(UNITS.len() - 1);
    // exponent ≤ 4, fits in i32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let value = bytes_f / 1024_f64.powi(exponent as i32);
    #[allow(clippy::indexing_slicing)] // exponent is bounded by UNITS.len() - 1 above
    let unit = UNITS[exponent];

    format!("{value:.1} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_has_video() {
        let mut format = Format::new(
            "137",
            "https://example.com/video",
            "mp4",
            DownloadProtocol::Https,
        );
        format.vcodec = Codec::Present("h264".to_string());
        format.acodec = Codec::Absent;

        assert!(format.has_video());
        assert!(!format.has_audio());
    }

    #[test]
    fn test_format_has_audio() {
        let mut format = Format::new(
            "140",
            "https://example.com/audio",
            "m4a",
            DownloadProtocol::Https,
        );
        format.vcodec = Codec::Absent;
        format.acodec = Codec::Present("aac".to_string());

        assert!(!format.has_video());
        assert!(format.has_audio());
    }

    #[test]
    fn test_is_hls_by_protocol() {
        let format = Format::new(
            "hls",
            "https://cdn.example.com/video.ts",
            "mp4",
            DownloadProtocol::M3u8,
        );
        assert!(format.is_hls());

        let format = Format::new(
            "hls",
            "https://cdn.example.com/video.ts",
            "mp4",
            DownloadProtocol::M3u8Native,
        );
        assert!(format.is_hls());
    }

    #[test]
    fn test_is_hls_by_url() {
        // Some extractors set protocol as Https even for m3u8 URLs
        let format = Format::new(
            "hls",
            "https://cdn.example.com/master.m3u8",
            "mp4",
            DownloadProtocol::Https,
        );
        assert!(format.is_hls());

        let format = Format::new(
            "hls",
            "https://cdn.example.com/index.m3u8?token=abc",
            "mp4",
            DownloadProtocol::Https,
        );
        assert!(format.is_hls());
    }

    #[test]
    fn test_is_hls_by_ext() {
        let format = Format::new(
            "hls",
            "https://cdn.example.com/video",
            "hls",
            DownloadProtocol::Https,
        );
        assert!(format.is_hls());
    }

    #[test]
    fn test_is_not_hls() {
        let format = Format::new(
            "mp4",
            "https://cdn.example.com/video.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        assert!(!format.is_hls());
    }
}
