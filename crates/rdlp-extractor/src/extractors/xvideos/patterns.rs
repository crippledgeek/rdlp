//! URL patterns for XVideos extractor.
//!
//! Static regex patterns compiled once at first use via `std::sync::LazyLock`.

use regex::Regex;
use std::sync::LazyLock;

/// URL pattern for XVideos video pages.
///
/// Supports:
/// - Canonical: `https://www.xvideos.com/video.ooumovia9b7/slug`
/// - Four-segment redirect: `/video.ooumovia9b7/47370580/0/slug`
/// - Language subdomains: `fr.xvideos.com`, `de.xvideos.es`
/// - `xvideos2.com` and `.es` TLD variants
pub static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://(?:[a-z]{2}\.)?(?:www\.)?xvideos(?:2)?\.(?:com|es)/video\.([a-z0-9]+)(?:/|$)",
    )
    .expect("Valid XVideos VIDEO_URL pattern")
});

/// URL pattern for XVideos embed pages.
pub static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://(?:[a-z]{2}\.)?(?:www\.)?xvideos(?:2)?\.(?:com|es)/embedframe/([a-z0-9]+)",
    )
    .expect("Valid XVideos EMBED_URL pattern")
});

/// Extract the video EID (alphanumeric slug after `video.`) from a URL.
pub fn extract_eid(url: &str) -> Option<String> {
    VIDEO_URL
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_canonical_video_url() {
        let url = "https://www.xvideos.com/video.ooumovia9b7/some-slug";
        let caps = VIDEO_URL.captures(url).expect("should match");
        assert_eq!(&caps[1], "ooumovia9b7");
    }

    #[test]
    fn matches_redirected_four_segment_video_url() {
        let url = "https://www.xvideos.com/video.ooumovia9b7/47370580/0/some-slug";
        let caps = VIDEO_URL.captures(url).expect("should match");
        assert_eq!(&caps[1], "ooumovia9b7");
    }

    #[test]
    fn matches_language_subdomain() {
        let fr_url = "https://fr.xvideos.com/video.ooumovia9b7/slug";
        assert!(VIDEO_URL.is_match(fr_url), "fr. subdomain should match");

        let de_es_url = "https://de.xvideos.es/video.ooumovia9b7/slug";
        assert!(VIDEO_URL.is_match(de_es_url), "de.xvideos.es should match");
    }

    #[test]
    fn rejects_non_xvideos() {
        assert!(!VIDEO_URL.is_match("https://www.xnxx.com/video12345/title"));
        assert!(!VIDEO_URL.is_match("https://example.com/video.abc123/slug"));
    }

    #[test]
    fn matches_embed_url() {
        let url = "https://www.xvideos.com/embedframe/ooumovia9b7";
        let caps = EMBED_URL.captures(url).expect("should match");
        assert_eq!(&caps[1], "ooumovia9b7");
    }
}
