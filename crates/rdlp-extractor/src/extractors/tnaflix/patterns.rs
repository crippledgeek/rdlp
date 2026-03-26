//! URL patterns for TNAFlix network sites
//!
//! Static regex patterns compiled once at first use via `std::sync::LazyLock`.
//!
//! ## Supported Sites
//!
//! - TNAFlix: `https://www.tnaflix.com/category/title/video123456`
//! - EMPFlix: `https://www.empflix.com/videos/title-123` and variants
//! - MovieFap: `https://www.moviefap.com/videos/abc123/title.html`

use regex::Regex;
use std::sync::LazyLock;

/// Static URL pattern regex for TNAFlix
///
/// Performance: Using static lazy patterns prevents regex compilation overhead:
/// - Without lazy: ~50-80μs compilation per constructor call
/// - With lazy: ~0.01μs access after first initialization
pub static TNAFLIX_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?tnaflix\.com/[^/]+/[^/]+/video(\d+)")
        .expect("Valid TNAFlix URL pattern")
});

/// Static URL pattern regex for EMPFlix
///
/// Supports multiple URL formats:
/// - `/videos/title-ID` format
/// - `/category/title/videoID` format
/// - `/category/ID` format
pub static EMPFLIX_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?empflix\.com/(?:videos/(?:[^/]+-)?(\d+)|[^/]+/[^/]+/video(\d+)|[^/]+/(\d+))")
        .expect("Valid EMPFlix URL pattern")
});

/// Static URL pattern regex for MovieFap
pub static MOVIEFAP_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?moviefap\.com/videos/([0-9a-f]+)/[^/]+\.html")
        .expect("Valid MovieFap URL pattern")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tnaflix_url_pattern() {
        assert!(TNAFLIX_URL_PATTERN.is_match("https://www.tnaflix.com/hd-videos/test/video123456"));
        assert!(TNAFLIX_URL_PATTERN.is_match("https://tnaflix.com/amateur-porn/title/video999"));
        assert!(!TNAFLIX_URL_PATTERN.is_match("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_empflix_url_pattern() {
        assert!(EMPFLIX_URL_PATTERN.is_match("https://www.empflix.com/videos/title-123"));
        assert!(EMPFLIX_URL_PATTERN.is_match("https://empflix.com/view/123"));
        assert!(EMPFLIX_URL_PATTERN.is_match(
            "https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093"
        ));
        assert!(!EMPFLIX_URL_PATTERN.is_match("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_moviefap_url_pattern() {
        assert!(
            MOVIEFAP_URL_PATTERN.is_match("https://www.moviefap.com/videos/abc123def/title.html")
        );
        assert!(!MOVIEFAP_URL_PATTERN.is_match("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_tnaflix_id_extraction() {
        let caps = TNAFLIX_URL_PATTERN
            .captures("https://www.tnaflix.com/hd-videos/test/video123456")
            .unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "123456");
    }

    // ---- Negative URL pattern tests ----

    #[test]
    fn test_tnaflix_rejects_missing_video_prefix() {
        assert!(!TNAFLIX_URL_PATTERN.is_match("https://www.tnaflix.com/category/title/123"));
    }

    #[test]
    fn test_tnaflix_rejects_non_numeric_id() {
        assert!(!TNAFLIX_URL_PATTERN.is_match("https://www.tnaflix.com/cat/title/videoABC"));
    }

    #[test]
    fn test_tnaflix_rejects_single_path_segment() {
        assert!(!TNAFLIX_URL_PATTERN.is_match("https://www.tnaflix.com/video123"));
    }

    #[test]
    fn test_tnaflix_rejects_wrong_domain() {
        assert!(!TNAFLIX_URL_PATTERN.is_match("https://www.empflix.com/cat/title/video123"));
    }

    #[test]
    fn test_empflix_rejects_wrong_domain() {
        assert!(!EMPFLIX_URL_PATTERN.is_match("https://www.tnaflix.com/videos/title-123"));
    }

    #[test]
    fn test_empflix_rejects_missing_id() {
        assert!(!EMPFLIX_URL_PATTERN.is_match("https://www.empflix.com/videos/title-"));
    }

    #[test]
    fn test_empflix_rejects_random_path() {
        assert!(!EMPFLIX_URL_PATTERN.is_match("https://www.empflix.com/search?q=test"));
    }

    #[test]
    fn test_moviefap_rejects_uppercase_hex() {
        // Regex is [0-9a-f] only — uppercase hex should not match
        assert!(!MOVIEFAP_URL_PATTERN.is_match("https://www.moviefap.com/videos/ABCDEF/title.html"));
    }

    #[test]
    fn test_moviefap_rejects_missing_html_extension() {
        assert!(!MOVIEFAP_URL_PATTERN.is_match("https://www.moviefap.com/videos/abc123/title"));
    }

    #[test]
    fn test_moviefap_rejects_non_hex_id() {
        assert!(!MOVIEFAP_URL_PATTERN.is_match("https://www.moviefap.com/videos/xyz!/title.html"));
    }

    #[test]
    fn test_moviefap_rejects_wrong_domain() {
        assert!(!MOVIEFAP_URL_PATTERN.is_match("https://www.empflix.com/videos/abc123/title.html"));
    }

    #[test]
    fn test_empflix_id_extraction() {
        // /videos/title-ID format
        let caps1 = EMPFLIX_URL_PATTERN
            .captures("https://www.empflix.com/videos/title-123")
            .unwrap();
        assert_eq!(caps1.get(1).unwrap().as_str(), "123");

        // /category/ID format
        let caps2 = EMPFLIX_URL_PATTERN
            .captures("https://empflix.com/view/456")
            .unwrap();
        assert_eq!(caps2.get(3).unwrap().as_str(), "456");

        // /category/title/videoID format
        let caps3 = EMPFLIX_URL_PATTERN
            .captures("https://www.empflix.com/amateur-porn/older-medical-doc/video3715093")
            .unwrap();
        assert_eq!(caps3.get(2).unwrap().as_str(), "3715093");
    }
}
