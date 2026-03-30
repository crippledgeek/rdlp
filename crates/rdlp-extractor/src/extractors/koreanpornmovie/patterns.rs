//! URL patterns for KoreanPornMovie.

use regex::Regex;
use std::sync::LazyLock;

/// Matches any path on koreanpornmovie.com with a slug.
/// Non-video paths are filtered by `is_video_url()`.
pub(crate) static VIDEO_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?koreanpornmovie\.com/(?P<slug>[a-z0-9%][a-z0-9%\-]+)/?")
        .expect("valid KoreanPornMovie URL pattern")
});

/// Non-video path prefixes to exclude.
const EXCLUDED_SLUGS: &[&str] = &[
    "tags", "tag", "actors", "actor", "category", "author", "page",
    "wp-admin", "wp-content", "wp-json", "dmca", "contact-us",
    "privacy-policy", "our-partner", "2557-statement",
];

/// Check if a URL is a video page (not a tag/actor/category/static page).
pub(crate) fn is_video_url(url: &str) -> bool {
    if !VIDEO_URL_PATTERN.is_match(url) {
        return false;
    }
    if let Some(slug) = extract_slug(url) {
        !EXCLUDED_SLUGS.iter().any(|&excl| slug == excl || slug.starts_with(&format!("{excl}/")))
    } else {
        false
    }
}

/// Matches search URLs: `https://koreanpornmovie.com/?s=<query>`
#[allow(dead_code)]
pub(crate) static SEARCH_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?koreanpornmovie\.com/\?s=")
        .expect("valid KoreanPornMovie search URL pattern")
});

/// Extract slug from a video page URL.
pub(crate) fn extract_slug(url: &str) -> Option<String> {
    VIDEO_URL_PATTERN
        .captures(url)
        .and_then(|caps| caps.name("slug"))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_url_matches() {
        assert!(is_video_url("https://koreanpornmovie.com/taste-of-a-young-woman-2025/"));
        assert!(is_video_url("https://koreanpornmovie.com/gangnam-full-salon-2024/"));
        assert!(is_video_url("https://www.koreanpornmovie.com/some-video/"));
        assert!(is_video_url("https://koreanpornmovie.com/%ed%99%98%ec%9e%a5%ed%95%98%eb%88%88%ea%b5%ac%eb%a8%bc/"));
    }

    #[test]
    fn non_video_urls_excluded() {
        assert!(!is_video_url("https://koreanpornmovie.com/tags/"));
        assert!(!is_video_url("https://koreanpornmovie.com/tag/"));
        assert!(!is_video_url("https://koreanpornmovie.com/actors/"));
        assert!(!is_video_url("https://koreanpornmovie.com/actor/"));
        assert!(!is_video_url("https://koreanpornmovie.com/category/"));
        assert!(!is_video_url("https://koreanpornmovie.com/dmca/"));
        assert!(!is_video_url("https://koreanpornmovie.com/privacy-policy/"));
        assert!(!is_video_url("https://koreanpornmovie.com/wp-admin/"));
        assert!(!is_video_url("https://koreanpornmovie.com/contact-us/"));
    }

    #[test]
    fn slug_extraction() {
        assert_eq!(
            extract_slug("https://koreanpornmovie.com/taste-of-a-young-woman-2025/"),
            Some("taste-of-a-young-woman-2025".to_string())
        );
        assert_eq!(
            extract_slug("https://koreanpornmovie.com/gangnam-full-salon-2024/"),
            Some("gangnam-full-salon-2024".to_string())
        );
    }
}
