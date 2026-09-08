//! SearchExtractor implementation for XVideos.
//!
//! XVideos search URL format: `https://www.xvideos.com/?k={query}&p={page}`
//! where `p` is 0-indexed. Adding `&top` sorts by most-viewed.

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use scraper::Html;

use super::XVideosExtractor;
use crate::base::common::{
    BaseExtractor, PagedSearch, SearchPage, SearchPageSpec, resolve_card_url, resolve_media_url,
};

const XVIDEOS_BASE_URL: &str = "https://www.xvideos.com";

/// Poster attributes in preference order. `data-src` holds the real image
/// (`src` is a blank-GIF placeholder on nearly every card), `data-mzl` is the
/// mosaique listing image, `src` the last resort.
const THUMBNAIL_ATTRS: [&str; 3] = ["data-src", "data-mzl", "src"];

/// Maximum results cap for a full search (matches the pre-refactor
/// `unwrap_or(500)`; mirrors the xnxx sibling's named cap).
const MAX_PLAYLIST_SIZE: usize = 500;

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

/// Extract a view count from concatenated `p.metadata` text.
///
/// Looks for a number (with optional `K`/`M`/`B` suffix) immediately
/// preceding the literal token `Views`. Returns `None` if the marker
/// isn't found or the number won't parse.
fn extract_view_count(metadata_text: &str) -> Option<u64> {
    let lower = metadata_text.to_ascii_lowercase();
    let views_idx = lower.find("views")?;
    let before = metadata_text[..views_idx].trim_end();
    let token = before.split_whitespace().next_back()?;
    BaseExtractor::parse_human_count(token)
}

/// Check whether the HTML contains a link to the next page.
fn has_next_page(html: &str, next_page: u32) -> bool {
    // Next page link would contain `p={next_page}` in the URL
    html.contains(&format!("p={next_page}"))
}

/// Parse search result items from XVideos search HTML.
pub(crate) fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let doc = Html::parse_document(html);

    let block_sel = crate::selector!("div.thumb-block");
    let anchor_sel = crate::selector!("div.thumb-inside a[href^='/video.']");
    let title_sel = crate::selector!("p.title a");
    let img_sel = crate::selector!("div.thumb-inside img");
    let dur_sel = crate::selector!(".duration, span.duration");
    let metadata_sel = crate::selector!("p.metadata");
    // Uploader link inside p.metadata. XVideos uses many href shapes for
    // uploaders — `/profiles/<user>`, `/channels/<slug>`, `/amateur-channels/…`,
    // `/pornstar-channels/…`, AND bare vanity URLs like `/young-libertines` or
    // `/tommy_cabrio_official`. The reliable common denominator across all of
    // them is `p.metadata a .name`, so use that rather than an href-prefix
    // allow-list (which caught only ~4% of cards in testing).
    let uploader_sel = crate::selector!("p.metadata a .name");

    let mut results = Vec::new();

    for block in doc.select(block_sel) {
        // Extract video URL from the anchor inside thumb-inside
        let video_url = block
            .select(anchor_sel)
            .next()
            .and_then(|a| a.value().attr("href"))
            // The anchor selector is an href-PREFIX match, which already makes
            // an authority impossible; resolving through `resolve_card_url`
            // (#665) removes the dependence on that selector staying a prefix
            // match rather than closing a live hole.
            .and_then(|href| resolve_card_url(XVIDEOS_BASE_URL, href));

        let Some(video_url) = video_url else {
            continue;
        };

        // Title from p.title a[title] attribute, fall back to text content
        let title = block
            .select(title_sel)
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
        //
        // Resolved through `resolve_media_url` — deliberately NOT
        // `resolve_card_url`: XVideos' posters live on `*.xvideos-cdn.com`, so
        // an origin comparison would drop every real thumbnail. What it refuses
        // is a non-`http(s)` reference reaching the desktop's `<img src>`, and
        // it absolutizes a relative one. Both the placeholder filter and the
        // resolution apply PER candidate: after an `or_else` chain they would
        // run only on whichever attribute won, so a `data-src` holding the
        // blank placeholder (or an unusable `data:` value) would yield `None`
        // instead of falling through to `data-mzl`/`src`. The `THUMBNUM`
        // substitution stays ahead of resolution so the placeholder token never
        // survives into a URL.
        let thumbnail_url = block.select(img_sel).next().and_then(|img| {
            let attrs = img.value();
            THUMBNAIL_ATTRS
                .iter()
                .filter_map(|attr| attrs.attr(attr))
                // An empty attribute joins to the BASE, so it would resolve
                // "successfully" to the site root rather than being skipped.
                .filter(|u| !u.is_empty() && !u.contains("lightbox-blank"))
                .map(|u| u.replace("THUMBNUM", "1"))
                .find_map(|u| resolve_media_url(XVIDEOS_BASE_URL, &u))
        });

        // Duration from .duration or span.duration
        let duration = block.select(dur_sel).next().and_then(|el| {
            let text: String = el.text().collect::<String>();
            parse_duration_text(text.trim())
        });

        // Uploader from the first profile/channel link in p.metadata
        let uploader = block
            .select(uploader_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        // View count from the bare text node next to "Views" inside p.metadata.
        // Shape is: `<span class="sprfluous"> - </span> 7.5M <span ...>Views</span>`
        // — concatenate all text under p.metadata and pull out `<num>[KMB]?` immediately
        // before the literal `Views` token.
        let view_count = block
            .select(metadata_sel)
            .next()
            .and_then(|m| extract_view_count(&m.text().collect::<String>()));

        results.push(SearchResultPreview {
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

    results
}

impl PagedSearch for XVideosExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[XVideos]"
    }

    // XVideos has no filter validation today (the pre-refactor single-GET
    // search path never validated); `Ok(())` is the only value that preserves that.
    fn validate_search_filters(&self, _filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        Ok(())
    }

    fn first_page_index(&self) -> u32 {
        1
    }

    fn max_results_default(&self) -> usize {
        MAX_PLAYLIST_SIZE
    }

    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let spec = SearchPageSpec {
            headers: &[],
            // XVideos is 0-indexed internally; external page is 1-indexed.
            build_url: |query, page| build_search_url(query, page.saturating_sub(1)),
            parse: |body, _query, page| {
                let results = parse_search_results(body);
                // Reproduce the original arg exactly: has_next_page(webpage, page_0indexed + 1),
                // where page_0indexed = page.saturating_sub(1). Equal to page for page>=1; correct at page 0.
                Ok(SearchPage {
                    has_more: has_next_page(body, page.saturating_sub(1) + 1)
                        && !results.is_empty(),
                    total_estimate: None,
                    results,
                })
            },
        };
        self.fetch_via_spec(spec, query, page, ctx).await
    }
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
        self.search_all_pages(query, ctx).await
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

    const FIXTURE: &str = include_str!("tests/xvideos_search_page.html");

    /// The four #665 reference shapes, in one array because on this site they
    /// share one outcome — see `no_result_can_move_the_authority_off_xvideos`.
    const HOSTILE_HREFS: [&str; 4] = [
        "https://evil.test/video.abc/x/",
        "//evil.test/video.x/",
        "@evil.test/video.x/",
        ".evil.test/video.x/",
    ];

    const LEGIT_HREF: &str = "/video.abc123/real_clip";

    fn host_of(url: &str) -> String {
        url::Url::parse(url)
            .expect("every emitted URL must parse")
            .host_str()
            .expect("every emitted URL must carry a host")
            .to_string()
    }

    fn thumb_block(href: &str, title: &str) -> String {
        format!(
            r#"<div class="thumb-block">
                 <div class="thumb-inside"><a href="{href}"><img data-src="/t.jpg"></a></div>
                 <p class="title"><a href="{href}" title="{title}">{title}</a></p>
               </div>"#
        )
    }

    fn page(blocks: &str) -> String {
        format!("<html><body>{blocks}</body></html>")
    }

    /// All four #665 shapes are dropped, and — unlike PornHub — none of them
    /// reaches `resolve_card_url` at all: the card anchor selector
    /// `a[href^='/video.']` is an attribute PREFIX match, the same guard
    /// hqporner spells as `h.starts_with("/hdporn/")`, so a reference that does
    /// not begin `/video.` is never selected. Measured on 2026-09-08 against
    /// the unconverted parser: all four were already refused there too, because
    /// a reference forced to begin with a single `/` is path-absolute and RFC
    /// 3986 gives it no way to introduce an authority. Routing this site
    /// through `resolve_card_url` therefore removes a dependence on that
    /// selector staying a prefix match rather than closing a live hole, and
    /// this test is what keeps the property pinned if it is ever loosened to a
    /// `*=` contains match.
    ///
    /// The legitimate card is co-located deliberately (eporner's prior art,
    /// `eporner/search.rs`): asserting only "nothing off-origin survives" would
    /// hold vacuously on an empty result, so the `len() == 1` is what makes the
    /// test fail if resolution ever empties the page instead of guarding it.
    #[test]
    fn no_result_can_move_the_authority_off_xvideos() {
        for hostile in HOSTILE_HREFS {
            let html = page(&format!(
                "{}{}",
                thumb_block(hostile, "Hostile"),
                thumb_block(LEGIT_HREF, "Real")
            ));
            let results = parse_search_results(&html);
            assert_eq!(
                results.len(),
                1,
                "href {hostile:?}: the hostile card must be dropped and the legitimate one kept"
            );
            assert_eq!(results[0].title, "Real");
            assert_eq!(host_of(&results[0].video_url), "www.xvideos.com");
        }
    }

    /// Posters go through `resolve_media_url`, so an off-origin CDN host is
    /// kept (that is where every real XVideos thumb lives), a relative one is
    /// absolutized rather than handed to the UI as `/t.jpg`, and a `data:`
    /// reference never reaches the desktop's `<img src>`. A bad poster costs
    /// the poster, not the card.
    #[test]
    fn posters_are_resolved_and_non_http_ones_dropped() {
        let block = |src: &str, title: &str| {
            format!(
                r#"<div class="thumb-block">
                     <div class="thumb-inside"><a href="{LEGIT_HREF}"><img data-src="{src}"></a></div>
                     <p class="title"><a href="{LEGIT_HREF}" title="{title}">{title}</a></p>
                   </div>"#
            )
        };
        let cdn = "https://img-hw.xvideos-cdn.com/videos/thumbs169/1/real.jpg";
        let html = page(&format!(
            "{}{}{}",
            block(cdn, "Cdn"),
            block("/t.jpg", "Relative"),
            block("data:text/html,x", "Bad")
        ));
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 3, "a bad poster must not cost the card");
        assert_eq!(results[0].thumbnail_url.as_deref(), Some(cdn));
        assert_eq!(
            results[1].thumbnail_url.as_deref(),
            Some("https://www.xvideos.com/t.jpg")
        );
        assert_eq!(results[2].thumbnail_url, None);
    }

    /// A `data-src` the pipeline cannot use must fall through to
    /// `data-mzl`/`src` rather than costing the card its poster. Both
    /// rejections are exercised, because they are enforced at different steps
    /// and an `or_else` chain would defeat each separately: the blank
    /// placeholder is dropped by the name filter, and a `data:` URI only by the
    /// resolver.
    #[test]
    fn an_unusable_data_src_falls_through_to_the_next_attribute() {
        let real = "https://img-hw.xvideos-cdn.com/videos/thumbs169/1/real.jpg";
        for unusable in [
            "https://cdn.xvideos-cdn.com/img/lightbox/lightbox-blank.gif",
            "data:image/gif;base64,R0lGOD",
        ] {
            let html = page(&format!(
                r#"<div class="thumb-block">
                     <div class="thumb-inside"><a href="{LEGIT_HREF}"><img
                        data-src="{unusable}" data-mzl="{real}"></a></div>
                     <p class="title"><a href="{LEGIT_HREF}" title="Lazy">Lazy</a></p>
                   </div>"#
            ));
            let results = parse_search_results(&html);
            assert_eq!(results.len(), 1);
            assert_eq!(
                results[0].thumbnail_url.as_deref(),
                Some(real),
                "data-src {unusable:?} must fall through"
            );
        }
    }

    #[test]
    fn ordinary_root_relative_href_resolves() {
        let results = parse_search_results(&page(&thumb_block(LEGIT_HREF, "Real")));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].video_url,
            format!("{XVIDEOS_BASE_URL}{LEGIT_HREF}"),
            "a normal root-relative href must still yield a usable URL"
        );
    }

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

    /// Regression: the uploader selector must match all of XVideos'
    /// uploader-link shapes, not just `/profiles/<user>`.  In practice the
    /// majority of cards use vanity URLs (`/tommy_cabrio_official`,
    /// `/young-libertines`, `/familystrokes`, …) with no `/profiles/` prefix.
    #[test]
    fn uploader_matches_profiles_and_vanity_urls() {
        let html = r#"
        <div class="thumb-block"><div class="thumb-inside"><div class="thumb">
          <a href="/video.a1/slug"><img data-src="https://x/1.jpg"/></a>
        </div></div><div class="thumb-under">
          <p class="title"><a title="Vanity URL">Vanity URL</a></p>
          <p class="metadata"><span class="bg">
            <a href="/tommy_cabrio_official"><span class="name">Tommycabrio</span></a>
          </span></p>
        </div></div>
        <div class="thumb-block"><div class="thumb-inside"><div class="thumb">
          <a href="/video.a2/slug"><img data-src="https://x/2.jpg"/></a>
        </div></div><div class="thumb-under">
          <p class="title"><a title="Profiles URL">Profiles URL</a></p>
          <p class="metadata"><span class="bg">
            <a href="/profiles/skinnyboba"><span class="name">Skinnyboba</span></a>
          </span></p>
        </div></div>
        <div class="thumb-block"><div class="thumb-inside"><div class="thumb">
          <a href="/video.a3/slug"><img data-src="https://x/3.jpg"/></a>
        </div></div><div class="thumb-under">
          <p class="title"><a title="Hyphen vanity">Hyphen vanity</a></p>
          <p class="metadata"><span class="bg">
            <a href="/young-libertines"><span class="name">Young Libertines</span></a>
          </span></p>
        </div></div>
        "#;
        let results = parse_search_results(html);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].uploader.as_deref(), Some("Tommycabrio"));
        assert_eq!(results[1].uploader.as_deref(), Some("Skinnyboba"));
        assert_eq!(results[2].uploader.as_deref(), Some("Young Libertines"));
    }

    /// Regression: prior to this fix every result had `view_count = None`
    /// because `parse_search_results` hardcoded the field. The live
    /// fixture (captured 2026-04-28 from `/?k=teen`) has a `Views` token
    /// in every metadata block, so most rows must populate `view_count`.
    #[test]
    fn parse_results_extracts_view_count() {
        const LIVE: &str = include_str!("tests/xvideos_search_live.html");
        let results = parse_search_results(LIVE);
        assert!(!results.is_empty(), "live fixture should yield results");

        let with_views = results.iter().filter(|r| r.view_count.is_some()).count();
        assert!(
            with_views >= results.len() / 2,
            "expected most rows to carry view_count; got {with_views}/{}",
            results.len()
        );

        // Spot-check: every parsed view count must be > 0
        for v in results.iter().filter_map(|r| r.view_count) {
            assert!(v > 0, "view_count of zero should be None, not Some(0)");
        }
    }

    /// Regression: `search_page`'s `parse` closure must reproduce the original
    /// pre-refactor `has_next_page(webpage, page_0indexed + 1)` call exactly,
    /// where `page_0indexed = page.saturating_sub(1)`. For `page >= 1` this is
    /// identical to passing `page` directly, but at the out-of-contract
    /// `page == 0` boundary the original computes `0.saturating_sub(1) + 1 == 1`,
    /// NOT `0`. Pin both sides of that boundary directly against `has_next_page`
    /// (the closure itself is only reachable via `PagedSearch::fetch_via_spec`,
    /// which performs a network fetch, so this exercises the restored
    /// arithmetic rather than the full search_page path).
    #[test]
    fn has_next_page_boundary_matches_original_arg_computation() {
        let html = "...p=1...";
        // page == 0: original arg is 0.saturating_sub(1) + 1 == 1, not 0.
        let page = 0_u32;
        assert_eq!(page.saturating_sub(1) + 1, 1);
        assert!(has_next_page(html, page.saturating_sub(1) + 1));
        assert!(!has_next_page(html, page));

        // page >= 1: the restored computation equals `page` directly.
        for page in 1_u32..=3 {
            assert_eq!(page.saturating_sub(1) + 1, page);
        }
    }

    #[test]
    fn extract_view_count_handles_known_shapes() {
        // The text concatenation inside p.metadata typically looks like:
        //   "6 min Smack My Bitch -  7.5M Views  - "
        assert_eq!(
            extract_view_count("6 min Smack My Bitch -  7.5M Views  - "),
            Some(7_500_000)
        );
        assert_eq!(
            extract_view_count("11 min Channel - 174.8k Views"),
            Some(174_800)
        );
        assert_eq!(extract_view_count("11 min - 1,234 Views"), Some(1_234));
        assert_eq!(extract_view_count("no views token here"), None);
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
