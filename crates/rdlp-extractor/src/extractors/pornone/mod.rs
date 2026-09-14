//! pornone.com extractor.
//!
//! The video page serves the real, per-rendition signed MP4 URLs directly in
//! static HTML (see `media.rs`) — there is no separate manifest or size
//! endpoint to probe, and the signed URLs are short-lived, so `extract` never
//! runs `detect_format_sizes_lazy`/HEAD probing: the served geometry and
//! bitrate are all the information the site offers, and a probe would only
//! spend one of the URL's few remaining minutes for no gain.

mod media;
mod patterns;
mod search;
mod search_patterns;

use async_trait::async_trait;
use lazy_regex::Regex;
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, SearchExtractor};
use rdlp_types::{
    DownloadProtocol, Format, InfoDict, SearchFilter, SearchFilterDescriptor, SearchPageResponse,
    SearchQuery, SearchResultPreview,
};
use scraper::Html;

use crate::base::common::json_ld::{
    extract_json_ld, extract_tags, extract_view_count, get_thumbnail_url,
};
use crate::base::common::{BaseExtractor, PagedSearch, SearchOrigin, SearchPage};

/// The one spelling of this site's display name (#756); `name()`,
/// `InfoDict::new`, log tags and filter errors all read it.
pub(crate) const NAME: rdlp_types::ExtractorName = rdlp_types::ExtractorName::PornOne;

/// pornone.com — server-rendered, individually-signed progressive MP4
/// renditions; cookie-free search with fixed-grid filler detection.
pub struct PornoneExtractor {
    /// Origin the listing/search URLs are built against. Production literal
    /// by default; test-injected to a mockito origin via `with_origin`,
    /// mirroring the PornoXO seam.
    origin: SearchOrigin,
}

impl PornoneExtractor {
    /// Create a new PornOne extractor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: search::default_origin(),
        }
    }

    /// Test-only: point the listing builders at a mockito origin.
    #[cfg(test)]
    pub(crate) fn with_origin(origin: SearchOrigin) -> Self {
        Self { origin }
    }
}

impl Default for PornoneExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for PornoneExtractor {
    fn name(&self) -> &str {
        NAME.as_str()
    }

    fn valid_url(&self) -> &Regex {
        &patterns::URL_PATTERN
    }

    fn priority(&self) -> i32 {
        50
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let video_id = patterns::parse_video_id(url)
            .ok_or_else(|| RdlpError::extraction("URL is not a PornOne video page", url))?;

        debug!(
            "[{NAME}] Extracting {video_id} from {}",
            rdlp_redact::RedactedUrl::new(url)
        );

        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // `Html` is !Send — parse, take what we need, drop before any await.
        let (renditions, json_ld) = {
            let document = Html::parse_document(&webpage);
            (
                media::parse_renditions(&document, url),
                extract_json_ld(&document),
            )
        };
        if renditions.is_empty() {
            return Err(RdlpError::extraction(
                "page served no signed <source> rendition",
                url,
            ));
        }

        let mut formats = Vec::with_capacity(renditions.len());
        for r in renditions {
            // Page-sourced URL: gate before it can be probed or returned.
            BaseExtractor::validate_url_security(&r.url)?;
            // `format_id` is a lookup key elsewhere (the selector DSL's
            // `FormatToken::FormatId`, resume-state's `find(|f| f.format_id
            // == saved.format_id)`), so height alone collides whenever the
            // site serves two renditions at the same height and different
            // bitrates — fold the bitrate in too.
            let mut f = Format::new(
                format!("mp4-{}p-{}k", r.height, r.bitrate_kbps),
                &r.url,
                "mp4",
                DownloadProtocol::Https,
            );
            f.width = Some(r.width);
            f.height = Some(r.height);
            f.tbr = Some(f64::from(r.bitrate_kbps));
            formats.push(f);
        }

        let title = json_ld
            .as_ref()
            .and_then(|j| j.name.clone())
            .unwrap_or_else(|| video_id.clone());
        let mut info = InfoDict::new(&video_id, &title, InfoExtractor::name(self), url);
        info.formats = formats;
        info.age_limit = Some(18);
        if let Some(j) = &json_ld {
            info.description = j.description.clone();
            info.thumbnail = get_thumbnail_url(j);
            info.upload_date = j
                .upload_date
                .as_deref()
                .and_then(BaseExtractor::parse_iso8601_date);
            info.duration = j
                .duration
                .as_deref()
                .and_then(BaseExtractor::parse_iso8601_duration);
            info.tags = extract_tags(j);
            info.view_count = extract_view_count(j);
        }
        info.propagate_duration();
        Ok(info)
    }
}

impl PagedSearch for PornoneExtractor {
    fn validate_search_filters(&self, filters: &[SearchFilter]) -> Result<()> {
        search_patterns::validate(filters)
    }

    /// `?page=0` and negative pages are not a thing on this site; floor at 1
    /// (PornoXO precedent).
    fn clamp_page(&self, page: u32) -> u32 {
        page.max(1)
    }

    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let url = search::build_search_url(&self.origin, query, page);
        debug!(
            "[{NAME}] Fetching search page {page}: {}",
            rdlp_redact::RedactedUrl::new(&url)
        );
        let body = BaseExtractor::fetch_webpage(&url, ctx).await?;
        let listing = search::parse_search_page(&self.origin, &body);
        // A filler page is the end of the listing — the site pads no-match,
        // past-the-end and out-of-range pages with the same 200 grid, so
        // "has more" is exactly "this page was real".
        Ok(SearchPage {
            results: listing.results,
            has_more: !listing.is_filler,
            total_estimate: None,
        })
    }
}

#[async_trait]
impl SearchExtractor for PornoneExtractor {
    fn name(&self) -> &str {
        NAME.as_str()
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        search_patterns::supported_filters()
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
    use crate::hls::test_support::*;

    const VIDEO_PAGE: &str = include_str!("tests/pornone_video_page.html");

    #[test]
    fn name_priority_and_routing() {
        let e = PornoneExtractor::new();
        assert_eq!(InfoExtractor::name(&e), "PornOne");
        assert_eq!(e.priority(), 50);
        assert!(e.suitable("https://pornone.com/ass/slug/42/"));
        assert!(!e.suitable("https://pornone.com/search/?q=x"));
    }

    #[tokio::test]
    async fn extract_yields_one_format_per_served_source_with_json_ld_metadata() {
        // mockito serves the captured page; formats are the fixture's signed
        // URLs (real pornone hosts, so the SSRF gate admits them).
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/1080p/pornone-sex-video-278218023/278218023/")
            .with_body(VIDEO_PAGE)
            .create_async()
            .await;
        let ctx = test_ctx();
        let url = format!(
            "{}/1080p/pornone-sex-video-278218023/278218023/",
            server.url()
        );
        // `is_suitable` is anchored on pornone.com, so call `extract` directly.
        let info = PornoneExtractor::new()
            .extract(&url, &ctx)
            .await
            .expect("extract");
        assert_eq!(info.id, "278218023");
        assert_eq!(info.title, "milf");
        assert_eq!(info.duration, Some(139.0));
        assert_eq!(info.age_limit, Some(18));
        assert!(
            info.thumbnail
                .as_deref()
                .is_some_and(|t| t.starts_with("https://th-eu4.pornone.com/"))
        );
        assert!(!info.formats.is_empty());
        for f in &info.formats {
            assert_eq!(f.ext, "mp4");
            assert_eq!(f.protocol, DownloadProtocol::Https);
            assert!(f.width.is_some() && f.height.is_some());
            assert_eq!(f.duration, Some(139.0), "propagate_duration");
        }
        let ids: Vec<&str> = info.formats.iter().map(|f| f.format_id.as_str()).collect();
        assert!(ids.contains(&"mp4-406p-500k"), "{ids:?}");
    }

    /// Two renditions sharing a height but not a bitrate must not collapse
    /// onto the same `format_id` — it is a lookup key elsewhere (the
    /// selector DSL's `FormatToken::FormatId`, resume-state's
    /// `find(|f| f.format_id == saved.format_id)` in
    /// `rdlp-api/src/orchestrator/state/mod.rs`), so a collision silently
    /// picks the wrong rendition. Fails against `format!("mp4-{}p",
    /// r.height)` (both would be `"mp4-406p"`); passes once the bitrate is
    /// folded in.
    #[tokio::test]
    async fn same_height_renditions_get_distinct_format_ids() {
        let mut server = mockito::Server::new_async().await;
        let page = r#"<video>
            <source src="https://s1.pornone.com/vid2/sig/1/1/9/9_720x406_500k.mp4" type="video/mp4"/>
            <source src="https://s1.pornone.com/vid2/sig/1/1/9/9_720x406_2000k.mp4" type="video/mp4"/>
        </video>"#;
        let _m = server
            .mock("GET", "/ass/slug/1/")
            .with_body(page)
            .create_async()
            .await;
        let ctx = test_ctx();
        let info = PornoneExtractor::new()
            .extract(&format!("{}/ass/slug/1/", server.url()), &ctx)
            .await
            .expect("extract");

        let ids: Vec<&str> = info.formats.iter().map(|f| f.format_id.as_str()).collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert_ne!(ids[0], ids[1], "same-height renditions collided: {ids:?}");
        assert!(ids.contains(&"mp4-406p-500k"), "{ids:?}");
        assert!(ids.contains(&"mp4-406p-2000k"), "{ids:?}");
    }

    #[tokio::test]
    async fn extract_fails_clearly_when_no_source_is_served() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/ass/slug/1/")
            .with_body("<html><body>no player</body></html>")
            .create_async()
            .await;
        let ctx = test_ctx();
        let err = PornoneExtractor::new()
            .extract(&format!("{}/ass/slug/1/", server.url()), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no signed <source>"), "{err}");
    }
}

/// End-to-end search: `fetch_page` → `SearchExtractor::search`, driven
/// against a mockito origin (the seam established for PornHub in #457).
#[cfg(test)]
mod paged_search_tests {
    use super::*;
    use crate::hls::test_support::test_ctx;

    const SEARCH_PAGE: &str = include_str!("tests/pornone_search_page.html");
    const FILLER_PAGE: &str = include_str!("tests/pornone_search_filler.html");

    fn query(q: &str) -> SearchQuery {
        SearchQuery {
            query: q.to_owned(),
            filters: Vec::new(),
            max_results: None,
            page: None,
        }
    }

    /// Page 1 real, page 2 filler: `search()` must aggregate only the real
    /// page's results and stop, not keep paging into the fixed filler grid.
    #[tokio::test]
    async fn search_stops_at_the_first_filler_page() {
        let mut server = mockito::Server::new_async().await;
        let _p1 = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/search/\?q=milf&page=1$".into()),
            )
            .with_body(SEARCH_PAGE)
            .create_async()
            .await;
        let _p2 = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/search/\?q=milf&page=2$".into()),
            )
            .with_body(FILLER_PAGE)
            .create_async()
            .await;

        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");
        let results = SearchExtractor::search(
            &PornoneExtractor::with_origin(origin),
            &query("milf"),
            &test_ctx(),
        )
        .await
        .expect("a healthy search must succeed");

        let page1 = search::parse_search_page(
            &SearchOrigin::new(&server.url()).expect("mockito origin is well formed"),
            SEARCH_PAGE,
        );
        assert_eq!(results.len(), page1.results.len());
    }

    /// A page-1 filler (no-match query) must report zero results, not the
    /// fixed popular-videos grid.
    #[tokio::test]
    async fn a_filler_first_page_reports_zero_results() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", mockito::Matcher::Any)
            .with_body(FILLER_PAGE)
            .create_async()
            .await;

        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");
        let results = SearchExtractor::search(
            &PornoneExtractor::with_origin(origin),
            &query("zqxjvkwplm"),
            &test_ctx(),
        )
        .await
        .expect("a filler page is zero results, not an error");
        assert!(results.is_empty(), "{}", results.len());
    }

    #[tokio::test]
    async fn an_unknown_filter_is_refused_before_fetching() {
        let mut server = mockito::Server::new_async().await;
        let never = server
            .mock("GET", mockito::Matcher::Any)
            .with_body(SEARCH_PAGE)
            .expect(0)
            .create_async()
            .await;

        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");
        let mut q = query("milf");
        q.filters.push(SearchFilter {
            key: "ordering".to_owned(),
            value: "newest".to_owned(),
        });
        let err = PornoneExtractor::with_origin(origin)
            .search_all_pages(&q, &test_ctx())
            .await
            .expect_err("PornOne accepts no filters");
        assert!(err.to_string().contains("ordering"), "{err}");
        never.assert_async().await;
    }
}
