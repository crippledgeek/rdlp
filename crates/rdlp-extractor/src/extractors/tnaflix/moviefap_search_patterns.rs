//! Search URL patterns and filter descriptors for MovieFap.
//!
//! Contains the URL builder and filter descriptor definitions used by
//! `MovieFapSearchExtractor`.

/// MovieFap base URL.
pub const MOVIEFAP_BASE_URL: &str = "https://www.moviefap.com";

/// Valid sort options for MovieFap search.
pub const VALID_ORDERINGS: &[&str] = &["relevance", "adddate", "viewnum", "rate", "duration"];

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
    vec![rdlp_types::SearchFilterDescriptor {
        key: "ordering".to_string(),
        display_name: "Sort by".to_string(),
        allowed_values: vec![
            rdlp_types::SearchFilterValue {
                value: "relevance".to_string(),
                label: "Relevance".to_string(),
            },
            rdlp_types::SearchFilterValue {
                value: "adddate".to_string(),
                label: "Newest".to_string(),
            },
            rdlp_types::SearchFilterValue {
                value: "viewnum".to_string(),
                label: "Most Viewed".to_string(),
            },
            rdlp_types::SearchFilterValue {
                value: "rate".to_string(),
                label: "Top Rated".to_string(),
            },
            rdlp_types::SearchFilterValue {
                value: "duration".to_string(),
                label: "Duration".to_string(),
            },
        ],
        default: Some("relevance".to_string()),
    }]
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
}
