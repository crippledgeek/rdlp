//! Search-related URL patterns and filter descriptors for xHamster.
//!
//! Contains the search URL pattern, URL builders, and filter descriptor definitions.

use regex::Regex;
use std::sync::LazyLock;

use super::patterns::DOMAINS;

/// URL pattern for xHamster search pages.
///
/// Matches: `https://xhamster.com/search/query-terms`
pub static XHAMSTER_SEARCH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"https?://(?:[^/?#]+\.)?(?:{DOMAINS})/search/[^/?#]+"
    ))
    .expect("Valid xHamster search URL pattern")
});

/// Check if a URL is an xHamster search URL.
#[allow(dead_code)]
pub fn is_search_url(url: &str) -> bool {
    XHAMSTER_SEARCH_PATTERN.is_match(url)
}

/// Build a search URL from a `SearchQuery`.
///
/// Format: `https://xhamster.com/search/{encoded_query}?{filter_params}`
pub fn build_search_url(query: &rdlp_types::SearchQuery) -> String {
    let encoded_query = query.query.split_whitespace().collect::<Vec<_>>().join("+");

    let mut url = format!("https://xhamster.com/search/{encoded_query}");

    let mut first_param = true;
    for filter in &query.filters {
        let sep = if first_param { '?' } else { '&' };
        url.push(sep);
        url.push_str(&filter.key);
        url.push('=');
        url.push_str(&filter.value);
        first_param = false;
    }

    url
}

/// Build a search URL for a specific page number.
pub fn build_search_url_page(query: &rdlp_types::SearchQuery, page: usize) -> String {
    let mut url = build_search_url(query);
    if url.contains('?') {
        url.push_str(&format!("&page={page}"));
    } else {
        url.push_str(&format!("?page={page}"));
    }
    url
}

/// Return the static filter descriptors for xHamster search.
pub fn search_filter_descriptors() -> Vec<rdlp_types::SearchFilterDescriptor> {
    vec![
        rdlp_types::SearchFilterDescriptor {
            key: "quality".to_string(),
            display_name: "Minimum quality".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "720p".to_string(),
                    label: "720p+".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "1080p".to_string(),
                    label: "1080p+".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "2160p".to_string(),
                    label: "4K+".to_string(),
                },
            ],
            default: None,
        },
        rdlp_types::SearchFilterDescriptor {
            key: "sort".to_string(),
            display_name: "Sort by".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "relevance".to_string(),
                    label: "Relevance".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "newest".to_string(),
                    label: "Newest".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "views".to_string(),
                    label: "Most viewed".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "best".to_string(),
                    label: "Top rated".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "longest".to_string(),
                    label: "Longest".to_string(),
                },
            ],
            default: Some("relevance".to_string()),
        },
        rdlp_types::SearchFilterDescriptor {
            key: "orientations".to_string(),
            display_name: "Orientation".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "straight".to_string(),
                    label: "Straight".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "gay".to_string(),
                    label: "Gay".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "shemale".to_string(),
                    label: "Transgender".to_string(),
                },
            ],
            default: None,
        },
        rdlp_types::SearchFilterDescriptor {
            key: "date".to_string(),
            display_name: "Upload date".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "daily".to_string(),
                    label: "Last 24 hours".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "weekly".to_string(),
                    label: "This week".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "monthly".to_string(),
                    label: "This month".to_string(),
                },
            ],
            default: None,
        },
        rdlp_types::SearchFilterDescriptor {
            key: "min-duration".to_string(),
            display_name: "Min duration".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "2".to_string(),
                    label: "2 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "5".to_string(),
                    label: "5 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "10".to_string(),
                    label: "10 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "30".to_string(),
                    label: "30 min".to_string(),
                },
            ],
            default: None,
        },
        rdlp_types::SearchFilterDescriptor {
            key: "max-duration".to_string(),
            display_name: "Max duration".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "2".to_string(),
                    label: "2 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "5".to_string(),
                    label: "5 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "10".to_string(),
                    label: "10 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "30".to_string(),
                    label: "30 min".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "40".to_string(),
                    label: "40+ min".to_string(),
                },
            ],
            default: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_url_detected() {
        assert!(is_search_url("https://xhamster.com/search/amateur"));
        assert!(is_search_url(
            "https://xhamster.com/search/hot+videos?quality=1080p"
        ));
        assert!(is_search_url(
            "https://xhamster.com/search/test?page=2&sort=newest"
        ));
        assert!(is_search_url("https://xhamster.desi/search/query"));
    }

    #[test]
    fn test_non_search_url_rejected() {
        assert!(!is_search_url("https://xhamster.com/videos/test-123"));
        assert!(!is_search_url("https://xhamster.com/users/someone/videos"));
        assert!(!is_search_url("https://xhamster.com/"));
    }

    #[test]
    fn test_build_search_url_basic() {
        let query = rdlp_types::SearchQuery {
            query: "test query".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(url.starts_with("https://xhamster.com/search/"));
        assert!(url.contains("test"));
    }

    #[test]
    fn test_build_search_url_with_filters() {
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![
                rdlp_types::SearchFilter {
                    key: "quality".to_string(),
                    value: "1080p".to_string(),
                },
                rdlp_types::SearchFilter {
                    key: "sort".to_string(),
                    value: "newest".to_string(),
                },
            ],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(url.contains("quality=1080p"));
        assert!(url.contains("sort=newest"));
    }

    #[test]
    fn test_build_search_url_encodes_special_chars() {
        let query = rdlp_types::SearchQuery {
            query: "hello world".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
    }

    #[test]
    fn test_search_filter_descriptors_not_empty() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 6);
        let keys: Vec<&str> = filters.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"quality"));
        assert!(keys.contains(&"sort"));
        assert!(keys.contains(&"orientations"));
        assert!(keys.contains(&"min-duration"));
        assert!(keys.contains(&"max-duration"));
    }

    #[test]
    fn test_search_filter_descriptors_have_values() {
        let filters = search_filter_descriptors();
        for filter in &filters {
            assert!(
                !filter.allowed_values.is_empty(),
                "Filter '{}' has no values",
                filter.key
            );
            assert!(
                !filter.display_name.is_empty(),
                "Filter '{}' has no display name",
                filter.key
            );
        }
    }
}
