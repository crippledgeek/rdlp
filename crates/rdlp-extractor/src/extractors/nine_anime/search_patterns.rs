//! URL builders, regex patterns, and filter descriptors for NineAnime search.

use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_types::{SearchFilterDescriptor, SearchFilterValue, SearchQuery};

/// Base URL for 9anime.
const BASE_URL: &str = "https://9animetv.to";

/// Approximate results per page.
pub(crate) const RESULTS_PER_PAGE: u64 = 30;

/// Extract film-name links from 9anime search results.
/// Captures: (1) href, (2) title.
pub(crate) static FILM_NAME_PATTERN: Lazy<Regex> = lazy_regex!(
    r#"<h3\s+class="film-name">\s*<a\s+href="([^"]+)"\s+title="([^"]+)"[^>]*class="dynamic-name"#
);

/// Extract thumbnail from img with data-src.
/// Captures: (1) thumbnail URL, (2) alt text.
pub(crate) static THUMBNAIL_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"<img\s+data-src="([^"]+)"[^>]*class="film-poster-img[^"]*"[^>]*alt="([^"]+)""#);

/// Extract episode count text from tick-eps div.
/// Captures: (1) episode text like "Ep 34/34" or "Ep Full".
pub(crate) static EPISODE_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"<div\s+class="tick-item\s+tick-eps">\s*([^<]+?)\s*</div>"#);

/// Detect "Next" pagination link.
pub(crate) static NEXT_PAGE_PATTERN: Lazy<Regex> = lazy_regex!(r#">Next\s*<"#);

/// Extract total page count from "of {N}" text.
/// Captures: (1) total pages number.
pub(crate) static TOTAL_PAGES_PATTERN: Lazy<Regex> = lazy_regex!(r#"of\s+(\d+)"#);

/// Build the search URL for a given query and page.
///
/// Page numbers are 0-based: page 0 = first page.
pub(crate) fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let sort = resolve_sort(query);
    let encoded = urlencoding::encode(&query.query);

    if sort.is_empty() {
        format!("{BASE_URL}/search?keyword={encoded}&page={page}")
    } else {
        format!("{BASE_URL}/search?keyword={encoded}&sort={sort}&page={page}")
    }
}

/// Resolve the `sort` URL parameter from query filters.
fn resolve_sort(query: &SearchQuery) -> &str {
    let ordering = query
        .filters
        .iter()
        .find(|f| f.key == "ordering")
        .map(|f| f.value.as_str())
        .unwrap_or("default");

    match ordering {
        "updated" => "recently-updated",
        "added" => "recently-added",
        "name" => "name-az",
        "mostwatched" => "most-watched",
        "score" => "score",
        "released" => "released-date",
        _ => "", // default = no sort param
    }
}

/// Return filter descriptors for NineAnime search.
pub(crate) fn search_filter_descriptors() -> Vec<SearchFilterDescriptor> {
    vec![SearchFilterDescriptor::new(
        "ordering",
        "Sort By",
        SearchFilterValue::list([
            ("default", "Default"),
            ("updated", "Recently Updated"),
            ("added", "Recently Added"),
            ("name", "Name A-Z"),
            ("mostwatched", "Most Watched"),
            ("score", "Score"),
            ("released", "Released Date"),
        ]),
        Some("default"),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    #[test]
    fn test_build_search_url_default() {
        let query = SearchQuery {
            query: "naruto".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 0);
        assert_eq!(url, "https://9animetv.to/search?keyword=naruto&page=0");
    }

    #[test]
    fn test_build_search_url_page_2() {
        let query = SearchQuery {
            query: "naruto".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 2);
        assert!(url.contains("page=2"));
    }

    #[test]
    fn test_build_search_url_with_sort() {
        let query = SearchQuery {
            query: "sailor moon".to_string(),
            filters: vec![SearchFilter {
                key: "ordering".to_string(),
                value: "mostwatched".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 0);
        assert!(url.contains("sort=most-watched"));
        assert!(url.contains("keyword=sailor%20moon"));
    }

    #[test]
    fn test_resolve_sort_all_values() {
        let make_query = |val: &str| SearchQuery {
            query: "test".to_string(),
            filters: vec![SearchFilter {
                key: "ordering".to_string(),
                value: val.to_string(),
            }],
            max_results: None,
            page: None,
        };

        assert_eq!(resolve_sort(&make_query("default")), "");
        assert_eq!(resolve_sort(&make_query("updated")), "recently-updated");
        assert_eq!(resolve_sort(&make_query("added")), "recently-added");
        assert_eq!(resolve_sort(&make_query("name")), "name-az");
        assert_eq!(resolve_sort(&make_query("mostwatched")), "most-watched");
        assert_eq!(resolve_sort(&make_query("score")), "score");
        assert_eq!(resolve_sort(&make_query("released")), "released-date");
    }

    #[test]
    fn test_search_filter_descriptors() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[0].allowed_values.len(), 7);
    }

    #[test]
    fn test_film_name_pattern() {
        let html = r#"<h3 class="film-name"><a href="/watch/sailor-moon-sailor-stars-643" title="Sailor Moon: Sailor Stars" class="dynamic-name" data-jname="Test">Sailor Moon: Sailor Stars</a></h3>"#;
        let cap = FILM_NAME_PATTERN.captures(html).unwrap();
        assert_eq!(&cap[1], "/watch/sailor-moon-sailor-stars-643");
        assert_eq!(&cap[2], "Sailor Moon: Sailor Stars");
    }

    #[test]
    fn test_thumbnail_pattern() {
        let html = r#"<img data-src="https://cdn.noitatnemucod.net/thumbnail/300x400/100/abc.jpg" class="film-poster-img lazyload" alt="Sailor Moon: Sailor Stars">"#;
        let cap = THUMBNAIL_PATTERN.captures(html).unwrap();
        assert_eq!(
            &cap[1],
            "https://cdn.noitatnemucod.net/thumbnail/300x400/100/abc.jpg"
        );
        assert_eq!(&cap[2], "Sailor Moon: Sailor Stars");
    }

    #[test]
    fn test_episode_pattern() {
        let html = r#"<div class="tick-item tick-eps">Ep 34/34</div>"#;
        let cap = EPISODE_PATTERN.captures(html).unwrap();
        assert_eq!(cap[1].trim(), "Ep 34/34");
    }

    #[test]
    fn test_episode_pattern_full() {
        let html = r#"<div class="tick-item tick-eps">
                    Ep Full
                </div>"#;
        let cap = EPISODE_PATTERN.captures(html).unwrap();
        assert_eq!(cap[1].trim(), "Ep Full");
    }

    #[test]
    fn test_next_page_pattern() {
        assert!(NEXT_PAGE_PATTERN.is_match(">Next <"));
        assert!(NEXT_PAGE_PATTERN.is_match(">Next<"));
        assert!(!NEXT_PAGE_PATTERN.is_match("Previous"));
    }

    #[test]
    fn test_total_pages_pattern() {
        let html = r#"<div class="btn btn-sm btn-blank">of 3</div>"#;
        let cap = TOTAL_PAGES_PATTERN.captures(html).unwrap();
        assert_eq!(&cap[1], "3");
    }

    #[test]
    fn test_url_encodes_query() {
        let query = SearchQuery {
            query: "one piece".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query, 0);
        assert!(url.contains("keyword=one%20piece"));
    }
}
