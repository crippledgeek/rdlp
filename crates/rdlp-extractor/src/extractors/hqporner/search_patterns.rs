//! URL builders for HQPorner search and listing pages.

use lazy_regex::{Lazy, Regex, lazy_regex};
use url::form_urlencoded;

/// HQPorner search base URL.
const SEARCH_BASE: &str = "https://hqporner.com/";

/// Pattern to extract the "Next" page URL from pagination.
static NEXT_PAGE_PATTERN: Lazy<Regex> = lazy_regex!(r#"href="([^"]+)"[^>]*class="[^"]*pagi-btn[^"]*">Next"#);

/// Build a search URL.
///
/// # Arguments
/// * `query` - Search keyword string.
/// * `page` - 1-based page number.
pub(crate) fn build_search_url(query: &str, page: u32) -> String {
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    if page <= 1 {
        format!("{SEARCH_BASE}?q={encoded}")
    } else {
        format!("{SEARCH_BASE}?q={encoded}&p={page}")
    }
}

/// Extract the next listing page URL from pagination HTML.
///
/// Parses the pagination links to find the "Next" page href.
pub(crate) fn next_listing_page_url(webpage: &str) -> String {
    NEXT_PAGE_PATTERN
        .captures(webpage)
        .and_then(|c| c.get(1))
        .map(|m| {
            let href = m.as_str();
            if href.starts_with("http") {
                href.to_string()
            } else {
                format!("https://hqporner.com{href}")
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url_page_1() {
        let url = build_search_url("massage", 1);
        assert_eq!(url, "https://hqporner.com/?q=massage");
    }

    #[test]
    fn test_build_search_url_page_2() {
        let url = build_search_url("massage", 2);
        assert_eq!(url, "https://hqporner.com/?q=massage&p=2");
    }

    #[test]
    fn test_build_search_url_encodes_spaces() {
        let url = build_search_url("big tits", 1);
        assert!(url.contains("big+tits") || url.contains("big%20tits"));
    }

    #[test]
    fn test_next_listing_page_url() {
        let html = r#"<a href="/?q=massage&p=3" class="button mobile-pagi pagi-btn">Next</a>"#;
        let next = next_listing_page_url(html);
        assert_eq!(next, "https://hqporner.com/?q=massage&p=3");
    }

    #[test]
    fn test_next_listing_page_url_category() {
        let html = r#"<a href="/category/amateur/3" class="button mobile-hide pagi-btn">Next</a>"#;
        let next = next_listing_page_url(html);
        assert_eq!(next, "https://hqporner.com/category/amateur/3");
    }

    #[test]
    fn test_next_listing_page_url_no_next() {
        let html = r#"<span class="pagi-btn-alt">5</span>"#;
        let next = next_listing_page_url(html);
        assert!(next.is_empty());
    }
}
