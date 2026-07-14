//! URL patterns and media type helpers for the generic extractor.

use lazy_regex::{Lazy, Regex, lazy_regex};

/// Catch-all URL pattern: matches any HTTP/HTTPS URL.
pub(crate) static GENERIC_URL_PATTERN: Lazy<Regex> = lazy_regex!(r"^https?://");

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

/// ASCII-case-insensitive prefix test that borrows instead of allocating.
///
/// Media type and subtype names are case-insensitive (RFC 6838 §4.2, RFC 9110
/// §8.3.1), so comparisons must case-fold — but `to_lowercase` would allocate a
/// `String` on every call. Mirrors the `eq_ignore_ascii_case` idiom already used
/// by `PrefetchedResponse::is_dash_content_type`.
fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// ASCII-case-insensitive substring test that borrows instead of allocating.
///
/// `needle` must be non-empty — `<[u8]>::windows` panics on a zero width. Every
/// call site passes a non-empty literal.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    debug_assert!(!needle.is_empty(), "windows(0) panics");
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Check if a Content-Type header indicates direct media.
///
/// Also the gate for the OpenGraph `og:video:type` / `og:audio:type` hint, which
/// distinguishes a real stream from an embed/player page (issue #493). Parameters
/// (`; codecs=…`) need no stripping here: every test is a prefix or substring
/// match against the type/subtype, which precedes the first `;`.
pub(crate) fn is_media_content_type(content_type: &str) -> bool {
    starts_with_ignore_ascii_case(content_type, "video/")
        || starts_with_ignore_ascii_case(content_type, "audio/")
        || contains_ignore_ascii_case(content_type, "mpegurl")
        || contains_ignore_ascii_case(content_type, "dash+xml")
        || contains_ignore_ascii_case(content_type, "x-flv")
        || contains_ignore_ascii_case(content_type, "mp2t")
}

/// Check if a Content-Type header indicates HTML.
pub(crate) fn is_html_content_type(content_type: &str) -> bool {
    contains_ignore_ascii_case(content_type, "text/html")
        || contains_ignore_ascii_case(content_type, "application/xhtml")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type/subtype names are case-insensitive (RFC 6838 §4.2, RFC 9110 §8.3.1).
    /// The zero-alloc rewrite must keep case-folding — a regression here would
    /// silently drop real streams served with an uppercase Content-Type.
    #[test]
    fn media_content_type_is_case_insensitive() {
        assert!(is_media_content_type("VIDEO/MP4"));
        assert!(is_media_content_type("Video/MP4; codecs=avc1.64001f"));
        assert!(is_media_content_type("Application/X-MPEGURL"));
        assert!(is_media_content_type("APPLICATION/DASH+XML"));
        assert!(is_html_content_type("TEXT/HTML; charset=UTF-8"));
    }

    /// Non-media types must not pass — `text/html` is the embed-page marker the
    /// OpenGraph gate keys on (issue #493).
    #[test]
    fn non_media_content_type_rejected() {
        assert!(!is_media_content_type("text/html"));
        assert!(!is_media_content_type("text/html; charset=UTF-8"));
        assert!(!is_media_content_type("application/x-shockwave-flash"));
        assert!(!is_media_content_type("application/json"));
        assert!(!is_media_content_type(""));
    }

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
