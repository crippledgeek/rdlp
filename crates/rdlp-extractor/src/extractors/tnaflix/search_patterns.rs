//! Search URL patterns and filter descriptors for TNAFlix.
//!
//! Contains the search URL builder, page URL builder, and filter descriptor
//! definitions used by `TNAFlixSearchExtractor`.

/// Build a TNAFlix search URL from a [`SearchQuery`](rdlp_core::SearchQuery).
///
/// Format: `https://www.tnaflix.com/search?what={encoded_query}&tab=`
///
/// Filters are appended as additional query parameters.
///
/// # Arguments
/// * `query` - The search query with optional filters.
///
/// # Returns
/// A fully-formed search URL string.
pub fn build_search_url(query: &rdlp_core::SearchQuery) -> String {
    let encoded_query = query.query.split_whitespace().collect::<Vec<_>>().join("+");

    let mut url = format!("https://www.tnaflix.com/search?what={encoded_query}&tab=");

    for filter in &query.filters {
        url.push('&');
        url.push_str(&filter.key);
        url.push('=');
        url.push_str(&filter.value);
    }

    url
}

/// Build a TNAFlix search URL for a specific page number.
///
/// Appends `&page={page}` to the base search URL.
///
/// # Arguments
/// * `query` - The search query with optional filters.
/// * `page` - 1-based page number.
///
/// # Returns
/// A fully-formed search URL string for the given page.
pub fn build_search_url_page(query: &rdlp_core::SearchQuery, page: usize) -> String {
    let mut url = build_search_url(query);
    url.push_str(&format!("&page={page}"));
    url
}

/// Return the static filter descriptors for TNAFlix search.
///
/// # Supported Filters
/// - `ordering` - Sort order (featured, newest, duration, rating)
pub fn search_filter_descriptors() -> Vec<rdlp_core::SearchFilterDescriptor> {
    vec![rdlp_core::SearchFilterDescriptor {
        key: "ordering".to_string(),
        display_name: "Sort by".to_string(),
        allowed_values: vec![
            rdlp_core::SearchFilterValue {
                value: "featured".to_string(),
                label: "Featured".to_string(),
            },
            rdlp_core::SearchFilterValue {
                value: "newest".to_string(),
                label: "Newest".to_string(),
            },
            rdlp_core::SearchFilterValue {
                value: "duration".to_string(),
                label: "Longest".to_string(),
            },
            rdlp_core::SearchFilterValue {
                value: "rating".to_string(),
                label: "Top rated".to_string(),
            },
        ],
        default: Some("featured".to_string()),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url_basic() {
        let query = rdlp_core::SearchQuery {
            query: "test query".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert_eq!(url, "https://www.tnaflix.com/search?what=test+query&tab=");
    }

    #[test]
    fn test_build_search_url_with_filter() {
        let query = rdlp_core::SearchQuery {
            query: "test".to_string(),
            filters: vec![rdlp_core::SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(url.contains("what=test"));
        assert!(url.contains("ordering=newest"));
    }

    #[test]
    fn test_build_search_url_no_raw_spaces() {
        let query = rdlp_core::SearchQuery {
            query: "hello world test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
        assert!(url.contains("hello+world+test"));
    }

    #[test]
    fn test_build_search_url_page() {
        let query = rdlp_core::SearchQuery {
            query: "test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url_page(&query, 3);
        assert!(url.contains("&page=3"));
    }

    #[test]
    fn test_build_search_url_page_with_filter() {
        let query = rdlp_core::SearchQuery {
            query: "test".to_string(),
            filters: vec![rdlp_core::SearchFilter {
                key: "ordering".to_string(),
                value: "rating".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = build_search_url_page(&query, 5);
        assert!(url.contains("ordering=rating"));
        assert!(url.contains("&page=5"));
    }

    #[test]
    fn test_search_filter_descriptors_not_empty() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[0].allowed_values.len(), 4);
    }

    #[test]
    fn test_search_filter_descriptors_have_default() {
        let filters = search_filter_descriptors();
        assert_eq!(filters[0].default, Some("featured".to_string()));
    }

    #[test]
    fn test_search_filter_descriptors_values() {
        let filters = search_filter_descriptors();
        let values: Vec<&str> = filters[0]
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"featured"));
        assert!(values.contains(&"newest"));
        assert!(values.contains(&"duration"));
        assert!(values.contains(&"rating"));
    }
}
