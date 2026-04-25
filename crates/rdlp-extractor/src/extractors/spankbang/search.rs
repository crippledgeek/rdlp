//! Search result parsing and `SearchExtractor` implementation for SpankBang.
//!
//! URL shape: `https://spankbang.com/s/<query>/[<page>/]?o=<ordering>`
//! - Page is path-based (NOT query string); page 1 omits the segment
//! - Ordering is query-string: `featured` (default), `new`, `popular`
//! - Spaces in the query become `+`
//! - Cookie `country=US` matches the live extractor

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};

use super::SpankBangExtractor;
use super::patterns;
use crate::base::common::BaseExtractor;

const SPANKBANG_BASE_URL: &str = "https://spankbang.com";
const SPANKBANG_NAME_STR: &str = "SpankBang";

/// Approximate result-cards-per-page on a typical query. Used as a coarse
/// `total_estimate` for the paginated response shape; not authoritative.
const RESULTS_PER_PAGE: u64 = 36;

/// Hard cap on full-search collection.
const MAX_PLAYLIST_SIZE: usize = 500;

/// Delay between paginated requests (ms).
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Build the search URL for the given query and **0-indexed** external page.
///
/// External page `0` → no path segment (page 1 of results).
/// External page `1` → `/2/` (page 2), etc. — SpankBang's paths are 1-indexed.
pub(crate) fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let kw: String = query
        .query
        .chars()
        .map(|c| if c == ' ' { '+' } else { c })
        .collect();

    let ordering = query
        .filters
        .iter()
        .find(|f| f.key == "ordering")
        .map(|f| f.value.as_str())
        .filter(|v| !v.is_empty());

    let path_page = if page == 0 {
        String::new()
    } else {
        format!("{}/", page + 1)
    };

    let qs = match ordering {
        Some(o) => format!("?o={o}"),
        None => String::new(),
    };

    format!("{SPANKBANG_BASE_URL}/s/{kw}/{path_page}{qs}")
}

/// Extract search-result anchors from a SpankBang search-page HTML.
///
/// Each video appears in two anchors per card (image wrapper + title link);
/// only the form carrying `title="..."` is captured by `SEARCH_RESULT`, which
/// already de-duplicates the image-wrapper form. We additionally de-duplicate
/// by video ID across the whole page since some pages echo the same video in
/// `recommended` / `editor's pick` rails alongside the main result grid.
pub(crate) fn parse_results(html: &str) -> Vec<SearchResultPreview> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut results = Vec::new();

    for caps in patterns::SEARCH_RESULT.captures_iter(html) {
        let id = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let slug = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let title = caps.get(3).map(|m| m.as_str().trim()).unwrap_or_default();

        if id.is_empty() || slug.is_empty() {
            continue;
        }
        if !seen.insert(id.to_string()) {
            continue;
        }

        let video_url = format!("{SPANKBANG_BASE_URL}/{id}/video/{slug}");
        results.push(SearchResultPreview {
            video_url,
            title: title.to_string(),
            thumbnail_url: None,
            duration: None,
            uploader: None,
            actors: Vec::new(),
            view_count: None,
            upload_date: None,
        });
    }

    results
}

/// Heuristic: a search page exposes a "next" page when the next page link
/// (`/<query>/<n+1>/`) is referenced anywhere in the rendered HTML, OR when
/// the result grid is "full" (≥ RESULTS_PER_PAGE entries on this page).
fn has_more_pages(html: &str, query: &SearchQuery, page: u32) -> bool {
    let next_path = {
        let kw: String = query
            .query
            .chars()
            .map(|c| if c == ' ' { '+' } else { c })
            .collect();
        format!("/s/{kw}/{}/", page + 2)
    };
    if html.contains(&next_path) {
        return true;
    }
    parse_results(html).len() as u64 >= RESULTS_PER_PAGE
}

#[async_trait]
impl SearchExtractor for SpankBangExtractor {
    fn name(&self) -> &str {
        SPANKBANG_NAME_STR
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "ordering".to_string(),
            display_name: "Ordering".to_string(),
            allowed_values: vec![
                SearchFilterValue {
                    value: "featured".to_string(),
                    label: "Featured (default)".to_string(),
                },
                SearchFilterValue {
                    value: "new".to_string(),
                    label: "Newest".to_string(),
                },
                SearchFilterValue {
                    value: "popular".to_string(),
                    label: "Most popular".to_string(),
                },
            ],
            default: Some("featured".to_string()),
        }]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut page = 0_u32;

        loop {
            let page_url = build_search_url(query, page);
            let sanitized = rdlp_security::sanitize_for_logging(&page_url);
            debug!("[spankbang] fetching search page {}: {sanitized}", page + 1);

            let webpage = BaseExtractor::fetch_webpage_with_headers(
                &page_url,
                &[("Cookie", "country=US")],
                ctx,
            )
            .await?;

            let mut new_this_page = 0usize;
            for r in parse_results(&webpage) {
                let id_seg: String = r
                    .video_url
                    .trim_start_matches(SPANKBANG_BASE_URL)
                    .trim_start_matches('/')
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                if !seen_ids.insert(id_seg) {
                    continue;
                }
                all_results.push(r);
                new_this_page += 1;
                if all_results.len() >= max_results {
                    break;
                }
            }

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if new_this_page == 0 {
                break;
            }

            if !has_more_pages(&webpage, query, page) {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(
            "[spankbang] search complete: {} results across {} page(s)",
            all_results.len(),
            page + 1
        );
        Ok(all_results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let page = query.page.unwrap_or(0);
        let page_url = build_search_url(query, page);

        let webpage = BaseExtractor::fetch_webpage_with_headers(
            &page_url,
            &[("Cookie", "country=US")],
            ctx,
        )
        .await?;

        let results = parse_results(&webpage);
        let more = has_more_pages(&webpage, query, page);

        Ok(SearchPageResponse {
            results,
            page,
            has_more: more,
            total_estimate: Some(RESULTS_PER_PAGE * 100),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    const SEARCH_PAGE: &str = include_str!("tests/spankbang_search_page.html");

    fn make_query(q: &str, filters: Vec<SearchFilter>) -> SearchQuery {
        SearchQuery {
            query: q.to_string(),
            filters,
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn url_composition_default_page1() {
        let q = make_query("blonde", vec![]);
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://spankbang.com/s/blonde/");
    }

    #[test]
    fn url_composition_page_2() {
        let q = make_query("blonde", vec![]);
        let url = build_search_url(&q, 1);
        assert_eq!(url, "https://spankbang.com/s/blonde/2/");
    }

    #[test]
    fn url_composition_with_ordering() {
        let q = make_query(
            "blonde",
            vec![SearchFilter {
                key: "ordering".to_string(),
                value: "new".to_string(),
            }],
        );
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://spankbang.com/s/blonde/?o=new");
    }

    #[test]
    fn url_composition_spaces_become_plus() {
        let q = make_query("two words", vec![]);
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://spankbang.com/s/two+words/");
    }

    #[test]
    fn parses_results_from_fixture() {
        let results = parse_results(SEARCH_PAGE);
        assert!(
            results.len() >= 30,
            "expected ≥ 30 deduped results from a full search page, got {}",
            results.len()
        );

        // Every URL must be on spankbang.com and follow the /<id>/video/<slug> shape.
        for r in &results {
            assert!(
                r.video_url.starts_with("https://spankbang.com/"),
                "unexpected URL: {}",
                r.video_url
            );
            assert!(
                r.video_url.contains("/video/"),
                "URL missing /video/ segment: {}",
                r.video_url
            );
            assert!(!r.title.is_empty(), "result has empty title");
        }

        // De-duplication holds: every video ID appears at most once.
        let mut ids = HashSet::new();
        for r in &results {
            let id = r
                .video_url
                .trim_start_matches("https://spankbang.com/")
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
            assert!(ids.insert(id.clone()), "duplicate id in results: {id}");
        }
    }

    #[test]
    fn supported_filters_advertises_ordering() {
        let ext = SpankBangExtractor::new();
        let filters = ext.supported_filters();
        assert_eq!(filters.len(), 1);
        let ordering = &filters[0];
        assert_eq!(ordering.key, "ordering");
        assert_eq!(ordering.default.as_deref(), Some("featured"));
        let labels: Vec<&str> = ordering
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(labels.contains(&"featured"));
        assert!(labels.contains(&"new"));
        assert!(labels.contains(&"popular"));
    }

    #[test]
    fn name_matches_info_extractor() {
        // Search and InfoExtractor names must agree so the registry's
        // case-insensitive lookup routes correctly.
        let ext = SpankBangExtractor::new();
        assert_eq!(SearchExtractor::name(&ext), "SpankBang");
    }
}
