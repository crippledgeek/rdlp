//! Search result parsing and `SearchExtractor` implementation for XNXX.
//!
//! XNXX uses path-based pagination: `/search/{query}/{page}` where `{page}`
//! is **1-indexed** (page 0 externally → path segment `1`).

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use scraper::{Html, Selector};
use std::time::Duration;

use super::XNXXExtractor;
use crate::base::common::BaseExtractor;

/// Base URL for search requests.
const XNXX_BASE_URL: &str = "https://www.xnxx.com";

/// Number of results per page (XNXX default).
const RESULTS_PER_PAGE: u64 = 36;

/// Delay between paginated requests (ms).
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Maximum results cap for a full search.
const MAX_PLAYLIST_SIZE: usize = 500;

/// Build the search URL for the given query and **0-indexed** external page.
///
/// XNXX uses 1-indexed path segments: external page `0` → path `/1`.
/// Spaces in the keyword become `+`; other characters are left as-is.
///
/// When filter `sort=top` is present, `?top` is appended as a query string.
pub(crate) fn build_search_url(query: &SearchQuery, page: u32) -> String {
    // XNXX is 1-indexed; external page is 0-indexed
    let display_page = page + 1;

    // Replace spaces with `+` for the path segment
    let kw: String = query
        .query
        .chars()
        .map(|c| if c == ' ' { '+' } else { c })
        .collect();

    let has_top_sort = query
        .filters
        .iter()
        .any(|f| f.key == "sort" && f.value == "top");

    if has_top_sort {
        format!("{XNXX_BASE_URL}/search/{kw}/{display_page}?top")
    } else {
        format!("{XNXX_BASE_URL}/search/{kw}/{display_page}")
    }
}

/// Parse search result items from XNXX search page HTML.
///
/// Iterates over `div.thumb-block` containers.  Each container holds:
/// - `div.thumb-inside a[href^="/video-"] img[data-src]` — image thumb
/// - `div.uploader span.name`                           — uploader name (optional)
/// - `div.thumb-under p a[href^="/video-"][title]`      — title + url
/// - `div.thumb-under p.metadata`                       — duration + view count
///
/// The metadata block has the shape:
///
/// ```html
/// <p class="metadata">
///   <span class="right">1.2M <span class="icon-f icf-eye"/><span class="superfluous">98%</span></span>
///   17min
///   <span class="video-hd"><span class="superfluous"> - </span>1080p</span>
/// </p>
/// ```
///
/// Duration is the bare text node after `span.right`; view count is the
/// leading number inside `span.right`. Both are best-effort and the parser
/// accepts cards with the metadata block missing or partially populated.
pub(crate) fn parse_results(html: &str) -> Vec<SearchResultPreview> {
    let doc = Html::parse_document(html);

    // Top-level card is div.thumb-block
    let block_sel = match Selector::parse("div.thumb-block") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    // Thumbnail image inside the thumb pane
    let thumb_img_sel = match Selector::parse(r#"div.thumb-inside a[href^="/video-"] img"#) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    // Title link in the text-under section
    let title_link_sel = match Selector::parse(r#"div.thumb-under a[href^="/video-"]"#) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let uploader_sel = match Selector::parse("div.uploader span.name") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let metadata_sel = match Selector::parse("div.thumb-under p.metadata") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let metadata_right_sel = match Selector::parse("span.right") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();

    for block in doc.select(&block_sel) {
        // Title link (has `title` attribute and href)
        let Some(title_link) = block.select(&title_link_sel).next() else {
            continue;
        };

        let href = match title_link.value().attr("href") {
            Some(h) => h,
            None => continue,
        };

        let video_url = format!("{XNXX_BASE_URL}{href}");

        // Prefer the `title` attribute; fall back to text content
        let title = title_link
            .value()
            .attr("title")
            .map(str::to_string)
            .unwrap_or_else(|| title_link.text().collect::<String>().trim().to_string());

        if title.is_empty() {
            continue;
        }

        // Thumbnail from `data-src` on the img inside the thumb pane
        let thumbnail_url = block
            .select(&thumb_img_sel)
            .next()
            .and_then(|img| img.value().attr("data-src"))
            .map(str::to_string);

        let uploader = block
            .select(&uploader_sel)
            .next()
            .map(|n| n.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        let (duration, view_count) =
            block
                .select(&metadata_sel)
                .next()
                .map_or((None, None), |meta| {
                    let view_count = meta
                        .select(&metadata_right_sel)
                        .next()
                        .and_then(|r| r.text().next())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .and_then(BaseExtractor::parse_human_count);

                    // Duration is a bare text node directly under p.metadata
                    // (e.g. "17min"). Walk the children, skip span.right and
                    // span.video-hd, and take the first non-empty text node.
                    let duration_text =
                        meta.text()
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .find(|t| {
                                // Skip the leading view-count text from span.right
                                // (already captured above) and the trailing
                                // "1080p" from span.video-hd.
                                BaseExtractor::parse_text_duration(t).is_some()
                            });
                    let duration = duration_text
                        .and_then(|t| BaseExtractor::parse_duration(t.trim_end_matches(',')));

                    (duration, view_count)
                });

        results.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration,
            uploader,
            actors: vec![],
            view_count,
            upload_date: None,
        });
    }

    results
}

/// Return `true` when the next page (`page + 2` in 1-indexed display) is
/// referenced in the HTML.
///
/// XNXX pagination links look like `/search/{kw}/{n}`. If the current
/// 0-indexed external page is `page`, the next display page is
/// `page + 2`, so we check for that path segment in the HTML.
pub(crate) fn has_more_pages(html: &str, query: &SearchQuery, page: u32) -> bool {
    let next_display = page + 2; // display pages are 1-indexed
    let kw: String = query
        .query
        .chars()
        .map(|c| if c == ' ' { '+' } else { c })
        .collect();
    let needle = format!("/search/{kw}/{next_display}");
    html.contains(&needle)
}

#[async_trait]
impl SearchExtractor for XNXXExtractor {
    fn name(&self) -> &str {
        XNXX_NAME_STR
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "sort".to_string(),
            display_name: "Sort order".to_string(),
            allowed_values: vec![SearchFilterValue {
                value: "top".to_string(),
                label: "Top rated".to_string(),
            }],
            default: None,
        }]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut page = 0_u32;

        loop {
            let page_url = build_search_url(query, page);
            let sanitized = rdlp_security::sanitize_for_logging(&page_url);
            debug!("[xnxx] Fetching search page {}: {sanitized}", page + 1);

            let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;

            let page_results = parse_results(&webpage);
            if page_results.is_empty() {
                break;
            }

            let more = has_more_pages(&webpage, query, page);
            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if !more {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(
            "[xnxx] Search complete: {} results across {} pages",
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
        // External `page` is 0-indexed; display page in URL is 1-indexed
        let page = query.page.unwrap_or(0);
        let page_url = build_search_url(query, page);

        let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;

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

// The name string must match XNXXExtractor::name()
const XNXX_NAME_STR: &str = "XNXX";

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    fn make_query(q: &str, filters: Vec<SearchFilter>) -> SearchQuery {
        SearchQuery {
            query: q.to_string(),
            filters,
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn url_composition_plain() {
        // External page 0 → display page 1 (1-indexed)
        let q = make_query("amateur", vec![]);
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://www.xnxx.com/search/amateur/1");
    }

    #[test]
    fn url_composition_top_sort() {
        // External page 1 → display page 2
        let q = make_query(
            "amateur",
            vec![SearchFilter {
                key: "sort".to_string(),
                value: "top".to_string(),
            }],
        );
        let url = build_search_url(&q, 1);
        assert_eq!(url, "https://www.xnxx.com/search/amateur/2?top");
    }

    #[test]
    fn selector_sanity_check() {
        let html = r#"<div class="thumb-inside"><div class="thumb"><a href="/video-14abc/foo"><img data-src="https://t.example/x.jpg"/></a></div></div>"#;
        let doc = Html::parse_document(html);
        let thumb_sel = Selector::parse("div.thumb-inside").unwrap();
        let blocks: Vec<_> = doc.select(&thumb_sel).collect();
        assert_eq!(blocks.len(), 1, "sanity: 1 thumb-inside block");
        let a_sel = Selector::parse(r#"a[href^="/video-"]"#).unwrap();
        let links: Vec<_> = blocks[0].select(&a_sel).collect();
        assert_eq!(links.len(), 1, "sanity: 1 link in block");
        let href = links[0].value().attr("href").unwrap();
        assert_eq!(href, "/video-14abc/foo");
    }

    #[test]
    fn parse_results_finds_video_links() {
        const FIXTURE: &str = include_str!("tests/xnxx_search_page.html");
        let results = parse_results(FIXTURE);
        assert!(!results.is_empty(), "should find at least one result");
        // Every result URL should start with the XNXX base URL + /video-
        for r in &results {
            assert!(
                r.video_url.starts_with("https://www.xnxx.com/video-"),
                "unexpected video_url: {}",
                r.video_url
            );
            assert!(!r.title.is_empty(), "title must not be empty");
        }
    }

    /// Regression guard: prior to this fix, every search result had
    /// `duration = None`, `uploader = None`, `view_count = None` because
    /// `parse_results` hardcoded those fields. The fixture page is dense
    /// with `p.metadata` blocks and `div.uploader span.name` markers, so
    /// at least one row must populate each.
    #[test]
    fn parse_results_extracts_metadata_fields() {
        const FIXTURE: &str = include_str!("tests/xnxx_search_page.html");
        let results = parse_results(FIXTURE);
        assert!(!results.is_empty(), "fixture should yield results");

        let with_duration = results.iter().filter(|r| r.duration.is_some()).count();
        let with_uploader = results.iter().filter(|r| r.uploader.is_some()).count();
        let with_views = results.iter().filter(|r| r.view_count.is_some()).count();

        assert!(
            with_duration >= results.len() / 2,
            "expected most rows to carry duration; got {with_duration}/{} \
             — verify p.metadata text-node parsing still works",
            results.len()
        );
        assert!(
            with_uploader > 0,
            "expected at least one row with uploader (the fixture has \
             21+ uploader spans)"
        );
        assert!(
            with_views >= results.len() / 2,
            "expected most rows to carry view_count; got {with_views}/{} \
             — verify span.right text node still parses",
            results.len()
        );

        // Spot-check: durations must be plausible (>= 1s, <= 4h).
        for r in results.iter().filter_map(|r| r.duration) {
            assert!((1.0..=14_400.0).contains(&r), "implausible duration {r}");
        }
    }

    #[test]
    fn url_spaces_become_plus() {
        let q = make_query("college amateur", vec![]);
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://www.xnxx.com/search/college+amateur/1");
    }

    #[test]
    fn has_more_pages_detects_next_page() {
        // Page 0 → next display = 2, so needle = "/search/amateur/2"
        let q = make_query("amateur", vec![]);
        let html_with_next = r#"<a href="/search/amateur/2">Next</a>"#;
        assert!(has_more_pages(html_with_next, &q, 0));

        let html_without_next = r#"<a href="/search/amateur/1">Prev</a>"#;
        assert!(!has_more_pages(html_without_next, &q, 0));
    }
}
