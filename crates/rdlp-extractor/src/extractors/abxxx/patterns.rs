//! URL patterns for ABXXX.
//!
//! ABXXX is a KVS (Kernel Video Sharing) tube site with the player config
//! delivered via a JSON XHR endpoint rather than inline `flashvars`.

use lazy_regex::{Lazy, Regex, lazy_regex};

/// URL pattern for ABXXX video pages.
///
/// Supports:
/// - `https://abxxx.com/video/129452/excogi-katie-carmine-in-hd/`
/// - `https://abxxx.com/video/129452/excogi-katie-carmine-in-hd`
/// - `https://www.abxxx.com/video/129452/`
pub(crate) static ABXXX_URL_PATTERN: Lazy<Regex> =
    lazy_regex!(r"https?://(?:www\.)?abxxx\.com/video/(?P<id>\d+)(?:/(?P<slug>[^/?#]+))?/?");

/// Whether `url` is an ABXXX video page.
#[must_use]
pub(crate) fn is_suitable(url: &str) -> bool {
    ABXXX_URL_PATTERN.is_match(url)
}

/// Extract the numeric video id from the URL.
#[must_use]
pub(crate) fn extract_video_id(url: &str) -> Option<String> {
    ABXXX_URL_PATTERN
        .captures(url)
        .and_then(|c| c.name("id"))
        .map(|m| m.as_str().to_string())
}

/// Extract the slug (title segment) from the URL, if present.
#[must_use]
pub(crate) fn extract_slug(url: &str) -> Option<String> {
    ABXXX_URL_PATTERN
        .captures(url)
        .and_then(|c| c.name("slug"))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_canonical_url() {
        assert!(is_suitable(
            "https://abxxx.com/video/129452/excogi-katie-carmine-in-hd/"
        ));
        assert!(is_suitable("https://www.abxxx.com/video/129452/some-title"));
        assert!(is_suitable("https://abxxx.com/video/1/"));
    }

    #[test]
    fn rejects_unrelated_urls() {
        assert!(!is_suitable("https://youtube.com/watch?v=test"));
        assert!(!is_suitable("https://abxxx.com/categories/foo/"));
        assert!(!is_suitable("https://example.com/video/1/title/"));
    }

    #[test]
    fn extracts_id_and_slug() {
        let url = "https://abxxx.com/video/129452/excogi-katie-carmine-in-hd/";
        assert_eq!(extract_video_id(url).as_deref(), Some("129452"));
        assert_eq!(
            extract_slug(url).as_deref(),
            Some("excogi-katie-carmine-in-hd")
        );
    }

    #[test]
    fn extracts_id_without_slug() {
        let url = "https://abxxx.com/video/42/";
        assert_eq!(extract_video_id(url).as_deref(), Some("42"));
        assert_eq!(extract_slug(url), None);
    }
}
