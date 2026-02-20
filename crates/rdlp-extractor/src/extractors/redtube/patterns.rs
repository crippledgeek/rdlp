//! URL and extraction patterns for RedTube
//!
//! Static regex patterns compiled once at first use via `std::sync::LazyLock`.

use regex::Regex;
use std::sync::LazyLock;

/// Static URL pattern regex for RedTube (initialized once at first use)
///
/// Supports:
/// - Standard URLs: https://www.redtube.com/123456
/// - Brazilian domain: https://www.redtube.com.br/123456
/// - Embed URLs: https://embed.redtube.com/?id=123456
pub static REDTUBE_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?:(?:\w+\.)?redtube\.com(?:\.br)?/|embed\.redtube\.com/\?.*\bid=)(?P<id>\d+)",
    )
    .expect("Valid RedTube URL pattern")
});

/// Regex to extract JavaScript sources object: sources: {"720": "url", ...}
pub static SOURCES_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"sources\s*:\s*(\{[^}]+\})"#).expect("Valid sources pattern"));

/// Regex to extract mediaDefinition array: mediaDefinition: [{...}, ...]
pub static MEDIA_DEF_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)mediaDefinition\s*:\s*(\[.+?\])").expect("Valid mediaDefinition pattern")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_pattern_standard() {
        assert!(REDTUBE_URL_PATTERN.is_match("https://www.redtube.com/123456"));
        assert!(REDTUBE_URL_PATTERN.is_match("https://redtube.com/12345678"));
    }

    #[test]
    fn test_url_pattern_brazilian() {
        assert!(REDTUBE_URL_PATTERN.is_match("https://www.redtube.com.br/987654"));
    }

    #[test]
    fn test_url_pattern_embed() {
        assert!(REDTUBE_URL_PATTERN.is_match("https://embed.redtube.com/?id=123456"));
    }

    #[test]
    fn test_url_pattern_invalid() {
        assert!(!REDTUBE_URL_PATTERN.is_match("https://youtube.com/watch?v=test"));
        assert!(!REDTUBE_URL_PATTERN.is_match("https://www.tnaflix.com/video/123"));
    }

    #[test]
    fn test_sources_pattern() {
        let webpage = r#"sources: {"720": "url1", "1080": "url2"}"#;
        assert!(SOURCES_PATTERN.is_match(webpage));

        let caps = SOURCES_PATTERN.captures(webpage).unwrap();
        assert!(caps.get(1).unwrap().as_str().contains("720"));
    }

    #[test]
    fn test_media_def_pattern() {
        let webpage = r#"mediaDefinition: [{"videoUrl": "test"}]"#;
        assert!(MEDIA_DEF_PATTERN.is_match(webpage));
    }
}
