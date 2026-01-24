use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

/// Video/audio format information
///
/// Represents a single downloadable stream (video, audio, or combined).
/// Sites often provide multiple formats with different qualities, codecs, and containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Video codec (e.g., "h264", "vp9", "av1")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcodec: Option<String>,

    /// Audio codec (e.g., "aac", "opus", "mp3")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acodec: Option<String>,

    /// File extension (e.g., "mp4", "webm", "m4a")
    pub ext: String,

    /// Human-readable format note
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_note: Option<String>,

    // === Protocol ===
    /// Download protocol (e.g., "https", "http", "m3u8", "m3u8_native", "http_dash_segments")
    pub protocol: String,

    /// Container format (e.g., "mp4", "webm", "mp4_dash")
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

    /// Dynamic range (e.g., "SDR", "HDR10", "HDR10+")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_range: Option<String>,

    /// Cached format description (lazy initialization)
    #[serde(skip)]
    cached_description: OnceLock<String>,

    /// Whether format has DRM protection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_drm: Option<bool>,
}

impl Format {
    /// Create a new format with required fields
    pub fn new(format_id: String, url: String, ext: String, protocol: String) -> Self {
        Self {
            format_id,
            url,
            quality: None,
            width: None,
            height: None,
            fps: None,
            tbr: None,
            vbr: None,
            abr: None,
            asr: None,
            vcodec: None,
            acodec: None,
            ext,
            format_note: None,
            protocol,
            container: None,
            filesize: None,
            filesize_approx: None,
            fragments: None,
            fragment_base_url: None,
            http_headers: None,
            language: None,
            dynamic_range: None,
            cached_description: OnceLock::new(),
            has_drm: None,
        }
    }

    /// Check if this format has video
    pub fn has_video(&self) -> bool {
        self.vcodec.as_ref().is_some_and(|c| c != "none")
    }

    /// Check if this format has audio
    pub fn has_audio(&self) -> bool {
        self.acodec.as_ref().is_some_and(|c| c != "none")
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
        if let Some(size) = self.filesize {
            format_bytes(size)
        } else if let Some(size) = self.filesize_approx {
            format!("~{}", format_bytes(size))
        } else {
            "Unknown".to_string()
        }
    }

    /// Check if this is a DASH format
    pub fn is_dash(&self) -> bool {
        self.protocol.contains("dash") || self.container.as_ref().is_some_and(|c| c.contains("dash"))
    }

    /// Check if this is an HLS format
    pub fn is_hls(&self) -> bool {
        self.protocol.contains("m3u8")
    }

    /// Get a human-readable format description
    ///
    /// This method caches the description after first computation.
    /// Subsequent calls return a reference to the cached value.
    pub fn description(&self) -> &str {
        self.cached_description.get_or_init(|| {
            let mut parts = Vec::new();

            if let Some(note) = &self.format_note {
                parts.push(note.clone());
            }

            if let Some(res) = self.resolution_string() {
                parts.push(res);
            }

            if let Some(fps) = self.fps {
                parts.push(format!("{fps}fps"));
            }

            if let Some(vcodec) = &self.vcodec {
                if vcodec != "none" {
                    parts.push(format!("vcodec:{vcodec}"));
                }
            }

            if let Some(acodec) = &self.acodec {
                if acodec != "none" {
                    parts.push(format!("acodec:{acodec}"));
                }
            }

            parts.push(self.ext.clone());

            parts.join(" ")
        })
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

impl Format {
    /// Format as table row for interactive selection UI
    ///
    /// Returns a formatted string suitable for display in selection menus:
    /// `"720p         | 1280x720   | 245.3 MB     | MP4    | h264/aac"`
    pub fn table_row(&self) -> String {
        let quality = self.format_note.as_deref().unwrap_or("unknown");

        let resolution = self.resolution_string().unwrap_or_else(|| "N/A".to_string());

        // Check if this is an HLS format (also check URL for .m3u8)
        let is_hls = self.is_hls() || self.url.contains(".m3u8");

        // For HLS: show segment count if available, otherwise "HLS stream"
        // For non-HLS: show exact size if available, otherwise approximate size with ~ prefix
        let size = if is_hls {
            if let Some(ref fragments) = self.fragments {
                format!("{} segments", fragments.len())
            } else {
                "HLS stream".to_string()
            }
        } else if let Some(filesize) = self.filesize {
            format!("{:.1} MB", filesize as f64 / (1024.0 * 1024.0))
        } else if let Some(filesize_approx) = self.filesize_approx {
            format!("~{:.0} MB", filesize_approx as f64 / (1024.0 * 1024.0))
        } else {
            "Unknown".to_string()
        };

        // For HLS, show "HLS" as the format type instead of the extension
        let format_type = if is_hls {
            "HLS".to_string()
        } else {
            self.ext.to_uppercase()
        };

        let codecs = match (&self.vcodec, &self.acodec) {
            (Some(v), Some(a)) => format!("{v}/{a}"),
            (Some(v), None) => format!("{v} (video only)"),
            (None, Some(a)) => format!("{a} (audio only)"),
            (None, None) => "Unknown".to_string(),
        };

        format!("{quality:<12} | {resolution:<10} | {size:<12} | {format_type:<6} | {codecs}")
    }
}

/// Fragment of a segmented download
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fragment {
    /// Fragment URL (absolute or relative to fragment_base_url)
    pub url: String,

    /// Fragment duration in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    /// Fragment size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
}

/// Format selector for parsing and evaluating format selection expressions
///
/// Supports yt-dlp format selection syntax:
/// - "best" / "worst" - best/worst quality
/// - "bestvideo" / "bestaudio" / "worstvideo" / "worstaudio"
/// - "bestvideo+bestaudio" - merge best video and audio
/// - "bestvideo[height<=1080]" - filters
/// - "bestvideo*" - prefer but fallback if not available
///
/// TODO: Full DSL parser implementation (Phase 6)
pub struct FormatSelector {
    expression: String,
}

impl FormatSelector {
    /// Parse a format selection expression
    ///
    /// Currently returns a placeholder. Full implementation in Phase 6.
    pub fn parse(expression: &str) -> crate::Result<Self> {
        Ok(Self {
            expression: expression.to_string(),
        })
    }

    /// Select formats from available list
    ///
    /// Currently implements basic "best" selection.
    /// Full DSL evaluation will be implemented in Phase 6.
    ///
    /// # Explicit Lifetime Annotation
    ///
    /// This method uses explicit lifetime `'a` to document the relationship between
    /// input and output:
    /// ```rust,ignore
    /// pub fn select<'a>(&self, formats: &'a [Format]) -> Vec<&'a Format>
    /// //               ^^         ^^                         ^^
    /// //               |          |                          |
    /// //               Lifetime  Input references            Output references
    /// //               parameter have lifetime 'a            have same lifetime 'a
    /// ```
    ///
    /// **Why explicit here?** While lifetime elision would infer the same thing,
    /// the explicit annotation makes it clear to API users that:
    /// - The returned references borrow from the `formats` parameter (not from `&self`)
    /// - The returned `Vec<&Format>` is valid only as long as the input slice exists
    /// - The selector itself (`&self`) can be dropped; returned references don't depend on it
    ///
    /// **Without explicit lifetime (elision would be ambiguous):**
    /// ```rust,ignore
    /// // Which lifetime? From &self or from &[Format]?
    /// pub fn select(&self, formats: &[Format]) -> Vec<&Format>
    /// ```
    pub fn select<'a>(&self, formats: &'a [Format]) -> Vec<&'a Format> {
        // Simplified implementation for now
        match self.expression.as_str() {
            "best" => {
                // Find best format with both video and audio
                if let Some(best) = formats
                    .iter()
                    .filter(|f| f.has_video() && f.has_audio())
                    .max_by(|a, b| {
                        a.quality.cmp(&b.quality)
                            .then(a.height.cmp(&b.height))
                            .then(a.tbr.partial_cmp(&b.tbr).unwrap_or(std::cmp::Ordering::Equal))
                    })
                {
                    vec![best]
                } else {
                    Vec::new()
                }
            }
            "bestvideo" => {
                if let Some(best) = formats
                    .iter()
                    .filter(|f| f.has_video())
                    .max_by(|a, b| {
                        a.height.cmp(&b.height)
                            .then(a.vbr.partial_cmp(&b.vbr).unwrap_or(std::cmp::Ordering::Equal))
                    })
                {
                    vec![best]
                } else {
                    Vec::new()
                }
            }
            "bestaudio" => {
                if let Some(best) = formats
                    .iter()
                    .filter(|f| f.has_audio())
                    .max_by(|a, b| {
                        a.abr.partial_cmp(&b.abr).unwrap_or(std::cmp::Ordering::Equal)
                    })
                {
                    vec![best]
                } else {
                    Vec::new()
                }
            }
            _ => {
                // Default: return best format
                if let Some(best) = formats
                    .iter()
                    .max_by(|a, b| {
                        a.quality.cmp(&b.quality)
                            .then(a.height.cmp(&b.height))
                            .then(a.tbr.partial_cmp(&b.tbr).unwrap_or(std::cmp::Ordering::Equal))
                    })
                {
                    vec![best]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

/// Format bytes as human-readable string
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let bytes_f = bytes as f64;
    let exponent = (bytes_f.log10() / 3.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);

    let value = bytes_f / 1000_f64.powi(exponent as i32);
    let unit = UNITS[exponent];

    format!("{value:.1} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_has_video() {
        let mut format = Format::new(
            "137".to_string(),
            "https://example.com/video".to_string(),
            "mp4".to_string(),
            "https".to_string(),
        );

        format.vcodec = Some("h264".to_string());
        format.acodec = Some("none".to_string());

        assert!(format.has_video());
        assert!(!format.has_audio());
    }

    #[test]
    fn test_format_has_audio() {
        let mut format = Format::new(
            "140".to_string(),
            "https://example.com/audio".to_string(),
            "m4a".to_string(),
            "https".to_string(),
        );

        format.vcodec = Some("none".to_string());
        format.acodec = Some("aac".to_string());

        assert!(!format.has_video());
        assert!(format.has_audio());
    }

    #[test]
    fn test_format_selector_best() {
        let formats = vec![
            {
                let mut f = Format::new("1".to_string(), "url1".to_string(), "mp4".to_string(), "https".to_string());
                f.quality = Some(1);
                f.height = Some(720);
                f.vcodec = Some("h264".to_string());
                f.acodec = Some("aac".to_string());
                f
            },
            {
                let mut f = Format::new("2".to_string(), "url2".to_string(), "mp4".to_string(), "https".to_string());
                f.quality = Some(2);
                f.height = Some(1080);
                f.vcodec = Some("h264".to_string());
                f.acodec = Some("aac".to_string());
                f
            },
        ];

        let selector = FormatSelector::parse("best").unwrap();
        let selected = selector.select(&formats);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].format_id, "2");
    }
}
