//! URL and extraction patterns for RedTube
//!
//! Static regex patterns compiled once at first use via `std::sync::LazyLock`.
//! Includes search URL builders and filter descriptors for the JSON API.

use rdlp_types::SearchFilter;
use regex::Regex;
use std::sync::LazyLock;
use url::form_urlencoded;

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

/// Regex to extract video cards from HTML search results.
///
/// Captures:
/// - `url`: Video page URL (href)
/// - `title`: Video title (title attribute)
/// - `thumb`: Thumbnail image URL (src attribute)
/// - `duration`: Duration text (e.g. "12:34")
pub static HTML_VIDEO_CARD_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a[^>]+href="(?P<url>https?://(?:www\.)?redtube\.com/\d+)"[^>]+title="(?P<title>[^"]*)"[^>]*>.*?(?:<img[^>]+src="(?P<thumb>[^"]*)")?.*?(?:<span[^>]*class="[^"]*duration[^"]*"[^>]*>(?P<duration>[\d:]+)</span>)?"#,
    )
    .expect("Valid HTML video card pattern")
});

/// Number of results per API page.
pub(crate) const API_RESULTS_PER_PAGE: u32 = 20;

/// Build the API search URL for the first page.
///
/// Format: `https://api.redtube.com/?data=redtube.Videos.searchVideos&output=json
///          &search={query}&thumbsize=big&{filters}`
pub(crate) fn build_api_search_url(query: &str, filters: &[SearchFilter]) -> String {
    let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let mut url = format!(
        "https://api.redtube.com/?data=redtube.Videos.searchVideos\
         &output=json&search={encoded_query}&thumbsize=big"
    );

    for filter in filters {
        match filter.key.as_str() {
            "ordering" => {
                url.push_str("&ordering=");
                url.push_str(&filter.value);
            }
            "period" => {
                url.push_str("&period=");
                url.push_str(&filter.value);
            }
            "category" => {
                let encoded: String =
                    form_urlencoded::byte_serialize(filter.value.as_bytes()).collect();
                url.push_str("&category=");
                url.push_str(&encoded);
            }
            "tags" => {
                // Tags are comma-separated, each sent as tags[]
                for tag in filter.value.split(',') {
                    let trimmed = tag.trim();
                    if !trimmed.is_empty() {
                        let encoded: String =
                            form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();
                        url.push_str("&tags[]=");
                        url.push_str(&encoded);
                    }
                }
            }
            _ => {} // Unknown filters are silently ignored (validation is done elsewhere)
        }
    }

    url
}

/// Append a page parameter to an existing API search URL.
pub(crate) fn build_api_search_url_page(base_url: &str, page: u32) -> String {
    format!("{base_url}&page={page}")
}

/// Build the API URL for fetching video info by ID.
///
/// Format: `https://api.redtube.com/?data=redtube.Videos.getVideoById
///          &video_id={id}&output=json&thumbsize=all`
pub(crate) fn build_api_video_url(video_id: &str) -> String {
    format!(
        "https://api.redtube.com/?data=redtube.Videos.getVideoById\
         &video_id={video_id}&output=json&thumbsize=all"
    )
}

/// Build the HTML search fallback URL.
///
/// Format: `https://www.redtube.com/?search={query}`
pub(crate) fn build_html_search_url(query: &str) -> String {
    let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    format!("https://www.redtube.com/?search={encoded_query}")
}

// Re-export filter descriptors from the filters submodule
pub use super::filters::search_filter_descriptors;

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

    #[test]
    fn test_build_api_search_url_basic() {
        let url = build_api_search_url("test query", &[]);
        assert!(url.starts_with("https://api.redtube.com/"));
        assert!(url.contains("search=test+query"));
        assert!(url.contains("output=json"));
        assert!(url.contains("thumbsize=big"));
    }

    #[test]
    fn test_build_api_search_url_with_ordering() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "newest".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("ordering=newest"));
    }

    #[test]
    fn test_build_api_search_url_with_tags() {
        let filters = vec![SearchFilter {
            key: "tags".to_string(),
            value: "tag1,tag2".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("tags[]=tag1"));
        assert!(url.contains("tags[]=tag2"));
    }

    #[test]
    fn test_build_api_search_url_with_category() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "Amateur".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("category=Amateur"));
    }

    #[test]
    fn test_build_api_search_url_encodes_special_chars() {
        let url = build_api_search_url("hello world", &[]);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
    }

    #[test]
    fn test_build_api_search_url_page() {
        let base = "https://api.redtube.com/?data=redtube.Videos.searchVideos&output=json&search=test&thumbsize=big";
        let paged = build_api_search_url_page(base, 3);
        assert!(paged.ends_with("&page=3"));
    }

    #[test]
    fn test_build_html_search_url() {
        let url = build_html_search_url("test query");
        assert_eq!(url, "https://www.redtube.com/?search=test+query");
    }

    #[test]
    fn test_build_api_video_url() {
        let url = build_api_video_url("123456");
        assert!(url.starts_with("https://api.redtube.com/"));
        assert!(url.contains("data=redtube.Videos.getVideoById"));
        assert!(url.contains("video_id=123456"));
        assert!(url.contains("output=json"));
        assert!(url.contains("thumbsize=all"));
    }

    #[test]
    fn test_build_api_video_url_different_ids() {
        let url1 = build_api_video_url("1");
        assert!(url1.contains("video_id=1"));

        let url2 = build_api_video_url("99999999");
        assert!(url2.contains("video_id=99999999"));
    }
}
