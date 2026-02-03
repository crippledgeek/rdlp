//! URL and extraction patterns for XTits
//!
//! Static regex patterns compiled once at first use via `once_cell::sync::Lazy`.

use once_cell::sync::Lazy;
use regex::Regex;

/// URL pattern for XTits video pages
///
/// Supports:
/// - Standard: `https://www.xtits.xxx/videos/183207/slug/`
/// - Without trailing slash: `https://www.xtits.xxx/videos/183207/slug`
/// - Without slug: `https://www.xtits.xxx/videos/183207/`
pub static XTITS_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?xtits\.xxx/videos/(?P<id>\d+)/")
        .expect("Valid XTits URL pattern")
});

/// URL pattern for XTits embed pages
///
/// Supports: `https://www.xtits.xxx/embed/183207`
pub static XTITS_EMBED_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?xtits\.xxx/embed/(?P<id>\d+)")
        .expect("Valid XTits embed pattern")
});

/// Regex to extract the KVS flashvars JavaScript object
///
/// KVS players embed video configuration in a `var flashvars = {...};` block.
/// This captures the JSON-like object content.
pub static FLASHVARS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)var\s+flashvars\s*=\s*\{(.+?)\};").expect("Valid flashvars pattern")
});

/// Check if URL is suitable for this extractor
pub fn is_suitable(url: &str) -> bool {
    XTITS_URL_PATTERN.is_match(url) || XTITS_EMBED_PATTERN.is_match(url)
}

/// Extract video ID from URL
pub fn extract_video_id(url: &str) -> Option<String> {
    XTITS_URL_PATTERN
        .captures(url)
        .or_else(|| XTITS_EMBED_PATTERN.captures(url))
        .and_then(|caps| caps.name("id"))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_pattern_standard() {
        assert!(XTITS_URL_PATTERN.is_match(
            "https://www.xtits.xxx/videos/183207/spicy-lesbians-and-straight-girl-smutty-adult-movie/"
        ));
        assert!(XTITS_URL_PATTERN.is_match("https://xtits.xxx/videos/183207/some-title/"));
    }

    #[test]
    fn test_url_pattern_embed() {
        assert!(XTITS_EMBED_PATTERN.is_match("https://www.xtits.xxx/embed/183207"));
    }

    #[test]
    fn test_url_pattern_invalid() {
        assert!(!is_suitable("https://youtube.com/watch?v=test"));
        assert!(!is_suitable(
            "https://www.pornhub.com/view_video.php?viewkey=ph123"
        ));
    }

    #[test]
    fn test_extract_video_id() {
        assert_eq!(
            extract_video_id(
                "https://www.xtits.xxx/videos/183207/spicy-lesbians-and-straight-girl-smutty-adult-movie/"
            ),
            Some("183207".to_string())
        );
        assert_eq!(
            extract_video_id("https://www.xtits.xxx/embed/183207"),
            Some("183207".to_string())
        );
        assert_eq!(extract_video_id("https://youtube.com/watch?v=test"), None);
    }

    #[test]
    fn test_flashvars_pattern() {
        let webpage =
            r#"var flashvars = { video_id: '123', video_url: 'https://example.com/v.mp4' };"#;
        assert!(FLASHVARS_PATTERN.is_match(webpage));
    }
}
