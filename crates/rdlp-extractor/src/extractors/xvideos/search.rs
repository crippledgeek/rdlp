//! SearchExtractor implementation for XVideos.
//!
//! XVideos search URL format: `https://www.xvideos.com/?k={query}&p={page}`
//! where `p` is 0-indexed. Adding `&top` sorts by most-viewed.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use scraper::{Html, Selector};

use super::XVideosExtractor;
use crate::base::common::BaseExtractor;

const XVIDEOS_BASE_URL: &str = "https://www.xvideos.com";

/// Build the search URL for a given query and 0-indexed page number.
fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let kw = urlencoding::encode(&query.query);
    let sort_top = query
        .filters
        .iter()
        .any(|f| f.key == "sort" && f.value == "top");

    if sort_top {
        format!("{XVIDEOS_BASE_URL}/?k={kw}&top&p={page}")
    } else {
        format!("{XVIDEOS_BASE_URL}/?k={kw}&p={page}")
    }
}

/// Parse duration text like "11 min", "1 h 20 min", "45 min" into seconds.
fn parse_duration_text(text: &str) -> Option<f64> {
    let text = text.trim();
    // Try "N h M min" pattern first
    if let Some(h_pos) = text.find(" h ") {
        let hours: f64 = text[..h_pos].trim().parse().ok()?;
        let rest = text[h_pos + 3..].trim();
        let mins: f64 = rest.trim_end_matches(" min").trim().parse().ok()?;
        return Some(hours * 3600.0 + mins * 60.0);
    }
    // Try "N min" pattern
    if let Some(stripped) = text.strip_suffix(" min") {
        let mins: f64 = stripped.trim().parse().ok()?;
        return Some(mins * 60.0);
    }
    // Try "N h" alone
    if let Some(stripped) = text.strip_suffix(" h") {
        let hours: f64 = stripped.trim().parse().ok()?;
        return Some(hours * 3600.0);
    }
    None
}

/// Check whether the HTML contains a link to the next page.
fn has_next_page(html: &str, next_page: u32) -> bool {
    // Next page link would contain `p={next_page}` in the URL
    html.contains(&format!("p={next_page}"))
}

/// Parse search result items from XVideos search HTML.
pub(crate) fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let doc = Html::parse_document(html);

    let block_sel = Selector::parse("div.thumb-block").expect("static selector");
    let anchor_sel =
        Selector::parse("div.thumb-inside a[href^='/video.']").expect("static selector");
    let title_sel = Selector::parse("p.title a").expect("static selector");
    let img_sel = Selector::parse("div.thumb-inside img").expect("static selector");
    let dur_sel = Selector::parse(".duration, span.duration").expect("static selector");
    // Uploader / channel / profile link lives inside p.metadata on XVideos search pages.
    let uploader_sel =
        Selector::parse("p.metadata a[href^='/profiles/'] .name, p.metadata a[href^='/channels/'] .name, p.metadata a[href^='/amateur-channels/'] .name, p.metadata a[href^='/pornstar-channels/'] .name")
            .expect("static selector");

    let mut results = Vec::new();

    for block in doc.select(&block_sel) {
        // Extract video URL from the anchor inside thumb-inside
        let video_url = block
            .select(&anchor_sel)
            .next()
            .and_then(|a| a.value().attr("href"))
            .map(|href| {
                if href.starts_with("http") {
                    href.to_string()
                } else {
                    format!("{XVIDEOS_BASE_URL}{href}")
                }
            });

        let Some(video_url) = video_url else {
            continue;
        };

        // Title from p.title a[title] attribute, fall back to text content
        let title = block
            .select(&title_sel)
            .next()
            .and_then(|a| {
                a.value().attr("title").map(|t| t.to_string()).or_else(|| {
                    let t: String = a.text().collect::<String>().trim().to_string();
                    if t.is_empty() { None } else { Some(t) }
                })
            })
            .unwrap_or_else(|| "Untitled".to_string());

        // Thumbnail: XVideos lazy-loads thumbs, so `src` is a placeholder
        // (`assets-cdn77.xvideos-cdn.com/img/lightbox/lightbox-blank.gif`).
        // The real URL lives in `data-src`. Some cards use a `THUMBNUM`
        // template placeholder in `data-src` that is replaced by XVideos'
        // client-side JS (`xv.thumbs.prepareVideo(videoId)`) at render time
        // — we substitute with `1` (the first-frame thumb, universally
        // available). Falls back to `data-mzl` (mosaique listing image)
        // if neither works, then `src` as last resort.
        let thumbnail_url = block.select(&img_sel).next().and_then(|img| {
            let attrs = img.value();
            attrs
                .attr("data-src")
                .or_else(|| attrs.attr("data-mzl"))
                .or_else(|| attrs.attr("src"))
                .filter(|u| !u.contains("lightbox-blank"))
                .map(|u| u.replace("THUMBNUM", "1"))
        });

        // Duration from .duration or span.duration
        let duration = block.select(&dur_sel).next().and_then(|el| {
            let text: String = el.text().collect::<String>();
            parse_duration_text(text.trim())
        });

        // Uploader from the first profile/channel link in p.metadata
        let uploader = block
            .select(&uploader_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        results.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration,
            uploader,
            actors: vec![],
            view_count: None,
            upload_date: None,
        });
    }

    results
}

#[async_trait]
impl SearchExtractor for XVideosExtractor {
    fn name(&self) -> &str {
        "XVideos"
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "sort".to_string(),
            display_name: "Sort".to_string(),
            allowed_values: vec![SearchFilterValue {
                value: "top".to_string(),
                label: "Most Viewed".to_string(),
            }],
            default: None,
        }]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let max_results = query.max_results.unwrap_or(500);
        let mut all_results = Vec::new();
        let mut page = 0_u32;

        loop {
            let page_url = build_search_url(query, page);
            let sanitized = rdlp_security::sanitize_for_logging(&page_url);
            debug!("[XVideos] Fetching search page {page}: {sanitized}");

            let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;
            let page_results = parse_search_results(&webpage);

            if page_results.is_empty() {
                break;
            }

            let next_page = page + 1;
            let more = has_next_page(&webpage, next_page);
            all_results.extend(page_results);

            if all_results.len() >= max_results || !more {
                all_results.truncate(max_results);
                break;
            }

            page = next_page;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        debug!(
            "[XVideos] Search complete: {} results across {} pages",
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
        // page in SearchQuery is 1-indexed per convention; XVideos uses 0-indexed internally
        let page_1indexed = query.page.unwrap_or(1);
        let page_0indexed = page_1indexed.saturating_sub(1);

        let page_url = build_search_url(query, page_0indexed);
        let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;

        let results = parse_search_results(&webpage);
        let has_more = has_next_page(&webpage, page_0indexed + 1) && !results.is_empty();
        let total_estimate = None;

        Ok(SearchPageResponse {
            results,
            page: page_1indexed,
            has_more,
            total_estimate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    const FIXTURE: &str = include_str!("tests/xvideos_search_page.html");

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
        let query = make_query("amateur", vec![]);
        let url = build_search_url(&query, 0);
        assert!(
            url.contains("k=amateur") && url.contains("p=0"),
            "URL should contain k=amateur and p=0, got: {url}"
        );
    }

    #[test]
    fn url_composition_top_sort() {
        let query = make_query(
            "amateur",
            vec![SearchFilter {
                key: "sort".to_string(),
                value: "top".to_string(),
            }],
        );
        let url = build_search_url(&query, 2);
        assert!(
            url.contains("top") && url.contains("p=2"),
            "URL should contain top filter and p=2, got: {url}"
        );
    }

    #[test]
    fn parse_results_finds_video_links() {
        let results = parse_search_results(FIXTURE);
        assert!(!results.is_empty(), "should find at least one result");
        // All results should have absolute URLs starting with https://
        for result in &results {
            assert!(
                result.video_url.starts_with("https://"),
                "video_url should be absolute: {}",
                result.video_url
            );
            assert!(!result.title.is_empty(), "title should not be empty");
        }
    }

    #[test]
    fn parse_duration_formats() {
        assert_eq!(parse_duration_text("11 min"), Some(660.0));
        assert_eq!(parse_duration_text("1 h 20 min"), Some(4800.0));
        assert_eq!(parse_duration_text("45 min"), Some(2700.0));
        assert_eq!(parse_duration_text("2 h"), Some(7200.0));
        assert_eq!(parse_duration_text(""), None);
    }

    #[test]
    fn parse_results_duration_from_fixture() {
        let results = parse_search_results(FIXTURE);
        // First item: "11 min" -> 660s
        assert_eq!(results[0].duration, Some(660.0), "first item duration");
        // Second item: "1 h 20 min" -> 4800s
        assert_eq!(results[1].duration, Some(4800.0), "second item duration");
        // Third item: "45 min" -> 2700s
        assert_eq!(results[2].duration, Some(2700.0), "third item duration");
    }

    /// XVideos sometimes serves a `THUMBNUM` template placeholder in the
    /// `data-src` attribute — meant to be filled in client-side by
    /// `xv.thumbs.prepareVideo()`. Our parser must substitute it with a
    /// concrete number (1) or the CDN returns 404. Regression test for
    /// the bug where the Chase Taylor result in `?k=just+18` had no thumb.
    #[test]
    fn thumbnum_placeholder_is_substituted() {
        let html = r#"
        <div class="thumb-block"><div class="thumb-inside"><div class="thumb">
          <a href="/video.abc123/test">
            <img src="https://assets-cdn77.xvideos-cdn.com/img/lightbox/lightbox-blank.gif"
                 data-src="https://thumb-cdn77.xvideos-cdn.com/uuid/3/xv_THUMBNUM_t.jpg"
                 data-mzl="https://thumb-cdn77.xvideos-cdn.com/uuid/3/mozaique_listing.jpg"/>
          </a>
        </div></div><div class="thumb-under"><p class="title"><a title="Test">Test</a></p></div></div>
        "#;
        let results = parse_search_results(html);
        assert_eq!(results.len(), 1);
        let thumb = results[0].thumbnail_url.as_deref().unwrap_or("");
        assert!(
            !thumb.contains("THUMBNUM"),
            "THUMBNUM must be substituted; got: {thumb}"
        );
        assert!(
            thumb.contains("xv_1_t.jpg"),
            "expected xv_1_t.jpg, got: {thumb}"
        );
    }

    /// The `src` attribute always holds the lazy-load placeholder
    /// `lightbox-blank.gif`; the parser must never return that URL.
    #[test]
    fn never_returns_lightbox_placeholder() {
        let html = r#"
        <div class="thumb-block"><div class="thumb-inside"><div class="thumb">
          <a href="/video.abc/test">
            <img src="https://assets-cdn77.xvideos-cdn.com/img/lightbox/lightbox-blank.gif"/>
          </a>
        </div></div><div class="thumb-under"><p class="title"><a title="T">T</a></p></div></div>
        "#;
        let results = parse_search_results(html);
        assert_eq!(results.len(), 1);
        assert!(
            results[0].thumbnail_url.is_none(),
            "should not expose the lightbox-blank placeholder"
        );
    }
}
