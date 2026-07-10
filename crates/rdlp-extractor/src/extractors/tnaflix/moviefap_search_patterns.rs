//! Search URL patterns and filter descriptors for MovieFap.
//!
//! Contains the URL builder and filter descriptor definitions used by
//! `MovieFapSearchExtractor`.

/// MovieFap base URL.
pub const MOVIEFAP_BASE_URL: &str = "https://www.moviefap.com";

/// Build a MovieFap search URL for a specific page.
///
/// Format: `https://www.moviefap.com/search/{encoded_query}/{sort}/{page}`
///
/// # Arguments
/// * `query` - The search query with optional filters.
/// * `page` - 1-based page number.
///
/// # Returns
/// A fully-formed MovieFap search URL string for the given page.
pub fn build_search_url(query: &rdlp_types::SearchQuery, page: usize) -> String {
    let encoded_query = query.query.split_whitespace().collect::<Vec<_>>().join("+");

    let sort = query
        .filters
        .iter()
        .find(|f| f.key == "ordering")
        .map(|f| f.value.as_str())
        .unwrap_or("relevance");

    format!("{MOVIEFAP_BASE_URL}/search/{encoded_query}/{sort}/{page}")
}

/// Return the static filter descriptors for MovieFap search.
///
/// # Supported Filters
/// - `ordering` - Sort order (relevance, adddate, viewnum, rate, duration)
pub fn search_filter_descriptors() -> Vec<rdlp_types::SearchFilterDescriptor> {
    vec![rdlp_types::SearchFilterDescriptor::new(
        "ordering",
        "Sort by",
        rdlp_types::SearchFilterValue::list([
            ("relevance", "Relevance"),
            ("adddate", "Newest"),
            ("viewnum", "Most Viewed"),
            ("rate", "Top Rated"),
            ("duration", "Duration"),
        ]),
        Some("relevance"),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_query(query: &str, ordering: Option<&str>) -> rdlp_types::SearchQuery {
        let filters = ordering
            .map(|o| {
                vec![rdlp_types::SearchFilter {
                    key: "ordering".to_string(),
                    value: o.to_string(),
                }]
            })
            .unwrap_or_default();

        rdlp_types::SearchQuery {
            query: query.to_string(),
            filters,
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn test_build_search_url_basic() {
        let query = make_query("test query", None);
        let url = build_search_url(&query, 1);
        assert_eq!(
            url,
            "https://www.moviefap.com/search/test+query/relevance/1"
        );
    }

    #[test]
    fn test_build_search_url_with_ordering() {
        let query = make_query("test", Some("adddate"));
        let url = build_search_url(&query, 1);
        assert_eq!(url, "https://www.moviefap.com/search/test/adddate/1");
    }

    #[test]
    fn test_build_search_url_page_2() {
        let query = make_query("hello world", None);
        let url = build_search_url(&query, 2);
        assert_eq!(
            url,
            "https://www.moviefap.com/search/hello+world/relevance/2"
        );
    }

    #[test]
    fn test_build_search_url_no_raw_spaces() {
        let query = make_query("hello world test", None);
        let url = build_search_url(&query, 1);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
        assert!(url.contains("hello+world+test"));
    }

    #[test]
    fn test_build_search_url_contains_moviefap_domain() {
        let query = make_query("test", None);
        let url = build_search_url(&query, 1);
        assert!(url.contains("moviefap.com"), "URL must use moviefap.com");
        assert!(!url.contains("tnaflix.com"), "URL must NOT use tnaflix.com");
        assert!(!url.contains("empflix.com"), "URL must NOT use empflix.com");
    }

    #[test]
    fn test_search_filter_descriptors_not_empty() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].key, "ordering");
    }

    #[test]
    fn test_search_filter_descriptors_values() {
        let filters = search_filter_descriptors();
        let values: Vec<&str> = filters[0]
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"relevance"));
        assert!(values.contains(&"adddate"));
        assert!(values.contains(&"viewnum"));
        assert!(values.contains(&"rate"));
        assert!(values.contains(&"duration"));
    }

    #[test]
    fn test_search_filter_descriptors_default() {
        let filters = search_filter_descriptors();
        assert_eq!(filters[0].default, Some("relevance".to_string()));
    }

    #[test]
    fn test_search_filter_descriptors_five_values() {
        let filters = search_filter_descriptors();
        assert_eq!(filters[0].allowed_values.len(), 5);
    }

    // ---- Negative tests ----

    #[test]
    fn test_build_search_url_empty_query() {
        let query = make_query("", None);
        let url = build_search_url(&query, 1);
        // Empty query → empty segment between /search/ and /relevance/
        assert!(url.contains("/search/"));
        assert!(url.contains("/relevance/1"));
    }

    #[test]
    fn test_build_search_url_special_chars_in_query() {
        let query = make_query("test&foo=bar", None);
        let url = build_search_url(&query, 1);
        // Special chars are not URL-encoded (matches yt-dlp behavior)
        assert!(url.contains("test&foo=bar"));
    }

    #[test]
    fn test_build_search_url_page_zero() {
        let query = make_query("test", None);
        let url = build_search_url(&query, 0);
        assert!(url.ends_with("/0"));
    }

    #[test]
    fn test_build_search_url_page_large() {
        let query = make_query("test", None);
        let url = build_search_url(&query, 99999);
        assert!(url.ends_with("/99999"));
    }

    #[test]
    fn test_build_search_url_invalid_ordering_still_builds() {
        // URL builder doesn't validate ordering — validation is separate
        let query = make_query("test", Some("invalid_sort"));
        let url = build_search_url(&query, 1);
        assert!(url.contains("/invalid_sort/"));
    }

    #[test]
    fn test_build_search_url_whitespace_only_query() {
        let query = make_query("   ", None);
        let url = build_search_url(&query, 1);
        // Whitespace-only splits to empty segments, joined is empty
        assert!(url.contains("/search//"));
    }

    #[test]
    fn test_build_search_url_duplicate_ordering_uses_first() {
        // Two ordering filters — .find() returns the first one
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![
                rdlp_types::SearchFilter {
                    key: "ordering".to_string(),
                    value: "adddate".to_string(),
                },
                rdlp_types::SearchFilter {
                    key: "ordering".to_string(),
                    value: "rate".to_string(),
                },
            ],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("/adddate/")); // first match wins
        assert!(!url.contains("/rate/"));
    }

    #[test]
    fn test_build_search_url_tabs_and_newlines_in_query() {
        let query = make_query("hello\tworld\nfoo", None);
        let url = build_search_url(&query, 1);
        // split_whitespace splits on tabs/newlines too
        assert!(url.contains("hello+world+foo"));
    }
}
