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

use regex::Regex;
use std::sync::LazyLock;

/// Pattern to extract a single KVS flashvar value (string).
///
/// Matches: `key: 'value'` anywhere in the flashvars block.
/// No line-start anchor because KVS sites often emit all entries on one line.
static KVS_STRING_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\w+)\s*:\s*'([^']*)'").expect("Valid KVS string pattern"));

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
}
