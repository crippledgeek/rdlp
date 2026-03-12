//! URL patterns for HQPorner extractor.
//!
//! Contains regex patterns for matching video, category, actress, and search URLs.

use regex::Regex;
use std::sync::LazyLock;

/// URL pattern for HQPorner video pages.
///
/// Matches:
/// - `https://hqporner.com/hdporn/81203-full_body_massage.html`
/// - `https://www.hqporner.com/hdporn/81203-slug.html`
/// - `https://m.hqporner.com/hdporn/81203-slug.html`
///
/// The slug is ignored by the server — only the numeric ID matters.
pub static HQPORNER_VIDEO_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        https?://
        (?:(?:www|m)\.)?
        hqporner\.com
        /hdporn/(?P<id>\d+)-[\w-]+\.html
        ",
    )
    .expect("Valid HQPorner video URL pattern")
});

/// URL pattern for HQPorner category pages.
///
/// Matches:
/// - `https://hqporner.com/category/amateur`
/// - `https://hqporner.com/category/1080p-porn/3`
pub static HQPORNER_CATEGORY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        https?://
        (?:(?:www|m)\.)?
        hqporner\.com
        /category/(?P<name>[\w-]+)
        (?:/(?P<page>\d+))?
        ",
    )
    .expect("Valid HQPorner category URL pattern")
});

/// URL pattern for HQPorner actress pages.
///
/// Matches:
/// - `https://hqporner.com/actress/emily-bloom`
/// - `https://hqporner.com/actress/emily-bloom/2`
pub static HQPORNER_ACTRESS_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        https?://
        (?:(?:www|m)\.)?
        hqporner\.com
        /actress/(?P<name>[\w-]+)
        (?:/(?P<page>\d+))?
        ",
    )
    .expect("Valid HQPorner actress URL pattern")
});

/// URL pattern for HQPorner search pages.
///
/// Matches:
/// - `https://hqporner.com/?q=massage`
/// - `https://hqporner.com/?q=massage&p=2`
pub static HQPORNER_SEARCH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        https?://
        (?:(?:www|m)\.)?
        hqporner\.com
        /\?q=
        ",
    )
    .expect("Valid HQPorner search URL pattern")
});

/// Pattern to extract the mydaddy.cc iframe embed URL from a video page.
pub static IFRAME_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"iframe[^>]+src="(//mydaddy\.cc/video/[^"]+)""#).expect("Valid iframe pattern")
});

/// Pattern to extract video links from listing pages.
pub static VIDEO_LINK_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href="(/hdporn/\d+-[\w-]+\.html)""#).expect("Valid video link pattern")
});

/// Extract the numeric video ID from a video URL.
pub fn extract_video_id(url: &str) -> Option<String> {
    HQPORNER_VIDEO_PATTERN
        .captures(url)
        .and_then(|c| c.name("id"))
        .map(|m| m.as_str().to_string())
}

/// Check whether a URL matches any HQPorner URL type.
pub fn is_suitable(url: &str) -> bool {
    HQPORNER_VIDEO_PATTERN.is_match(url)
        || HQPORNER_CATEGORY_PATTERN.is_match(url)
        || HQPORNER_ACTRESS_PATTERN.is_match(url)
        || HQPORNER_SEARCH_PATTERN.is_match(url)
}

/// Check whether a URL is a category page.
pub fn is_category_url(url: &str) -> bool {
    HQPORNER_CATEGORY_PATTERN.is_match(url)
}

/// Check whether a URL is an actress page.
pub fn is_actress_url(url: &str) -> bool {
    HQPORNER_ACTRESS_PATTERN.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_pattern_standard() {
        assert!(
            HQPORNER_VIDEO_PATTERN
                .is_match("https://hqporner.com/hdporn/81203-full_body_massage.html")
        );
    }

    #[test]
    fn test_video_pattern_www() {
        assert!(
            HQPORNER_VIDEO_PATTERN
                .is_match("https://www.hqporner.com/hdporn/81203-full_body_massage.html")
        );
    }

    #[test]
    fn test_video_pattern_mobile() {
        assert!(
            HQPORNER_VIDEO_PATTERN
                .is_match("https://m.hqporner.com/hdporn/81203-full_body_massage.html")
        );
    }

    #[test]
    fn test_video_pattern_rejects_category() {
        assert!(!HQPORNER_VIDEO_PATTERN.is_match("https://hqporner.com/category/amateur"));
    }

    #[test]
    fn test_extract_video_id() {
        assert_eq!(
            extract_video_id("https://hqporner.com/hdporn/81203-full_body_massage.html"),
            Some("81203".to_string())
        );
    }

    #[test]
    fn test_category_pattern() {
        assert!(HQPORNER_CATEGORY_PATTERN.is_match("https://hqporner.com/category/1080p-porn"));
        assert!(HQPORNER_CATEGORY_PATTERN.is_match("https://hqporner.com/category/1080p-porn/3"));
    }

    #[test]
    fn test_actress_pattern() {
        assert!(HQPORNER_ACTRESS_PATTERN.is_match("https://hqporner.com/actress/emily-bloom"));
        assert!(HQPORNER_ACTRESS_PATTERN.is_match("https://hqporner.com/actress/emily-bloom/2"));
    }

    #[test]
    fn test_search_pattern() {
        assert!(HQPORNER_SEARCH_PATTERN.is_match("https://hqporner.com/?q=massage"));
        assert!(HQPORNER_SEARCH_PATTERN.is_match("https://hqporner.com/?q=massage&p=2"));
    }

    #[test]
    fn test_is_suitable_all_types() {
        assert!(is_suitable("https://hqporner.com/hdporn/81203-slug.html"));
        assert!(is_suitable("https://hqporner.com/category/amateur"));
        assert!(is_suitable("https://hqporner.com/actress/emily-bloom"));
        assert!(is_suitable("https://hqporner.com/?q=test"));
        assert!(!is_suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_iframe_pattern() {
        let html = r#"<iframe width="560" height="350" src="//mydaddy.cc/video/97d0145823aeb8edca/" frameborder="0"></iframe>"#;
        let caps = IFRAME_PATTERN.captures(html).unwrap();
        assert_eq!(
            caps.get(1).unwrap().as_str(),
            "//mydaddy.cc/video/97d0145823aeb8edca/"
        );
    }

    #[test]
    fn test_video_link_pattern() {
        let html =
            r#"<a href="/hdporn/81203-full_body_massage.html" class="click-trigger">test</a>"#;
        let caps = VIDEO_LINK_PATTERN.captures(html).unwrap();
        assert_eq!(
            caps.get(1).unwrap().as_str(),
            "/hdporn/81203-full_body_massage.html"
        );
    }
}
