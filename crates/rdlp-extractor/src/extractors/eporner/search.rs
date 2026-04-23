//! EPorner search: `/tag/{kw-hyphen}/{page}/` (1-indexed).
//!
//! Optional path-stackable sort modifiers: `/top-rated/`, `/longest/`.
//! Filter key: `sort` ∈ {`top-rated`, `longest`}.

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, RdlpError, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};
use scraper::{Html, Selector};
use std::sync::LazyLock;

use super::EPornerExtractor;
use crate::base::common::BaseExtractor;

const EPORNER_ROOT: &str = "https://www.eporner.com";

static RESULT_LINK: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href^='/video-']").unwrap());

/// Normalize "Beach Sunset" → "beach-sunset", collapsing runs of non-alnum
/// to single hyphens and trimming edges.
fn keyword_to_tag(kw: &str) -> String {
    let normalized: String = kw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    normalized
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Look up the `sort` filter value from a `SearchQuery`.
fn get_sort(query: &SearchQuery) -> Option<&str> {
    query
        .filters
        .iter()
        .find(|f| f.key == "sort")
        .map(|f| f.value.as_str())
}

fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let tag = keyword_to_tag(&query.query);
    let sort = get_sort(query);
    let display_page = page + 1;
    match sort {
        Some("top-rated") => {
            format!("{EPORNER_ROOT}/tag/{tag}/top-rated/{display_page}/")
        }
        Some("longest") => {
            format!("{EPORNER_ROOT}/tag/{tag}/longest/{display_page}/")
        }
        _ => format!("{EPORNER_ROOT}/tag/{tag}/{display_page}/"),
    }
}

fn parse_results(html: &str) -> Vec<SearchResultPreview> {
    let doc = Html::parse_document(html);
    let img_selector = Selector::parse("img").unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for link in doc.select(&RESULT_LINK) {
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        if !seen.insert(href.to_string()) {
            continue;
        }
        let title = link
            .value()
            .attr("title")
            .map(str::to_string)
            .unwrap_or_default();
        let thumbnail_url = link
            .select(&img_selector)
            .next()
            .and_then(|i| i.value().attr("src").map(str::to_string));
        let video_url = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("{EPORNER_ROOT}{href}")
        };
        out.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration: None,
            uploader: None,
            actors: vec![],
            view_count: None,
            upload_date: None,
        });
    }
    out
}

#[async_trait]
impl SearchExtractor for EPornerExtractor {
    fn name(&self) -> &str {
        "eporner"
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "sort".into(),
            display_name: "Sort order".into(),
            allowed_values: vec![
                SearchFilterValue {
                    value: "top-rated".into(),
                    label: "Top rated".into(),
                },
                SearchFilterValue {
                    value: "longest".into(),
                    label: "Longest".into(),
                },
            ],
            default: None,
        }]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let page_resp = self.search_page(query, ctx).await?;
        Ok(page_resp.results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let page = query.page.unwrap_or(0);
        let url = build_search_url(query, page);
        let html =
            BaseExtractor::fetch_webpage(&url, ctx)
                .await
                .map_err(|e| RdlpError::Network {
                    message: format!("eporner search: fetch failed: {e:#}"),
                    url: Some(url.clone()),
                })?;
        let results = parse_results(&html);
        // Detect if a next page exists by checking for a page+2 link in pagination
        let next_page_str = format!("/{}/", page + 2);
        let has_more = html.contains(&next_page_str);
        Ok(SearchPageResponse {
            results,
            page,
            has_more,
            total_estimate: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    // Fixture recorded live on 2026-04-23 from www.eporner.com/tag/amateur/1/
    const FIXTURE: &str = include_str!("tests/eporner_tag_page.html");

    #[test]
    fn keyword_hyphenation() {
        assert_eq!(keyword_to_tag("Beach Sunset"), "beach-sunset");
        assert_eq!(keyword_to_tag("Amateur"), "amateur");
    }

    #[test]
    fn url_composition_plain_is_1indexed() {
        let q = SearchQuery {
            query: "amateur".into(),
            page: None,
            filters: vec![],
            max_results: None,
        };
        assert_eq!(
            build_search_url(&q, 0),
            "https://www.eporner.com/tag/amateur/1/"
        );
        assert_eq!(
            build_search_url(&q, 1),
            "https://www.eporner.com/tag/amateur/2/"
        );
    }

    #[test]
    fn url_composition_with_sort() {
        let q = SearchQuery {
            query: "amateur".into(),
            page: None,
            filters: vec![SearchFilter {
                key: "sort".into(),
                value: "top-rated".into(),
            }],
            max_results: None,
        };
        assert_eq!(
            build_search_url(&q, 0),
            "https://www.eporner.com/tag/amateur/top-rated/1/"
        );
    }

    #[test]
    fn parse_results_finds_video_links() {
        let results = parse_results(FIXTURE);
        assert!(!results.is_empty(), "Expected search results from tag page fixture");
    }
}
