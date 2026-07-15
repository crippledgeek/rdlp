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
/// `needle` must be non-empty — `<[u8]>::windows` panics on a zero width. That is
/// unreachable by construction: every call site passes a non-empty literal, and
/// the attacker-controlled string is always the `haystack`. The `assert!`
/// documents the precondition and costs nothing for literal needles.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    assert!(!needle.is_empty(), "windows(0) panics");
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// The bare `type/subtype`, with any parameters and surrounding space removed.
///
/// `media-type = type "/" subtype *( OWS ";" OWS parameter )` (RFC 9110 §8.3.1),
/// so only the span before the first `;` identifies the type. Stripping matters
/// because the substring tests below would otherwise scan the parameters too, and
/// `text/html; profile=mp2t` would answer as media.
fn type_subtype(content_type: &str) -> &str {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
}

/// Check if a Content-Type header indicates direct media.
///
/// Also the gate for the OpenGraph `og:video:type` / `og:audio:type` hint, which
/// distinguishes a real stream from an embed/player page (issue #493).
pub(crate) fn is_media_content_type(content_type: &str) -> bool {
    let ct = type_subtype(content_type);
    starts_with_ignore_ascii_case(ct, "video/")
        || starts_with_ignore_ascii_case(ct, "audio/")
        || contains_ignore_ascii_case(ct, "mpegurl")
        || contains_ignore_ascii_case(ct, "dash+xml")
        || contains_ignore_ascii_case(ct, "x-flv")
        || contains_ignore_ascii_case(ct, "mp2t")
}

/// Check if a Content-Type header indicates HTML.
pub(crate) fn is_html_content_type(content_type: &str) -> bool {
    let ct = type_subtype(content_type);
    contains_ignore_ascii_case(ct, "text/html")
        || contains_ignore_ascii_case(ct, "application/xhtml")
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

    /// Only the type/subtype decides — a parameter must never satisfy the match.
    ///
    /// The substring tests (`mpegurl`, `mp2t`, …) would otherwise scan the
    /// parameters too, so `text/html; profile=mp2t` would pass as media and let
    /// an embed page back through the OpenGraph gate (issue #493).
    #[test]
    fn media_content_type_ignores_parameters() {
        assert!(!is_media_content_type("text/html; profile=mp2t"));
        assert!(!is_media_content_type("text/html; codecs=mpegurl"));
        assert!(!is_media_content_type("application/json; x=dash+xml"));
        // …while a genuine media type with parameters still passes.
        assert!(is_media_content_type("video/mp4; codecs=avc1.64001f"));
        assert!(is_media_content_type(
            "application/vnd.apple.mpegurl; charset=utf-8"
        ));
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
