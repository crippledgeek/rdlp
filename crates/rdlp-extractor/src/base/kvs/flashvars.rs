//! KVS (Kernel Video Sharing) flashvars parsing utilities.
//!
//! KVS is a common video platform used by many tube sites. It embeds video URLs
//! and metadata in a JavaScript `flashvars` object with `key: 'value'` syntax.
//!
//! # Flashvars Format
//!
//! KVS flashvars use a JavaScript object literal syntax:
//! ```javascript
//! var flashvars = {
//!     video_id: '183207',
//!     video_url: 'https://example.com/video.mp4/',
//!     video_url_text: '480p',
//!     video_alt_url: 'https://example.com/video_720p.mp4/',
//!     video_alt_url_text: '720p',
//!     preview_url: 'https://example.com/preview.jpg',
//!     video_duration: '1698',
//! };
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use rdlp_extractor::base::kvs::{KvsFlashvars, parse_kvs_flashvars};
//!
//! let flashvars = parse_kvs_flashvars(flashvars_content);
//! let video_url = flashvars.get("video_url");
//! let duration = flashvars.get_f64("video_duration");
//! ```

use lazy_regex::{Lazy, Regex, lazy_regex};

/// Pattern to detect `kt_player.js` script tag in HTML.
static KVS_SCRIPT_PATTERN: Lazy<Regex> = lazy_regex!(r#"<script[^>]+src=["'][^"']*kt_player\.js"#);

/// Pattern to extract the flashvars block from HTML.
static KVS_FLASHVARS_BLOCK: Lazy<Regex> = lazy_regex!(r#"(?:var\s+)?flashvars\s*=\s*\{([\s\S]*?)\}"#);

/// Pattern to extract a single KVS flashvar value (string).
///
/// Matches: `key: 'value'` anywhere in the flashvars block.
/// No line-start anchor because KVS sites often emit all entries on one line.
static KVS_STRING_PATTERN: Lazy<Regex> = lazy_regex!(r"(\w+)\s*:\s*'([^']*)'");

/// Parsed KVS flashvars as key-value pairs.
///
/// Provides convenient accessors for common value types.
#[derive(Debug, Clone, Default)]
pub(crate) struct KvsFlashvars {
    vars: Vec<(String, String)>,
}

impl KvsFlashvars {
    /// Create a new empty flashvars container.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self { vars: Vec::new() }
    }

    /// Get a string value by key.
    #[must_use]
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Get a value and parse it as f64.
    #[must_use]
    pub(crate) fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|s| s.parse().ok())
    }

    /// Get a value and parse it as u64.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|s| s.parse().ok())
    }

    /// Check if a key exists and has a non-empty value.
    #[must_use]
    pub(crate) fn has(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| !v.is_empty())
    }

    /// Get all key-value pairs as a slice.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn as_slice(&self) -> &[(String, String)] {
        &self.vars
    }

    /// Get the number of flashvars.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.vars.len()
    }

    /// Check if empty.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }
}

/// Parse KVS flashvars content into a structured container.
///
/// # Arguments
/// * `flashvars_content` - The raw flashvars JavaScript object content
///
/// # Returns
/// Parsed flashvars as a `KvsFlashvars` container
///
/// # Example
///
/// ```rust,ignore
/// let content = "video_url: 'https://example.com/video.mp4/', video_duration: '1698'";
/// let flashvars = parse_kvs_flashvars(content);
/// assert_eq!(flashvars.get("video_url"), Some("https://example.com/video.mp4/"));
/// assert_eq!(flashvars.get_f64("video_duration"), Some(1698.0));
/// ```
#[must_use]
pub(crate) fn parse_kvs_flashvars(flashvars_content: &str) -> KvsFlashvars {
    let vars = KVS_STRING_PATTERN
        .captures_iter(flashvars_content)
        .map(|caps| {
            (
                caps.get(1)
                    .expect("capture group 1 exists in matched pattern")
                    .as_str()
                    .to_string(),
                caps.get(2)
                    .expect("capture group 2 exists in matched pattern")
                    .as_str()
                    .to_string(),
            )
        })
        .collect();

    KvsFlashvars { vars }
}

// ============================================================================
// KVS format extraction from HTML
// ============================================================================

/// A video URL extracted from KVS flashvars.
#[derive(Debug, Clone)]
pub(crate) struct KvsFormat {
    /// Video URL
    pub url: String,
    /// Quality label (e.g., "480p", "720p") from `video_url_text` / `video_alt_url_text`
    pub quality: Option<String>,
    /// Whether this is the primary URL or an alternate
    pub is_primary: bool,
}

/// Check if the page contains a KVS player (`kt_player.js` script tag).
pub(crate) fn is_kvs_page(html_source: &str) -> bool {
    KVS_SCRIPT_PATTERN.is_match(html_source)
}

/// Extract KVS video format URLs from a page's raw HTML source.
///
/// Detects the `flashvars` block and parses `video_url`, `video_alt_url`,
/// `video_alt_url2`, etc. Returns an empty `Vec` if no flashvars found.
///
/// Does NOT check for `kt_player.js` — call [`is_kvs_page`] first if needed.
pub(crate) fn extract_kvs_formats(html_source: &str) -> Vec<KvsFormat> {
    let flashvars_content = match KVS_FLASHVARS_BLOCK.captures(html_source) {
        Some(caps) => caps.get(1).map(|m| m.as_str()).unwrap_or(""),
        None => return Vec::new(),
    };

    let vars = parse_kvs_flashvars(flashvars_content);
    let mut formats = Vec::new();

    // Primary video_url
    if let Some(url) = vars.get("video_url").filter(|u| !u.is_empty()) {
        formats.push(KvsFormat {
            url: url.to_string(),
            quality: vars.get("video_url_text").map(|s| s.to_string()),
            is_primary: true,
        });
    }

    // Alternate URLs: video_alt_url, video_alt_url2, video_alt_url3, ...
    if let Some(url) = vars.get("video_alt_url").filter(|u| !u.is_empty()) {
        formats.push(KvsFormat {
            url: url.to_string(),
            quality: vars.get("video_alt_url_text").map(|s| s.to_string()),
            is_primary: false,
        });
    }

    for i in 2..=5 {
        let key = format!("video_alt_url{i}");
        let text_key = format!("video_alt_url{i}_text");
        if let Some(url) = vars.get(&key).filter(|u| !u.is_empty()) {
            formats.push(KvsFormat {
                url: url.to_string(),
                quality: vars.get(&text_key).map(|s| s.to_string()),
                is_primary: false,
            });
        }
    }

    formats
}

/// Extract KVS metadata (thumbnail, duration) from flashvars.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct KvsMetadata {
    /// Preview/thumbnail URL from `preview_url`
    pub thumbnail: Option<String>,
    /// Duration in seconds from `video_duration`
    pub duration: Option<f64>,
}

/// Extract KVS metadata from a page's raw HTML source.
#[allow(dead_code)]
pub(crate) fn extract_kvs_metadata(html_source: &str) -> KvsMetadata {
    let flashvars_content = match KVS_FLASHVARS_BLOCK.captures(html_source) {
        Some(caps) => caps.get(1).map(|m| m.as_str()).unwrap_or(""),
        None => return KvsMetadata::default(),
    };

    let vars = parse_kvs_flashvars(flashvars_content);
    KvsMetadata {
        thumbnail: vars.get("preview_url").map(|s| s.to_string()),
        duration: vars.get_f64("video_duration"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kvs_flashvars() {
        let content = r#"
            video_id: '183207',
            video_url: 'https://example.com/video.mp4/',
            video_url_text: '480p',
            video_alt_url: 'https://example.com/video_720p.mp4/',
            video_alt_url_text: '720p',
            preview_url: 'https://example.com/preview.jpg',
            video_duration: '1698',
        "#;

        let flashvars = parse_kvs_flashvars(content);

        assert_eq!(flashvars.get("video_id"), Some("183207"));
        assert_eq!(
            flashvars.get("video_url"),
            Some("https://example.com/video.mp4/")
        );
        assert_eq!(flashvars.get("video_url_text"), Some("480p"));
        assert_eq!(flashvars.get("video_alt_url_text"), Some("720p"));
        assert_eq!(flashvars.get_f64("video_duration"), Some(1698.0));
        assert!(flashvars.has("video_url"));
        assert!(!flashvars.has("nonexistent"));
    }

    #[test]
    fn test_parse_kvs_flashvars_single_line() {
        // Real KVS pages often emit all flashvars on a single line
        let content = "video_id: '183207', video_url: 'https://example.com/video.mp4/', video_url_text: '480p'";

        let flashvars = parse_kvs_flashvars(content);

        assert_eq!(flashvars.get("video_id"), Some("183207"));
        assert_eq!(flashvars.get("video_url_text"), Some("480p"));
        assert_eq!(flashvars.len(), 3);
    }

    #[test]
    fn test_parse_kvs_flashvars_empty() {
        let flashvars = parse_kvs_flashvars("");
        assert!(flashvars.is_empty());
        assert_eq!(flashvars.get("anything"), None);
    }

    #[test]
    fn test_parse_kvs_flashvars_empty_value() {
        let content = "video_alt_url: '', video_url: 'https://example.com/video.mp4/'";
        let flashvars = parse_kvs_flashvars(content);

        assert_eq!(flashvars.get("video_alt_url"), Some(""));
        assert!(!flashvars.has("video_alt_url")); // Empty string means not "has"
        assert!(flashvars.has("video_url"));
    }

    #[test]
    fn test_is_kvs_page() {
        assert!(is_kvs_page(r#"<script src="/js/kt_player.js"></script>"#));
        assert!(is_kvs_page(
            r#"<script src="https://cdn.example.com/kt_player.js?v=1.2"></script>"#
        ));
        assert!(!is_kvs_page(r#"<script src="/js/player.js"></script>"#));
    }

    #[test]
    fn test_extract_kvs_formats() {
        let html = r#"<html><body>
            <script src="/js/kt_player.js"></script>
            <script>
                var flashvars = {
                    video_url: 'https://cdn.example.com/480.mp4/',
                    video_url_text: '480p',
                    video_alt_url: 'https://cdn.example.com/720.mp4/',
                    video_alt_url_text: '720p',
                };
            </script>
        </body></html>"#;

        let formats = extract_kvs_formats(html);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].url, "https://cdn.example.com/480.mp4/");
        assert_eq!(formats[0].quality.as_deref(), Some("480p"));
        assert!(formats[0].is_primary);
        assert_eq!(formats[1].url, "https://cdn.example.com/720.mp4/");
        assert_eq!(formats[1].quality.as_deref(), Some("720p"));
        assert!(!formats[1].is_primary);
    }

    #[test]
    fn test_extract_kvs_formats_no_flashvars() {
        let html = r#"<html><body><p>No flashvars</p></body></html>"#;
        assert!(extract_kvs_formats(html).is_empty());
    }

    #[test]
    fn test_extract_kvs_metadata() {
        let html = r#"<script>
            var flashvars = {
                preview_url: 'https://cdn.example.com/thumb.jpg',
                video_duration: '1698',
            };
        </script>"#;

        let meta = extract_kvs_metadata(html);
        assert_eq!(
            meta.thumbnail.as_deref(),
            Some("https://cdn.example.com/thumb.jpg")
        );
        assert_eq!(meta.duration, Some(1698.0));
    }
}
