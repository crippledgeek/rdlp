//! URL and extraction patterns for XTits
//!
//! Static regex patterns compiled once at first use via `lazy_regex!`.

use lazy_regex::{lazy_regex, Lazy, Regex};

/// URL pattern for XTits video pages
///
/// Supports:
/// - Standard: `https://www.xtits.xxx/videos/183207/slug/`
/// - Without trailing slash: `https://www.xtits.xxx/videos/183207/slug`
/// - Without slug: `https://www.xtits.xxx/videos/183207/`
pub static XTITS_URL_PATTERN: Lazy<Regex> = lazy_regex!(r"https?://(?:www\.)?xtits\.(?:xxx|com)/videos/(?P<id>\d+)/");

/// URL pattern for XTits embed pages
///
/// Supports: `https://www.xtits.xxx/embed/183207`
pub static XTITS_EMBED_PATTERN: Lazy<Regex> = lazy_regex!(r"https?://(?:www\.)?xtits\.(?:xxx|com)/embed/(?P<id>\d+)");

/// Regex to extract the KVS flashvars JavaScript object
///
/// KVS players embed video configuration in a `var flashvars = {...};` block.
/// This captures the JSON-like object content.
pub static FLASHVARS_PATTERN: Lazy<Regex> = lazy_regex!(r"(?s)var\s+flashvars\s*=\s*\{(.+?)\};");

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

    #[test]
    fn test_url_pattern_com_domain() {
        assert!(
            XTITS_URL_PATTERN
                .is_match("https://www.xtits.com/videos/50088/blonde-amateur-gf-amateur/")
        );
        assert!(XTITS_URL_PATTERN.is_match("https://xtits.com/videos/12345/test-title/"));
    }

    #[test]
    fn test_embed_pattern_com_domain() {
        assert!(XTITS_EMBED_PATTERN.is_match("https://www.xtits.com/embed/50088"));
        assert!(XTITS_EMBED_PATTERN.is_match("https://xtits.com/embed/50088"));
    }

    #[test]
    fn test_extract_video_id_com_domain() {
        assert_eq!(
            extract_video_id("https://www.xtits.com/videos/50088/blonde-amateur-gf-amateur/"),
            Some("50088".to_string())
        );
        assert_eq!(
            extract_video_id("https://www.xtits.com/embed/50088"),
            Some("50088".to_string())
        );
    }

    #[test]
    fn test_suitable_com_domain() {
        assert!(is_suitable("https://www.xtits.com/videos/50088/test/"));
        assert!(is_suitable("https://www.xtits.com/embed/50088"));
    }
}
