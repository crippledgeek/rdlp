//! EPorner search: `/tag/{kw-hyphen}/{page}/` (1-indexed).
//!
//! Optional path-stackable sort modifiers: `/top-rated/`, `/longest/`.
//! Filter key: `sort` ∈ {`top-rated`, `longest`}.

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use scraper::{Html, Selector};
use std::sync::LazyLock;

use super::EPornerExtractor;
use crate::base::common::{
    BaseExtractor, PagedSearch, SearchPage, SearchPageSpec, resolve_card_url, resolve_media_url,
};

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

fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let tag = keyword_to_tag(&query.query);
    let sort = crate::base::common::filter_value(&query.filters, "sort");
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

/// The usable poster reference on an eporner card `img`.
///
/// 101 of the 125 cards in the committed capture are lazy-loaded:
/// `<img class="lazyimg" src="data:image/gif;base64,…1x1…"
/// data-src="https://static-eu-cdn.eporner.com/thumbs/…">`. Reading `src`
/// alone therefore yielded a blank transparent pixel for 81% of results —
/// and a `data:` URI is precisely what must not reach the desktop's
/// `<img src>`. So the placeholder is skipped in favour of `data-src`, which
/// is where the real poster lives; the 24 eagerly-loaded cards still answer
/// from `src`.
fn card_poster_src(img: scraper::ElementRef<'_>) -> Option<&str> {
    img.value()
        .attr("src")
        .filter(|s| !s.is_empty() && !s.starts_with("data:"))
        .or_else(|| img.value().attr("data-src"))
        .filter(|s| !s.is_empty())
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

        let Some(video_url) = resolve_card_url(EPORNER_ROOT, href) else {
            continue;
        };

        let thumbnail_url = cover_a
            .select(&MBIMG_SEL)
            .next()
            .and_then(card_poster_src)
            .and_then(|src| resolve_media_url(EPORNER_ROOT, src));

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
            .and_then(card_poster_src)
            .and_then(|src| resolve_media_url(EPORNER_ROOT, src));
        let Some(video_url) = resolve_card_url(EPORNER_ROOT, href) else {
            continue;
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

impl PagedSearch for EPornerExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[EPorner]"
    }

    // EPorner has no filter validation today; Ok(()) preserves that.
    fn validate_search_filters(&self, _filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        Ok(())
    }

    fn first_page_index(&self) -> u32 {
        0
    }

    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let spec = SearchPageSpec {
            headers: &[],
            build_url: build_search_url,
            parse: |body, _query, page| {
                let results = parse_results(body);
                let has_more = body.contains(&format!("/{}/", page + 2));
                Ok(SearchPage {
                    results,
                    total_estimate: None,
                    has_more,
                })
            },
        };
        self.fetch_via_spec(spec, query, page, ctx).await
    }
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
        Ok(self.search_page_response(query, ctx).await?.results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        self.search_page_response(query, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    // Fixture recorded live on 2026-04-23 from www.eporner.com/tag/amateur/1/
    const FIXTURE: &str = include_str!("tests/eporner_tag_page.html");

    /// EPorner's card selector is `a[href^='/video-']` — an attribute PREFIX
    /// match, which is the same guard hqporner spells as
    /// `h.starts_with("/hdporn/")`. Measured: it already refused every hostile
    /// href below, so unlike PornoXO and XHamster this site was not reachable
    /// through the concatenation, and routing it through `resolve_card_url`
    /// removes a dependence on that selector rather than closing a live hole.
    /// The assertion is on the parsed host so it stays honest if the selector
    /// is ever loosened to a `*=` contains match.
    #[test]
    fn no_result_can_move_the_authority_off_eporner() {
        for hostile in [
            "https://evil.test/video-abc/x/",
            "//evil.test/video-abc/x/",
            "@evil.test/video-abc/x/",
            ".evil.test/video-abc/x/",
        ] {
            let html = format!(
                r#"<html><body>
                    <div class="mbcontent"><a href="{hostile}"><img src="/t.jpg" alt="Hostile"></a></div>
                    <div class="mbcontent"><a href="/video-1/real/"><img src="/r.jpg" alt="Real"></a></div>
                </body></html>"#
            );
            let results = parse_results(&html);
            // Without this the test also passes when `parse_results` returns
            // NOTHING — a resolution regression that empties the page would
            // read as a green guard. The hostile anchor is never selected, so
            // exactly the one legitimate card must come back.
            assert_eq!(
                results.len(),
                1,
                "href {hostile:?}: the legitimate card must still be parsed"
            );
            for r in results {
                let host = url::Url::parse(&r.video_url)
                    .expect("every emitted result URL must parse")
                    .host_str()
                    .map(str::to_owned);
                assert_eq!(
                    host.as_deref(),
                    Some("www.eporner.com"),
                    "href {hostile:?} moved the authority: {}",
                    r.video_url
                );
            }
        }
    }

    /// A relative poster is resolved against the site root rather than handed
    /// to the UI as `/r.jpg`, and a `data:` one is dropped.
    #[test]
    fn thumbnails_are_resolved_and_non_http_ones_dropped() {
        let html = r#"<html><body>
            <div class="mbcontent"><a href="/video-1/real/"><img src="/r.jpg" alt="Real"></a></div>
            <div class="mbcontent"><a href="/video-2/bad/"><img src="data:text/html,x" alt="Bad"></a></div>
        </body></html>"#;
        let results = parse_results(html);
        assert_eq!(results.len(), 2, "a bad poster must not cost the card");
        assert_eq!(
            results[0].thumbnail_url.as_deref(),
            Some("https://www.eporner.com/r.jpg")
        );
        assert_eq!(results[1].thumbnail_url, None);
    }

    /// A lazy-loaded card takes its poster from `data-src`, not from the
    /// 1x1 `data:` placeholder sitting in `src`. Both attributes are present
    /// on the same element, so this pins WHICH one wins rather than merely
    /// that something came out.
    #[test]
    fn a_lazy_loaded_card_reads_its_poster_from_data_src() {
        let html = r#"<html><body>
            <div class="mbcontent"><a href="/video-1/lazy/"><img class="lazyimg"
                src="data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=="
                data-src="https://static-eu-cdn.eporner.com/thumbs/static4/1/x_240.jpg" alt="Lazy"></a></div>
        </body></html>"#;
        let results = parse_results(html);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].thumbnail_url.as_deref(),
            Some("https://static-eu-cdn.eporner.com/thumbs/static4/1/x_240.jpg"),
            "the data: placeholder must never win over data-src"
        );
    }

    /// The synthetic thumbnail test above cannot see a `src` SHAPE that the
    /// real site emits and `resolve_media_url` rejects — if the capture's
    /// posters were, say, lazy-loaded `data:` placeholders, every eporner
    /// thumbnail would vanish with the suite green. This is the eporner
    /// analogue of pornoxo's `every_card_on_both_captures_has_a_cdn_thumbnail`.
    ///
    /// Measured on the committed capture: the cards inside `div.mbcontent`
    /// carry real `https://static-eu-cdn.eporner.com/thumbs/...` srcs. The
    /// `data:image/gif;base64` 1x1 placeholders elsewhere in that file sit
    /// OUTSIDE the card anchors, so `MBIMG_SEL` never reaches them.
    #[test]
    fn every_card_in_the_real_capture_still_yields_a_cdn_thumbnail() {
        let results = parse_results(FIXTURE);
        assert!(!results.is_empty(), "the capture must parse to some cards");
        assert!(
            results.iter().all(|r| r
                .thumbnail_url
                .as_deref()
                .is_some_and(|t| t.starts_with("https://") && t.contains("/thumbs/"))),
            "every card must keep a CDN poster: {:?}",
            results
                .iter()
                .find(|r| r.thumbnail_url.is_none())
                .map(|r| &r.video_url)
        );
    }

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
