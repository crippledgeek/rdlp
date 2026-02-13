//! URL patterns for 9anime extractor.
//!
//! Static regex patterns compiled once at first use via `once_cell::sync::Lazy`.
//!
//! ## Supported URLs
//!
//! - Watch: `https://9animetv.to/watch/{slug}-{id}?ep={ep-id}`
//! - Home/search pages are not yet supported.

use once_cell::sync::Lazy;
use regex::Regex;

/// URL pattern for 9anime episode watch pages.
///
/// Captures:
/// - `slug` — anime slug (kebab-case title)
/// - `anime_id` — numeric anime ID
/// - `ep_id` — numeric episode ID (from query string)
pub static WATCH_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"https?://(?:www\.)?9animetv\.to/watch/(?P<slug>[\w-]+)-(?P<anime_id>\d+)\?ep=(?P<ep_id>\d+)",
    )
    .expect("Valid 9anime watch URL pattern")
});

/// Looser pattern that matches watch URLs without the `?ep=` query parameter.
///
/// Used by `suitable()` to match anime pages that might not have an episode selected.
pub static WATCH_URL_LOOSE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?9animetv\.to/watch/(?P<slug>[\w-]+)-(?P<anime_id>\d+)")
        .expect("Valid 9anime loose watch URL pattern")
});

/// Check if a URL is suitable for this extractor.
pub fn is_suitable(url: &str) -> bool {
    WATCH_URL_LOOSE_PATTERN.is_match(url)
}

/// Extract the anime ID from a watch URL.
pub fn extract_anime_id(url: &str) -> Option<String> {
    WATCH_URL_LOOSE_PATTERN
        .captures(url)
        .and_then(|caps| caps.name("anime_id").map(|m| m.as_str().to_string()))
}

/// Extract the episode ID from a watch URL query string.
pub fn extract_episode_id(url: &str) -> Option<String> {
    WATCH_URL_PATTERN
        .captures(url)
        .and_then(|caps| caps.name("ep_id").map(|m| m.as_str().to_string()))
}

/// Extract the slug from a watch URL.
pub fn extract_slug(url: &str) -> Option<String> {
    WATCH_URL_LOOSE_PATTERN
        .captures(url)
        .and_then(|caps| caps.name("slug").map(|m| m.as_str().to_string()))
}

/// Check if the URL contains an `?ep=` episode parameter.
///
/// URLs with `?ep=` target a single episode; URLs without it target the
/// entire anime (season/playlist).
pub fn has_episode_param(url: &str) -> bool {
    WATCH_URL_PATTERN.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_url_full() {
        let url = "https://9animetv.to/watch/sword-art-online-2274?ep=26565";
        assert!(WATCH_URL_PATTERN.is_match(url));
        assert_eq!(extract_anime_id(url), Some("2274".to_string()));
        assert_eq!(extract_episode_id(url), Some("26565".to_string()));
        assert_eq!(extract_slug(url), Some("sword-art-online".to_string()));
    }

    #[test]
    fn test_watch_url_without_ep() {
        let url = "https://9animetv.to/watch/sword-art-online-2274";
        assert!(WATCH_URL_LOOSE_PATTERN.is_match(url));
        assert!(is_suitable(url));
        assert_eq!(extract_anime_id(url), Some("2274".to_string()));
        assert_eq!(extract_episode_id(url), None);
    }

    #[test]
    fn test_watch_url_with_www() {
        let url = "https://www.9animetv.to/watch/one-piece-100?ep=12345";
        assert!(is_suitable(url));
        assert_eq!(extract_anime_id(url), Some("100".to_string()));
        assert_eq!(extract_episode_id(url), Some("12345".to_string()));
    }

    #[test]
    fn test_not_suitable() {
        assert!(!is_suitable("https://youtube.com/watch?v=test"));
        assert!(!is_suitable("https://9animetv.to/home"));
        assert!(!is_suitable("https://9animetv.to/search?keyword=test"));
    }

    #[test]
    fn test_has_episode_param() {
        assert!(has_episode_param(
            "https://9animetv.to/watch/sword-art-online-2274?ep=26565"
        ));
        assert!(!has_episode_param(
            "https://9animetv.to/watch/sword-art-online-2274"
        ));
    }

    #[test]
    fn test_slug_with_numbers() {
        let url =
            "https://9animetv.to/watch/attack-on-titan-the-final-season-part-3-18329?ep=99999";
        assert!(is_suitable(url));
        assert_eq!(extract_anime_id(url), Some("18329".to_string()));
        assert_eq!(
            extract_slug(url),
            Some("attack-on-titan-the-final-season-part-3".to_string())
        );
    }
}
