//! URL builders, regex patterns, and filter descriptors for XTits search.

use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_types::{SearchFilterDescriptor, SearchFilterValue, SearchQuery};
use url::form_urlencoded;

/// Results per page (XTits KVS default).
pub(crate) const RESULTS_PER_PAGE: u64 = 100;

/// Regex to extract video items from KVS AJAX response HTML.
///
/// Captures: (1) video URL, (2) title, (3) thumbnail URL.
pub(crate) static ITEM_PATTERN: Lazy<Regex> = lazy_regex!(
    r#"<a\s+[^>]*class="[^"]*js-open-popup[^"]*"\s+href="([^"]+)"\s+title="([^"]+)"[^>]*\sthumb="([^"]*)""#
);

/// Regex to extract duration from `<span class="label time">`.
pub(crate) static DURATION_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"<span\s+class="label\s+time"[^>]*>\s*(?:<[^>]+>\s*)*(\d+:\d+)\s*</span>"#);

/// Regex to detect the highest page number in pagination links.
pub(crate) static PAGE_NUMBER_PATTERN: Lazy<Regex> = lazy_regex!(r"from_videos\+from_albums:(\d+)");

/// Build the AJAX search URL for a given query and page.
pub(crate) fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let sort_by = resolve_sort_by(query);
    let page_str = format!("{page:02}");
    let encoded_query: String = form_urlencoded::byte_serialize(query.query.as_bytes()).collect();

    format!(
        "https://www.xtits.com/search/?q={encoded_query}&mode=async&function=get_block\
         &block_id=list_videos_videos_list_search_result\
         &sort_by={sort_by}&from_videos={page_str}&from_albums={page_str}"
    )
}

/// Resolve the `sort_by` parameter from query filters.
///
/// Combines `ordering` and `period` filters:
/// - `ordering=newest` → `post_date`
/// - `ordering=rating` + `period=monthly` → `rating_month`
/// - `ordering=mostviewed` + `period=today` → `video_viewed_today`
fn resolve_sort_by(query: &SearchQuery) -> String {
    let ordering = query
        .filters
        .iter()
        .find(|f| f.key == "ordering")
        .map(|f| f.value.as_str())
        .unwrap_or("relevance");

    let base = match ordering {
        "newest" => "post_date",
        "rating" => "rating",
        "mostviewed" => "video_viewed",
        _ => return String::new(), // relevance = empty sort_by
    };

    // Period only applies to rating and mostviewed
    if ordering == "newest" {
        return base.to_string();
    }

    let period = query
        .filters
        .iter()
        .find(|f| f.key == "period")
        .map(|f| f.value.as_str())
        .unwrap_or("alltime");

    match period {
        "monthly" => format!("{base}_month"),
        "weekly" => format!("{base}_week"),
        "today" => format!("{base}_today"),
        _ => base.to_string(), // alltime = no suffix
    }
}

/// Return the filter descriptors for XTits search.
pub(crate) fn search_filter_descriptors() -> Vec<SearchFilterDescriptor> {
    vec![
        SearchFilterDescriptor::new(
            "ordering",
            "Sort By",
            SearchFilterValue::list([
                ("relevance", "Relevance"),
                ("newest", "Newest"),
                ("rating", "Top Rated"),
                ("mostviewed", "Most Viewed"),
            ]),
            Some("relevance"),
        ),
        SearchFilterDescriptor::new(
            "period",
            "Time Period",
            SearchFilterValue::list([
                ("alltime", "All Time"),
                ("monthly", "This Month"),
                ("weekly", "This Week"),
                ("today", "Today"),
            ]),
            Some("alltime"),
        ),
    ]
}

/// Parse a `MM:SS` duration string into seconds.
pub(crate) fn parse_duration(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    match parts.len() {
        2 => {
            let mins: f64 = parts[0].parse().ok()?;
            let secs: f64 = parts[1].parse().ok()?;
            Some(mins * 60.0 + secs)
        }
        3 => {
            let hours: f64 = parts[0].parse().ok()?;
            let mins: f64 = parts[1].parse().ok()?;
            let secs: f64 = parts[2].parse().ok()?;
            Some(hours * 3600.0 + mins * 60.0 + secs)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    #[test]
    fn test_build_search_url_default() {
        let query = SearchQuery {
            query: "amateur".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("q=amateur"));
        assert!(url.contains("mode=async"));
        assert!(url.contains("from_videos=01"));
        assert!(url.contains("sort_by="));
    }

    #[test]
    fn test_build_search_url_page_2() {
        let query = SearchQuery {
            query: "test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 2);
        assert!(url.contains("from_videos=02"));
        assert!(url.contains("from_albums=02"));
    }

    #[test]
    fn test_resolve_sort_by_newest() {
        let query = SearchQuery {
            query: "test".to_string(),
            filters: vec![SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("sort_by=post_date"));
    }

    #[test]
    fn test_resolve_sort_by_rating_monthly() {
        let query = SearchQuery {
            query: "test".to_string(),
            filters: vec![
                SearchFilter {
                    key: "ordering".to_string(),
                    value: "rating".to_string(),
                },
                SearchFilter {
                    key: "period".to_string(),
                    value: "monthly".to_string(),
                },
            ],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("sort_by=rating_month"));
    }

    #[test]
    fn test_resolve_sort_by_mostviewed_today() {
        let query = SearchQuery {
            query: "test".to_string(),
            filters: vec![
                SearchFilter {
                    key: "ordering".to_string(),
                    value: "mostviewed".to_string(),
                },
                SearchFilter {
                    key: "period".to_string(),
                    value: "today".to_string(),
                },
            ],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("sort_by=video_viewed_today"));
    }

    #[test]
    fn test_resolve_sort_by_relevance_ignores_period() {
        let query = SearchQuery {
            query: "test".to_string(),
            filters: vec![
                SearchFilter {
                    key: "ordering".to_string(),
                    value: "relevance".to_string(),
                },
                SearchFilter {
                    key: "period".to_string(),
                    value: "monthly".to_string(),
                },
            ],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("sort_by=&"));
    }

    #[test]
    fn test_parse_duration_mm_ss() {
        assert_eq!(parse_duration("10:35"), Some(635.0));
        assert_eq!(parse_duration("0:30"), Some(30.0));
        assert_eq!(parse_duration("26:47"), Some(1607.0));
    }

    #[test]
    fn test_parse_duration_hh_mm_ss() {
        assert_eq!(parse_duration("1:30:00"), Some(5400.0));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("abc"), None);
    }

    #[test]
    fn test_search_filter_descriptors() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[1].key, "period");
        assert_eq!(filters[0].allowed_values.len(), 4);
        assert_eq!(filters[1].allowed_values.len(), 4);
    }

    #[test]
    fn test_item_pattern_matches() {
        let html = r#"<a class="link js-open-popup" href="https://www.xtits.com/videos/50088/test/" title="Test Video" thumb="https://i.xtits.com/thumb.jpg" vthumb="https://www.xtits.com/get_file/5/abc/50088vthumbs.mp4/">"#;
        let cap = ITEM_PATTERN.captures(html).unwrap();
        assert_eq!(&cap[1], "https://www.xtits.com/videos/50088/test/");
        assert_eq!(&cap[2], "Test Video");
        assert_eq!(&cap[3], "https://i.xtits.com/thumb.jpg");
    }

    #[test]
    fn test_duration_pattern_matches() {
        let html = r#"<span class="label time"><i class="icon-hd"></i>10:35</span>"#;
        let cap = DURATION_PATTERN.captures(html).unwrap();
        assert_eq!(&cap[1], "10:35");
    }

    #[test]
    fn test_page_number_pattern() {
        let cap = PAGE_NUMBER_PATTERN
            .captures("from_videos+from_albums:03")
            .unwrap();
        assert_eq!(&cap[1], "03");
    }

    #[test]
    fn test_url_encodes_query() {
        let query = SearchQuery {
            query: "big tits".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 1);
        assert!(url.contains("q=big+tits") || url.contains("q=big%20tits"));
    }
}
