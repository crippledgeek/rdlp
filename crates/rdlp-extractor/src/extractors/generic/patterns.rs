//! URL patterns and media type helpers for the generic extractor.

use regex::Regex;
use std::sync::LazyLock;

/// Catch-all URL pattern: matches any HTTP/HTTPS URL.
pub(crate) static GENERIC_URL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://").expect("valid generic URL regex"));

/// Known media file extensions for direct URL detection.
#[allow(dead_code)]
pub(crate) const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "webm", "mkv", "m3u8", "mpd", "ts", "flv", "avi", "mov", "m4v", "mp3", "m4a", "ogg",
    "opus", "wav", "flac", "aac",
];

/// Check if a URL path has a known media file extension.
#[allow(dead_code)]
pub(crate) fn has_media_extension(url: &str) -> bool {
    // Extract path, stripping query string
    let path = url.split('?').next().unwrap_or(url);
    if let Some(ext) = path.rsplit('.').next() {
        let ext_lower = ext.to_lowercase();
        MEDIA_EXTENSIONS.contains(&ext_lower.as_str())
    } else {
        false
    }
}

/// Check if a Content-Type header indicates direct media.
pub(crate) fn is_media_content_type(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();
    ct.starts_with("video/")
        || ct.starts_with("audio/")
        || ct.contains("mpegurl")
        || ct.contains("dash+xml")
        || ct.contains("x-flv")
        || ct.contains("mp2t")
}

/// Check if a Content-Type header indicates HTML.
pub(crate) fn is_html_content_type(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();
    ct.contains("text/html") || ct.contains("application/xhtml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_extension_detected() {
        assert!(has_media_extension("https://cdn.example.com/video.mp4"));
        assert!(has_media_extension("https://cdn.example.com/stream.m3u8"));
        assert!(has_media_extension(
            "https://cdn.example.com/video.webm?token=abc"
        ));
        assert!(has_media_extension("https://cdn.example.com/audio.opus"));
    }

    #[test]
    fn non_media_extension_rejected() {
        assert!(!has_media_extension("https://example.com/page.html"));
        assert!(!has_media_extension("https://example.com/page.php"));
        assert!(!has_media_extension("https://example.com/page"));
        assert!(!has_media_extension("https://example.com/"));
    }

    #[test]
    fn media_content_type_detected() {
        assert!(is_media_content_type("video/mp4"));
        assert!(is_media_content_type("video/webm; codecs=vp9"));
        assert!(is_media_content_type("audio/mpeg"));
        assert!(is_media_content_type("application/vnd.apple.mpegurl"));
        assert!(is_media_content_type("application/x-mpegURL"));
        assert!(is_media_content_type("application/dash+xml"));
    }

    #[test]
    fn html_content_type_not_media() {
        assert!(!is_media_content_type("text/html"));
        assert!(!is_media_content_type("text/html; charset=utf-8"));
        assert!(!is_media_content_type("application/json"));
    }

    #[test]
    fn html_content_type_detected() {
        assert!(is_html_content_type("text/html"));
        assert!(is_html_content_type("text/html; charset=utf-8"));
        assert!(is_html_content_type("application/xhtml+xml"));
    }

    #[test]
    fn generic_url_pattern_matches() {
        assert!(GENERIC_URL_PATTERN.is_match("https://example.com/video"));
        assert!(GENERIC_URL_PATTERN.is_match("http://example.com/"));
        assert!(!GENERIC_URL_PATTERN.is_match("ftp://example.com/file"));
        assert!(!GENERIC_URL_PATTERN.is_match("not-a-url"));
    }
}
