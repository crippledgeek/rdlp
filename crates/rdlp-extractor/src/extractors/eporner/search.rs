//! EPorner search: `/tag/{kw-hyphen}/{page}/` (1-indexed).
//!
//! Optional path-stackable sort modifiers: `/top-rated/`, `/longest/`.
//! Filter key: `sort` ∈ {`top-rated`, `longest`}.

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, RdlpError, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use scraper::{Html, Selector};
use std::sync::LazyLock;

use super::EPornerExtractor;
use crate::base::common::BaseExtractor;

const EPORNER_ROOT: &str = "https://www.eporner.com";

static RESULT_LINK: LazyLock<Selector> = crate::static_selector!("a[href^='/video-']");
static MBCONTENT_SEL: LazyLock<Selector> = crate::static_selector!("div.mbcontent");
static MBTIT_LINK_SEL: LazyLock<Selector> = crate::static_selector!("p.mbtit a[href^='/video-']");
static MBTIM_SEL: LazyLock<Selector> = crate::static_selector!("span.mbtim");
static MBVIE_SEL: LazyLock<Selector> = crate::static_selector!("span.mbvie");
static MB_UPLOADER_SEL: LazyLock<Selector> = crate::static_selector!("span.mb-uploader a");
static MBIMG_SEL: LazyLock<Selector> = crate::static_selector!("img");

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

/// Parse EPorner search/tag results.
///
/// Each card is a `div.mbcontent` (thumbnail + cover anchor) immediately
/// followed by a sibling `div.mbunder` containing structured metadata:
///
/// ```html
/// <p class="mbtit"><a href="/video-…">Title</a></p>
/// <p class="mbstats">
///   <span class="mbtim" title="Duration">14:57</span>
///   <span class="mbrate" title="Rating">85%</span>
///   <span class="mbvie" title="Views">364,017</span>
///   <span class="mb-uploader"><a href="/profile/X/" title="Uploader">X</a></span>
/// </p>
/// ```
///
/// Anchor inside `div.mbcontent` is the cover; the title text lives in
/// `p.mbtit a` of the sibling `div.mbunder`. Walk both via the shared
/// outer `div.mb` parent — which the existing fixture and live page both
/// expose. As a fallback for pages where structured markup is absent,
/// keep the legacy permissive selector path so the basic href harvest
/// still works.
fn parse_results(html: &str) -> Vec<SearchResultPreview> {
    let doc = Html::parse_document(html);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    // Structured path: walk each `div.mbcontent` cover anchor and pair it
    // with the sibling metadata block via a shared video URL.
    for cover in doc.select(&MBCONTENT_SEL) {
        let Some(cover_a) = cover.select(&RESULT_LINK).next() else {
            continue;
        };
        let Some(href) = cover_a.value().attr("href") else {
            continue;
        };
        if !seen.insert(href.to_string()) {
            continue;
        }

        let video_url = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("{EPORNER_ROOT}{href}")
        };

        let thumbnail_url = cover_a
            .select(&MBIMG_SEL)
            .next()
            .and_then(|i| i.value().attr("src").map(str::to_string));

        // Find the matching mbunder by scanning forward from the parent of
        // mbcontent until the next mbtit anchor pointing at the same href.
        // In practice the structured pages place mbcontent and mbunder as
        // direct siblings inside a div.mb wrapper, so a document-wide scan
        // for `p.mbtit a[href=…]` plus its nearest stats ancestor is the
        // simplest reliable path.
        let mut title = cover_a
            .select(&MBIMG_SEL)
            .next()
            .and_then(|i| i.value().attr("alt"))
            .map(str::to_string)
            .unwrap_or_default();
        let mut duration = None;
        let mut view_count = None;
        let mut uploader = None;

        for tit_a in doc.select(&MBTIT_LINK_SEL) {
            if tit_a.value().attr("href") != Some(href) {
                continue;
            }
            // Title text: prefer the anchor's text content; fall back to
            // the cover img's alt attribute.
            let txt: String = tit_a.text().collect::<String>().trim().to_string();
            if !txt.is_empty() {
                title = txt;
            }
            // Walk up to mbunder, then descend to mbstats children.
            if let Some(mbunder) = tit_a
                .ancestors()
                .filter_map(scraper::ElementRef::wrap)
                .find(|e| {
                    e.value()
                        .has_class("mbunder", scraper::CaseSensitivity::CaseSensitive)
                })
            {
                duration = mbunder
                    .select(&MBTIM_SEL)
                    .next()
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .as_deref()
                    .and_then(BaseExtractor::parse_duration);
                view_count = mbunder
                    .select(&MBVIE_SEL)
                    .next()
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .as_deref()
                    .and_then(BaseExtractor::parse_human_count);
                uploader = mbunder
                    .select(&MB_UPLOADER_SEL)
                    .next()
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .filter(|s| !s.is_empty());
            }
            break;
        }

        out.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration,
            uploader,
            uploader_url: None,
            actors: vec![],
            view_count,
            upload_date: None,
        });
    }

    if !out.is_empty() {
        return out;
    }

    // Fallback: permissive selector for pages that omit the mb* structure
    // (older snapshots / partial captures). Title-only.
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
            .select(&MBIMG_SEL)
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
            uploader_url: None,
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
        "EPorner"
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
        assert!(
            !results.is_empty(),
            "Expected search results from tag page fixture"
        );
    }

    /// Regression: prior to this fix every result had `duration / uploader /
    /// view_count = None` because `parse_results` only used the permissive
    /// `a[href^='/video-']` selector. The fixture (recorded 2026-04-23) is
    /// dense with `span.mbtim` / `span.mbvie` / `span.mb-uploader` markers,
    /// so most rows must populate each field.
    #[test]
    fn parse_results_extracts_metadata_fields() {
        let results = parse_results(FIXTURE);
        assert!(!results.is_empty(), "fixture should yield results");

        let with_duration = results.iter().filter(|r| r.duration.is_some()).count();
        let with_views = results.iter().filter(|r| r.view_count.is_some()).count();
        let with_uploader = results.iter().filter(|r| r.uploader.is_some()).count();

        assert!(
            with_duration >= results.len() / 2,
            "expected most rows to carry duration; got {with_duration}/{}",
            results.len()
        );
        assert!(
            with_views >= results.len() / 2,
            "expected most rows to carry view_count; got {with_views}/{}",
            results.len()
        );
        // Uploader is sometimes absent (e.g. for studio-uploaded content);
        // we just need at least one row to prove the selector works.
        assert!(with_uploader > 0, "expected at least one row with uploader");
    }
}
