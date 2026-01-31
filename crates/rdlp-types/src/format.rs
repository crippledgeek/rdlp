//! Format types for video/audio streams

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{self, Write};
use std::sync::OnceLock;

/// Video/audio format information
///
/// Represents a single downloadable stream (video, audio, or combined).
/// Sites often provide multiple formats with different qualities, codecs, and containers.
#[derive(Clone, Serialize, Deserialize)]
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

    /// Total duration in seconds (for HLS: sum of segment durations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

impl fmt::Debug for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Format");

        // Always show required fields
        d.field("format_id", &self.format_id);
        d.field("url", &self.url);
        d.field("ext", &self.ext);
        d.field("protocol", &self.protocol);

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
        if let Some(v) = &self.vcodec {
            d.field("vcodec", v);
        }
        if let Some(v) = &self.acodec {
            d.field("acodec", v);
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
        if let Some(v) = &self.dynamic_range {
            d.field("dynamic_range", v);
        }
        if let Some(v) = &self.has_drm {
            d.field("has_drm", v);
        }
        if let Some(v) = &self.duration {
            d.field("duration", v);
        }

        d.finish()
    }
}

impl Format {
    /// Create a new format with required fields
    #[must_use]
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
            duration: None,
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
        self.protocol.contains("dash")
            || self.container.as_ref().is_some_and(|c| c.contains("dash"))
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

    /// Format as table row for interactive selection UI
    ///
    /// Returns a formatted string suitable for display in selection menus:
    /// `"720p         | 1280x720   | 245.3 MB     | MP4    | h264/aac"`
    ///
    /// Optimized to minimize heap allocations using pre-allocated buffer.
    pub fn table_row(&self) -> String {
        // Pre-allocate buffer for typical row length (~80 chars)
        let mut buf = String::with_capacity(80);

        // Quality column: append fps when non-standard (e.g. "1080p60")
        let quality_base = self.format_note.as_deref().unwrap_or("unknown");
        match self.fps {
            Some(fps) if fps > 0.0 && (fps - 30.0).abs() > 1.0 => {
                let _ = write!(buf, "{quality_base}{fps:.0}");
                let col_len = quality_base.len() + format!("{fps:.0}").len();
                for _ in col_len..12 {
                    buf.push(' ');
                }
                buf.push_str(" | ");
            }
            _ => {
                let _ = write!(buf, "{quality_base:<12} | ");
            }
        }

        // Resolution: avoid intermediate String allocation
        match (self.width, self.height) {
            (Some(w), Some(h)) => {
                let _ = write!(buf, "{w}x{h}");
                // Pad to 10 chars
                let len = buf.len() - 15; // account for "quality | "
                for _ in len..10 {
                    buf.push(' ');
                }
            }
            _ => buf.push_str("N/A       "),
        }
        buf.push_str(" | ");

        // Check if this is an HLS format (also check URL for .m3u8)
        let is_hls = self.is_hls() || self.url.contains(".m3u8");

        // Size column: write directly to buffer
        let size_start = buf.len();
        if is_hls {
            // For HLS: show duration + segment count when available
            let seg_count = self
                .fragments
                .as_ref()
                .map(|f| f.len() as u64)
                .or(self.filesize_approx);

            match (self.duration, seg_count) {
                (Some(dur), Some(segs)) => {
                    let mins = dur as u64 / 60;
                    let secs = dur as u64 % 60;
                    let _ = write!(buf, "{mins}:{secs:02} ({segs} seg)");
                }
                (Some(dur), None) => {
                    let mins = dur as u64 / 60;
                    let secs = dur as u64 % 60;
                    let _ = write!(buf, "{mins}:{secs:02}");
                }
                (None, Some(segs)) => {
                    let _ = write!(buf, "{segs} segments");
                }
                (None, None) => {
                    buf.push_str("HLS stream");
                }
            }
        } else if let Some(filesize) = self.filesize {
            let _ = write!(buf, "{:.1} MB", filesize as f64 / (1024.0 * 1024.0));
        } else if let Some(filesize_approx) = self.filesize_approx {
            let _ = write!(buf, "~{:.0} MB", filesize_approx as f64 / (1024.0 * 1024.0));
        } else {
            buf.push_str("Unknown");
        }
        // Pad size to 12 chars
        let size_len = buf.len() - size_start;
        for _ in size_len..12 {
            buf.push(' ');
        }
        buf.push_str(" | ");

        // Format type column: avoid to_uppercase() allocation for HLS
        let format_start = buf.len();
        if is_hls {
            buf.push_str("HLS");
        } else {
            // Write uppercase directly
            for c in self.ext.chars() {
                buf.push(c.to_ascii_uppercase());
            }
        }
        // Pad to 6 chars
        let format_len = buf.len() - format_start;
        for _ in format_len..6 {
            buf.push(' ');
        }
        buf.push_str(" | ");

        // Codecs column: write directly
        match (&self.vcodec, &self.acodec) {
            (Some(v), Some(a)) => {
                let _ = write!(buf, "{v}/{a}");
            }
            (Some(v), None) => {
                let _ = write!(buf, "{v} (video only)");
            }
            (None, Some(a)) => {
                let _ = write!(buf, "{a} (audio only)");
            }
            (None, None) => buf.push_str("Unknown"),
        }

        if self.has_drm.unwrap_or(false) {
            buf.push_str(" [DRM]");
        }

        buf
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

// ============================================================================
// Format Selection DSL
// ============================================================================
//
// Grammar:
//   expression   = format_spec ( "/" format_spec )*        -- fallback chain
//   format_spec  = selector ( "+" selector )?              -- video+audio merge
//   selector     = base_name filter*                       -- base with filters
//   filter       = "[" field op value "]"
//   base_name    = "best" | "worst" | "b" | "w"
//                | "bestvideo" | "bv" | "bv*"
//                | "bestaudio" | "ba" | "ba*"
//                | "worstvideo" | "wv"
//                | "worstaudio" | "wa"
//                | <format_id>
//   field        = "height" | "width" | "ext" | "vcodec" | "acodec"
//                | "fps" | "tbr" | "vbr" | "abr" | "asr"
//                | "filesize" | "protocol" | "format_id"
//   op           = "<=" | ">=" | "!=" | "<" | ">" | "="
//   value        = number | string

/// Parsed format selection expression supporting yt-dlp-compatible syntax.
///
/// # Examples
///
/// ```
/// use rdlp_types::FormatSelector;
///
/// // Basic selectors
/// let sel = FormatSelector::parse("best").unwrap();
/// let sel = FormatSelector::parse("bv+ba").unwrap();
///
/// // Filters
/// let sel = FormatSelector::parse("bv[height<=720]+ba").unwrap();
///
/// // Fallback chains
/// let sel = FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b").unwrap();
/// ```
pub struct FormatSelector {
    expression: String,
    fallbacks: Vec<FormatSpec>,
}

/// A single format specification: either a single selector or a video+audio merge.
#[derive(Debug, Clone, PartialEq)]
enum FormatSpec {
    Single(Selector),
    Merge { video: Selector, audio: Selector },
}

/// A base selector with optional filters.
#[derive(Debug, Clone, PartialEq)]
struct Selector {
    base: BaseSelector,
    filters: Vec<Filter>,
}

/// The base selector type determining which formats are candidates.
#[derive(Debug, Clone, PartialEq)]
enum BaseSelector {
    /// Best combined (video+audio) format
    Best,
    /// Worst combined (video+audio) format
    Worst,
    /// Best video-only format (excludes combined)
    BestVideo,
    /// Best video format (may include combined)
    BestVideoStar,
    /// Worst video-only format
    WorstVideo,
    /// Best audio-only format (excludes combined)
    BestAudio,
    /// Best audio format (may include combined)
    BestAudioStar,
    /// Worst audio-only format
    WorstAudio,
    /// Match a specific format ID
    FormatId(String),
}

/// A filter condition applied to a format field.
#[derive(Debug, Clone, PartialEq)]
struct Filter {
    field: FilterField,
    op: FilterOp,
    value: FilterValue,
}

/// Format fields that can be filtered on.
#[derive(Debug, Clone, PartialEq)]
enum FilterField {
    Height,
    Width,
    Ext,
    Vcodec,
    Acodec,
    Fps,
    Tbr,
    Vbr,
    Abr,
    Asr,
    Filesize,
    Protocol,
    FormatId,
}

/// Comparison operators for filters.
#[derive(Debug, Clone, PartialEq)]
enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A filter value: either a number or a string.
#[derive(Debug, Clone, PartialEq)]
enum FilterValue {
    Number(f64),
    Text(String),
}

impl FormatSelector {
    /// Parse a format selection expression.
    ///
    /// Returns an error if the expression is empty or contains invalid syntax.
    pub fn parse(expression: &str) -> Result<Self, String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("Empty format expression".to_string());
        }

        let fallback_parts = split_top_level(expression, '/');
        let mut fallbacks = Vec::with_capacity(fallback_parts.len());

        for part in &fallback_parts {
            let part = part.trim();
            if part.is_empty() {
                return Err("Empty fallback in format expression".to_string());
            }
            fallbacks.push(parse_format_spec(part)?);
        }

        Ok(Self {
            expression: expression.to_string(),
            fallbacks,
        })
    }

    /// Get the original expression string.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Select formats from the available list.
    ///
    /// Tries each fallback in order, returning the first non-empty result.
    /// Returns 1 format for single selectors, 2 for merge (`video+audio`),
    /// or 0 if nothing matches.
    pub fn select<'a>(&self, formats: &'a [Format]) -> Vec<&'a Format> {
        for spec in &self.fallbacks {
            let result = select_spec(spec, formats);
            if !result.is_empty() {
                return result;
            }
        }
        Vec::new()
    }
}

// ---- Parser helpers ----

/// Split a string on a delimiter, but only at the top level (not inside `[...]`).
fn split_top_level(s: &str, delim: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;

    for ch in s.chars() {
        if ch == '[' {
            depth += 1;
            current.push(ch);
        } else if ch == ']' {
            depth = depth.saturating_sub(1);
            current.push(ch);
        } else if ch == delim && depth == 0 {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Parse a format spec: `selector` or `selector+selector`.
fn parse_format_spec(s: &str) -> Result<FormatSpec, String> {
    let merge_parts = split_top_level(s, '+');

    match merge_parts.len() {
        1 => Ok(FormatSpec::Single(parse_selector(merge_parts[0].trim())?)),
        2 => Ok(FormatSpec::Merge {
            video: parse_selector(merge_parts[0].trim())?,
            audio: parse_selector(merge_parts[1].trim())?,
        }),
        _ => Err(format!("Invalid format spec (too many '+' operators): {s}")),
    }
}

/// Parse a selector: `base_name[filter1][filter2]...`
fn parse_selector(s: &str) -> Result<Selector, String> {
    if s.is_empty() {
        return Err("Empty selector".to_string());
    }

    // Split base name from filters: everything before the first `[`
    let (base_str, filter_str) = match s.find('[') {
        Some(idx) => (&s[..idx], &s[idx..]),
        None => (s, ""),
    };

    let base_str = base_str.trim();
    if base_str.is_empty() {
        return Err(format!("Missing base selector before filters in: {s}"));
    }

    let base = parse_base_selector(base_str)?;
    let filters = parse_filters(filter_str)?;

    Ok(Selector { base, filters })
}

/// Parse a base selector name into a `BaseSelector`.
fn parse_base_selector(s: &str) -> Result<BaseSelector, String> {
    match s {
        "best" | "b" => Ok(BaseSelector::Best),
        "worst" | "w" => Ok(BaseSelector::Worst),
        "bestvideo" | "bv" => Ok(BaseSelector::BestVideo),
        "bv*" | "bestvideo*" => Ok(BaseSelector::BestVideoStar),
        "worstvideo" | "wv" => Ok(BaseSelector::WorstVideo),
        "bestaudio" | "ba" => Ok(BaseSelector::BestAudio),
        "ba*" | "bestaudio*" => Ok(BaseSelector::BestAudioStar),
        "worstaudio" | "wa" => Ok(BaseSelector::WorstAudio),
        other => {
            // Treat as a literal format ID
            if other.contains(|c: char| c.is_whitespace()) {
                return Err(format!("Invalid selector: {other}"));
            }
            Ok(BaseSelector::FormatId(other.to_string()))
        }
    }
}

/// Parse a chain of `[field op value]` filters from a string like `[height<=720][ext=mp4]`.
fn parse_filters(s: &str) -> Result<Vec<Filter>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }

    let mut filters = Vec::new();
    let mut remaining = s;

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        if !remaining.starts_with('[') {
            return Err(format!("Expected '[' in filter expression: {remaining}"));
        }

        let close = remaining
            .find(']')
            .ok_or_else(|| format!("Unclosed filter bracket in: {remaining}"))?;

        let inner = &remaining[1..close];
        filters.push(parse_single_filter(inner)?);
        remaining = &remaining[close + 1..];
    }

    Ok(filters)
}

/// Parse the inside of a single filter: `field op value`.
fn parse_single_filter(s: &str) -> Result<Filter, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty filter".to_string());
    }

    // Find the operator — try two-char operators first, then single-char
    let (field_str, op, value_str) = if let Some(idx) = s.find("<=") {
        (&s[..idx], FilterOp::Le, &s[idx + 2..])
    } else if let Some(idx) = s.find(">=") {
        (&s[..idx], FilterOp::Ge, &s[idx + 2..])
    } else if let Some(idx) = s.find("!=") {
        (&s[..idx], FilterOp::Ne, &s[idx + 2..])
    } else if let Some(idx) = s.find('<') {
        (&s[..idx], FilterOp::Lt, &s[idx + 1..])
    } else if let Some(idx) = s.find('>') {
        (&s[..idx], FilterOp::Gt, &s[idx + 1..])
    } else if let Some(idx) = s.find('=') {
        (&s[..idx], FilterOp::Eq, &s[idx + 1..])
    } else {
        return Err(format!("No operator found in filter: {s}"));
    };

    let field_str = field_str.trim();
    let value_str = value_str.trim();

    let field = parse_filter_field(field_str)?;
    let value = parse_filter_value(value_str);

    Ok(Filter { field, op, value })
}

/// Parse a field name into `FilterField`.
fn parse_filter_field(s: &str) -> Result<FilterField, String> {
    match s {
        "height" => Ok(FilterField::Height),
        "width" => Ok(FilterField::Width),
        "ext" => Ok(FilterField::Ext),
        "vcodec" => Ok(FilterField::Vcodec),
        "acodec" => Ok(FilterField::Acodec),
        "fps" => Ok(FilterField::Fps),
        "tbr" => Ok(FilterField::Tbr),
        "vbr" => Ok(FilterField::Vbr),
        "abr" => Ok(FilterField::Abr),
        "asr" => Ok(FilterField::Asr),
        "filesize" => Ok(FilterField::Filesize),
        "protocol" => Ok(FilterField::Protocol),
        "format_id" => Ok(FilterField::FormatId),
        other => Err(format!("Unknown filter field: {other}")),
    }
}

/// Parse a filter value — try as number first, fall back to text.
fn parse_filter_value(s: &str) -> FilterValue {
    if let Ok(n) = s.parse::<f64>() {
        FilterValue::Number(n)
    } else {
        FilterValue::Text(s.to_string())
    }
}

// ---- Selection logic ----

/// Evaluate a `FormatSpec` against the format list.
fn select_spec<'a>(spec: &FormatSpec, formats: &'a [Format]) -> Vec<&'a Format> {
    match spec {
        FormatSpec::Single(sel) => {
            if let Some(f) = select_one(sel, formats) {
                vec![f]
            } else {
                Vec::new()
            }
        }
        FormatSpec::Merge { video, audio } => {
            let v = select_one(video, formats);
            let a = select_one(audio, formats);
            match (v, a) {
                (Some(v), Some(a)) => vec![v, a],
                // If only one side matched, return it (downloader can handle single-stream)
                (Some(v), None) => vec![v],
                (None, Some(a)) => vec![a],
                (None, None) => Vec::new(),
            }
        }
    }
}

/// Select a single format matching a `Selector`.
fn select_one<'a>(sel: &Selector, formats: &'a [Format]) -> Option<&'a Format> {
    let candidates: Vec<&Format> = formats
        .iter()
        .filter(|f| !f.has_drm.unwrap_or(false))
        .filter(|f| matches_base(&sel.base, f))
        .filter(|f| sel.filters.iter().all(|filter| matches_filter(filter, f)))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    match sort_direction(&sel.base) {
        SortDirection::Best => candidates
            .into_iter()
            .max_by(|a, b| rank_formats(&sel.base, a, b)),
        SortDirection::Worst => candidates
            .into_iter()
            .min_by(|a, b| rank_formats(&sel.base, a, b)),
    }
}

/// Whether a format is a candidate for the given base selector.
///
/// For `Best`/`Worst`, a format qualifies if it has both video and audio,
/// OR if codecs are unknown (both `None`) — assumed to be a combined stream.
fn matches_base(base: &BaseSelector, f: &Format) -> bool {
    let codecs_unknown = f.vcodec.is_none() && f.acodec.is_none();
    match base {
        BaseSelector::Best | BaseSelector::Worst => {
            (f.has_video() && f.has_audio()) || codecs_unknown
        }
        BaseSelector::BestVideo | BaseSelector::WorstVideo => f.has_video() && !f.has_audio(),
        BaseSelector::BestVideoStar => f.has_video() || codecs_unknown,
        BaseSelector::BestAudio | BaseSelector::WorstAudio => f.has_audio() && !f.has_video(),
        BaseSelector::BestAudioStar => f.has_audio() || codecs_unknown,
        BaseSelector::FormatId(id) => f.format_id == *id,
    }
}

/// Whether a format passes a single filter condition.
fn matches_filter(filter: &Filter, f: &Format) -> bool {
    match &filter.field {
        FilterField::Height => {
            compare_opt_num(f.height.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Width => compare_opt_num(f.width.map(|v| v as f64), &filter.op, &filter.value),
        FilterField::Fps => compare_opt_num(f.fps, &filter.op, &filter.value),
        FilterField::Tbr => compare_opt_num(f.tbr, &filter.op, &filter.value),
        FilterField::Vbr => compare_opt_num(f.vbr, &filter.op, &filter.value),
        FilterField::Abr => compare_opt_num(f.abr, &filter.op, &filter.value),
        FilterField::Asr => compare_opt_num(f.asr.map(|v| v as f64), &filter.op, &filter.value),
        FilterField::Filesize => {
            compare_opt_num(f.filesize.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Ext => compare_str(&f.ext, &filter.op, &filter.value),
        FilterField::Vcodec => match &f.vcodec {
            Some(v) => compare_str(v, &filter.op, &filter.value),
            None => false,
        },
        FilterField::Acodec => match &f.acodec {
            Some(v) => compare_str(v, &filter.op, &filter.value),
            None => false,
        },
        FilterField::Protocol => compare_str(&f.protocol, &filter.op, &filter.value),
        FilterField::FormatId => compare_str(&f.format_id, &filter.op, &filter.value),
    }
}

/// Compare an optional numeric value against a filter.
/// Returns `false` if the value is `None` (conservative — missing data doesn't match).
fn compare_opt_num(val: Option<f64>, op: &FilterOp, filter_val: &FilterValue) -> bool {
    let val = match val {
        Some(v) => v,
        None => return false,
    };
    let target = match filter_val {
        FilterValue::Number(n) => *n,
        FilterValue::Text(s) => match s.parse::<f64>() {
            Ok(n) => n,
            Err(_) => return false,
        },
    };
    match op {
        FilterOp::Eq => (val - target).abs() < f64::EPSILON,
        FilterOp::Ne => (val - target).abs() >= f64::EPSILON,
        FilterOp::Lt => val < target,
        FilterOp::Le => val <= target,
        FilterOp::Gt => val > target,
        FilterOp::Ge => val >= target,
    }
}

/// Compare a string value against a filter.
fn compare_str(val: &str, op: &FilterOp, filter_val: &FilterValue) -> bool {
    let target = match filter_val {
        FilterValue::Text(s) => s.as_str(),
        FilterValue::Number(n) => {
            // Numeric filter on string field — convert number to string for comparison
            // Use a stack-allocated comparison via to_string
            let s = n.to_string();
            return match op {
                FilterOp::Eq => val == s,
                FilterOp::Ne => val != s,
                _ => false, // Ordering ops don't make sense for string-vs-number
            };
        }
    };
    match op {
        FilterOp::Eq => val == target,
        FilterOp::Ne => val != target,
        // String ordering (lexicographic) for <, <=, >, >=
        FilterOp::Lt => val < target,
        FilterOp::Le => val <= target,
        FilterOp::Gt => val > target,
        FilterOp::Ge => val >= target,
    }
}

enum SortDirection {
    Best,
    Worst,
}

fn sort_direction(base: &BaseSelector) -> SortDirection {
    match base {
        BaseSelector::Worst | BaseSelector::WorstVideo | BaseSelector::WorstAudio => {
            SortDirection::Worst
        }
        _ => SortDirection::Best,
    }
}

/// Rank two formats by quality. Used with `max_by` (best) or `min_by` (worst).
fn rank_formats(base: &BaseSelector, a: &Format, b: &Format) -> std::cmp::Ordering {
    match base {
        BaseSelector::BestAudio | BaseSelector::BestAudioStar | BaseSelector::WorstAudio => {
            // Audio ranking: abr > asr
            a.abr
                .partial_cmp(&b.abr)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    a.asr
                        .partial_cmp(&b.asr)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        }
        BaseSelector::BestVideo | BaseSelector::BestVideoStar | BaseSelector::WorstVideo => {
            // Video ranking: height > vbr > fps
            a.height
                .cmp(&b.height)
                .then(
                    a.vbr
                        .partial_cmp(&b.vbr)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(
                    a.fps
                        .partial_cmp(&b.fps)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        }
        _ => {
            // Combined/general ranking: quality > height > tbr > fps
            a.quality
                .cmp(&b.quality)
                .then(a.height.cmp(&b.height))
                .then(
                    a.tbr
                        .partial_cmp(&b.tbr)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(
                    a.fps
                        .partial_cmp(&b.fps)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
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

    // ---- Format method tests ----

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

    // ---- Test helpers ----

    fn make_combined(id: &str, ext: &str, height: u32, quality: i32) -> Format {
        let mut f = Format::new(
            id.to_string(),
            format!("url_{id}"),
            ext.to_string(),
            "https".to_string(),
        );
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("aac".to_string());
        f.height = Some(height);
        f.width = Some(height * 16 / 9);
        f.quality = Some(quality);
        f.tbr = Some(height as f64 * 2.0);
        f
    }

    fn make_video_only(id: &str, ext: &str, height: u32) -> Format {
        let mut f = Format::new(
            id.to_string(),
            format!("url_{id}"),
            ext.to_string(),
            "https".to_string(),
        );
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("none".to_string());
        f.height = Some(height);
        f.width = Some(height * 16 / 9);
        f.vbr = Some(height as f64 * 1.5);
        f
    }

    fn make_audio_only(id: &str, ext: &str, abr: f64) -> Format {
        let mut f = Format::new(
            id.to_string(),
            format!("url_{id}"),
            ext.to_string(),
            "https".to_string(),
        );
        f.vcodec = Some("none".to_string());
        f.acodec = Some("aac".to_string());
        f.abr = Some(abr);
        f
    }

    fn test_formats() -> Vec<Format> {
        vec![
            make_combined("c360", "mp4", 360, 1),
            make_combined("c720", "mp4", 720, 2),
            make_combined("c1080", "mp4", 1080, 3),
            make_video_only("v720", "mp4", 720),
            make_video_only("v1080", "webm", 1080),
            make_video_only("v1440", "mp4", 1440),
            make_audio_only("a128", "m4a", 128.0),
            make_audio_only("a256", "m4a", 256.0),
            make_audio_only("a64", "webm", 64.0),
        ]
    }

    // ---- Parser tests ----

    #[test]
    fn test_parse_basic_selectors() {
        assert!(FormatSelector::parse("best").is_ok());
        assert!(FormatSelector::parse("worst").is_ok());
        assert!(FormatSelector::parse("b").is_ok());
        assert!(FormatSelector::parse("w").is_ok());
        assert!(FormatSelector::parse("bestvideo").is_ok());
        assert!(FormatSelector::parse("bv").is_ok());
        assert!(FormatSelector::parse("bv*").is_ok());
        assert!(FormatSelector::parse("bestaudio").is_ok());
        assert!(FormatSelector::parse("ba").is_ok());
        assert!(FormatSelector::parse("ba*").is_ok());
        assert!(FormatSelector::parse("worstvideo").is_ok());
        assert!(FormatSelector::parse("worstaudio").is_ok());
    }

    #[test]
    fn test_parse_format_id() {
        let sel = FormatSelector::parse("720p").unwrap();
        assert_eq!(sel.expression(), "720p");
    }

    #[test]
    fn test_parse_merge() {
        assert!(FormatSelector::parse("bv+ba").is_ok());
        assert!(FormatSelector::parse("bestvideo+bestaudio").is_ok());
        assert!(FormatSelector::parse("bv*+ba").is_ok());
    }

    #[test]
    fn test_parse_filters() {
        assert!(FormatSelector::parse("bv[height<=720]").is_ok());
        assert!(FormatSelector::parse("bv[height<=720]+ba[abr>=128]").is_ok());
        assert!(FormatSelector::parse("best[ext=mp4]").is_ok());
        assert!(FormatSelector::parse("bv[height<=1080][ext=mp4]").is_ok());
    }

    #[test]
    fn test_parse_fallback() {
        assert!(FormatSelector::parse("bv+ba/b").is_ok());
        assert!(FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/best").is_ok());
    }

    #[test]
    fn test_parse_errors() {
        assert!(FormatSelector::parse("").is_err());
        assert!(FormatSelector::parse("bv[height<=").is_err());
        assert!(FormatSelector::parse("bv[unknownfield=1]").is_err());
        assert!(FormatSelector::parse("[height<=720]").is_err()); // missing base
        assert!(FormatSelector::parse("bv[height]").is_err()); // no operator
    }

    #[test]
    fn test_parse_empty_fallback_rejected() {
        assert!(FormatSelector::parse("/best").is_err());
        assert!(FormatSelector::parse("best/").is_ok()); // trailing empty is trimmed out by split
    }

    #[test]
    fn test_parse_all_operators() {
        assert!(FormatSelector::parse("bv[height=720]").is_ok());
        assert!(FormatSelector::parse("bv[height!=720]").is_ok());
        assert!(FormatSelector::parse("bv[height<720]").is_ok());
        assert!(FormatSelector::parse("bv[height<=720]").is_ok());
        assert!(FormatSelector::parse("bv[height>720]").is_ok());
        assert!(FormatSelector::parse("bv[height>=720]").is_ok());
    }

    #[test]
    fn test_parse_all_fields() {
        for field in &[
            "height",
            "width",
            "ext",
            "vcodec",
            "acodec",
            "fps",
            "tbr",
            "vbr",
            "abr",
            "asr",
            "filesize",
            "protocol",
            "format_id",
        ] {
            let expr = format!("best[{field}=1]");
            assert!(
                FormatSelector::parse(&expr).is_ok(),
                "Failed to parse field: {field}"
            );
        }
    }

    // ---- Selection tests ----

    #[test]
    fn test_select_best() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080"); // highest quality combined
    }

    #[test]
    fn test_select_worst() {
        let formats = test_formats();
        let sel = FormatSelector::parse("worst").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c360"); // lowest quality combined
    }

    #[test]
    fn test_select_bestvideo() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bestvideo").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v1440"); // highest video-only
    }

    #[test]
    fn test_select_bestaudio() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bestaudio").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a256"); // highest audio-only
    }

    #[test]
    fn test_select_worstvideo() {
        let formats = test_formats();
        let sel = FormatSelector::parse("worstvideo").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v720"); // lowest video-only
    }

    #[test]
    fn test_select_worstaudio() {
        let formats = test_formats();
        let sel = FormatSelector::parse("worstaudio").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a64"); // lowest audio-only
    }

    #[test]
    fn test_select_bv_star_includes_combined() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv*").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        // bv* considers all formats with video, including combined — v1440 has highest height
        assert_eq!(result[0].format_id, "v1440");
    }

    #[test]
    fn test_select_ba_star_includes_combined() {
        let formats = test_formats();
        let sel = FormatSelector::parse("ba*").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        // ba* considers all formats with audio — a256 has highest abr
        assert_eq!(result[0].format_id, "a256");
    }

    #[test]
    fn test_select_merge() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv+ba").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format_id, "v1440"); // best video-only
        assert_eq!(result[1].format_id, "a256"); // best audio-only
    }

    #[test]
    fn test_select_filter_height_le() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best[height<=720]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720"); // best combined <=720
    }

    #[test]
    fn test_select_filter_height_lt() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best[height<720]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c360"); // only combined <720
    }

    #[test]
    fn test_select_filter_ext() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[ext=webm]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v1080"); // only webm video
    }

    #[test]
    fn test_select_filter_ext_ne() {
        let formats = test_formats();
        let sel = FormatSelector::parse("ba[ext!=m4a]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a64"); // only webm audio
    }

    #[test]
    fn test_select_filter_abr_ge() {
        let formats = test_formats();
        let sel = FormatSelector::parse("ba[abr>=128]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a256"); // best audio >=128
    }

    #[test]
    fn test_select_multiple_filters() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height<=1080][ext=mp4]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v720"); // mp4 video-only <=1080
    }

    #[test]
    fn test_select_merge_with_filters() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height<=720]+ba[abr>=128]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format_id, "v720"); // best video <=720
        assert_eq!(result[1].format_id, "a256"); // best audio >=128
    }

    #[test]
    fn test_select_fallback() {
        let formats = test_formats();
        // No video-only format at exactly 360p — falls back to best
        let sel = FormatSelector::parse("bv[height=360]/best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080"); // fallback to best
    }

    #[test]
    fn test_select_fallback_first_matches() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height<=720]/best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v720"); // first fallback matches
    }

    #[test]
    fn test_select_format_id() {
        let formats = test_formats();
        let sel = FormatSelector::parse("c720").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720");
    }

    #[test]
    fn test_select_drm_excluded() {
        let mut formats = test_formats();
        // Make the best combined format DRM-protected
        formats[2].has_drm = Some(true); // c1080
        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720"); // c1080 excluded, next best
    }

    #[test]
    fn test_select_empty_formats() {
        let formats: Vec<Format> = vec![];
        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_no_match() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height>=4320]").unwrap();
        let result = sel.select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_missing_field_conservative() {
        // Format without fps set — filter on fps should not match
        let mut f = make_combined("test", "mp4", 1080, 5);
        f.fps = None;
        let formats = vec![f];
        let sel = FormatSelector::parse("best[fps>=30]").unwrap();
        let result = sel.select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_shorthand_aliases() {
        let formats = test_formats();

        let sel_b = FormatSelector::parse("b").unwrap();
        let sel_best = FormatSelector::parse("best").unwrap();
        assert_eq!(
            sel_b.select(&formats)[0].format_id,
            sel_best.select(&formats)[0].format_id
        );

        let sel_w = FormatSelector::parse("w").unwrap();
        let sel_worst = FormatSelector::parse("worst").unwrap();
        assert_eq!(
            sel_w.select(&formats)[0].format_id,
            sel_worst.select(&formats)[0].format_id
        );
    }

    #[test]
    fn test_select_complex_expression() {
        let formats = test_formats();
        // "best mp4 video + m4a audio, fallback to best combined mp4, fallback to anything"
        let sel =
            FormatSelector::parse("bv[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4]/best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format_id, "v720"); // best mp4 video <=1080
        assert_eq!(result[1].format_id, "a256"); // best m4a audio
    }
}
