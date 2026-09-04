//! PornoXO extractor.
//!
//! The video page embeds a signed HLS ladder in an inline `playerConfig`
//! block. The signature is minted per page load and expires, so the master
//! URL is fetched and expanded during extraction and never cached.

mod patterns;
mod player;
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
use crate::base::common::{BaseExtractor, PagedSearch, SearchOrigin, SearchPage, filter_value};
use crate::hls::detect_format_sizes_lazy;

/// PornoXO — signed per-page-load HLS ladder read from an inline `playerConfig`.
pub struct PornoxoExtractor {
    /// Origin the listing/search URLs are built against. Production literal by
    /// default; test-injected to a mockito origin via `with_origin`, mirroring
    /// the PornHub seam. Typed rather than a `String` so only a shape-validated
    /// origin can reach the URL builders and the card URLs they produce.
    origin: SearchOrigin,
}

impl PornoxoExtractor {
    /// Create a new PornoXO extractor.
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

impl Default for PornoxoExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for PornoxoExtractor {
    fn name(&self) -> &str {
        "PornoXO"
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
            .ok_or_else(|| RdlpError::extraction("URL is not a PornoXO video page", url))?;

        debug!(
            "[PornoXO] Extracting {video_id} from {}",
            rdlp_redact::RedactedUrl::new(url)
        );

        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        let master_url = player::extract_master_url(&webpage)
            .map_err(|e| RdlpError::extraction(format!("{e:#}"), url))?;

        // The master URL comes from page JavaScript and is therefore
        // attacker-controlled. `expand_hls_url` does not validate its own seed
        // URL, so gate it here before any fetch. Shares the playlist-URI gate
        // rather than restating it, so production behaviour cannot drift
        // between the master and the variants it resolves.
        crate::hls::validate_resolved_url(&master_url)
            .map_err(|e| RdlpError::extraction(format!("master URL rejected: {e}"), url))?;

        // `Html` is !Send, so parse and drop it before any await.
        let json_ld = extract_json_ld(&Html::parse_document(&webpage));

        // Explicit M3u8: the master URL ends in `.mp4`, so extension sniffing
        // would misclassify it as progressive HTTP. That `.mp4` suffix belongs
        // to the protocol's disguise and stops there — `ext` is `m3u8`, as
        // every other HLS extractor seeds it, because `expand_hls_url` copies
        // the seed's `ext` onto every expanded rendition and it feeds
        // `%(ext)s` and the container/remux decisions downstream.
        let seed = Format::new("hls", &master_url, "m3u8", DownloadProtocol::M3u8);

        // MUST run before `detect_format_sizes_lazy` (issue #269 / #279).
        let formats = crate::hls::expand_hls_in_place(vec![seed], ctx.http_client.clone()).await;
        let extractor_name = InfoExtractor::name(self);
        let (formats, hls_flags) = detect_format_sizes_lazy(formats, ctx, extractor_name).await;

        if formats.is_empty() {
            return Err(RdlpError::extraction(
                "signed HLS master expanded to no formats",
                url,
            ));
        }

        let title = json_ld
            .as_ref()
            .and_then(|j| j.name.clone())
            .unwrap_or_else(|| video_id.clone());

        let mut info = InfoDict::new(&video_id, &title, extractor_name, url);
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
        if hls_flags.is_live {
            info.is_live = Some(true);
        }
        Ok(info)
    }
}

impl PagedSearch for PornoxoExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[PornoXO]"
    }

    fn validate_search_filters(&self, filters: &[SearchFilter]) -> Result<()> {
        search_patterns::validate(filters)
    }

    /// Floor an explicitly requested page at 1.
    ///
    /// The same failure the `max_page` guard covers, at the low end: `?page=0`
    /// answers HTTP 200 with page 1's grid, so without this the response
    /// echoes `page: 0` over page 1's videos. `clamp_page`'s default is
    /// identity and a universal `.max(first_page_index())` would be wrong for
    /// 0-indexed sites, so it is opt-in per site; ABXXX is the precedent.
    fn clamp_page(&self, page: u32) -> u32 {
        page.max(1)
    }

    /// Fetch and parse one listing page.
    ///
    /// A custom body rather than a `fetch_via_spec` one-liner, because two of
    /// this site's behaviours cannot be expressed as a build-URL/parse pair:
    /// the out-of-range page has to be refused BEFORE parsing, and a 403 means
    /// different things on the two routes.
    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let route = filter_value(&query.filters, "route").unwrap_or("search");
        let url = search::build_listing_url(&self.origin, query, page);

        debug!(
            "[PornoXO] Fetching {route} listing page {page}: {}",
            rdlp_redact::RedactedUrl::new(&url)
        );

        // Deliberately routed through `fetch_webpage`, which runs
        // `check_http_response` BEFORE reading the body. That ordering is the
        // structural defence for this site's nastiest behaviour: `/search/?q=`
        // answers a nonsense query with HTTP 404 AND a fully populated 52-row
        // grid of unrelated filler. Parsing that body would hand the operator
        // 52 confident, wrong results.
        let body = match BaseExtractor::fetch_webpage(&url, ctx).await {
            Ok(body) => body,
            // The search route sits behind a Cloudflare challenge; a
            // clearance solved once in a browser transplants into our client
            // and survives pagination, so the actionable advice is cookies.
            // The tag route is cookie-free, so the same status there means
            // something else and must not send the operator chasing cookies.
            Err(RdlpError::Http { status: 403, .. }) if route == "search" => {
                return Err(RdlpError::Extraction {
                    message: "PornoXO search is behind a Cloudflare challenge. Pass \
                              --cookies-from-browser <browser> after solving it once in \
                              that browser, or use --search-filter route=tag to list a \
                              tag instead (which returns different results)."
                        .to_owned(),
                    url: None,
                });
            }
            Err(e) => return Err(e),
        };

        let listing = search::parse_listing_page(&self.origin, &body);

        // An out-of-range page answers HTTP 200 with page 1's videos rather
        // than an error or an empty grid, so the bound has to come from the
        // pager on the page, never from the response status or row count.
        // Returning page 1 labelled as page 999 would be confidently wrong.
        if let Some(max) = listing.max_page
            && page > max
        {
            return Err(RdlpError::Extraction {
                message: format!(
                    "page {page} is beyond this listing's last page ({max}); \
                     the site silently serves page 1 for out-of-range pages"
                ),
                url: None,
            });
        }

        Ok(SearchPage {
            results: listing.results,
            // `?page=999` returning HTTP 200 with page 1's content means an
            // empty-grid stop condition never fires; the `Next` anchor is the
            // only signal that terminates.
            has_more: listing.has_next,
            // The site publishes no result count on either route.
            total_estimate: None,
        })
    }
}

#[async_trait]
impl SearchExtractor for PornoxoExtractor {
    fn name(&self) -> &str {
        "PornoXO"
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

    const VIDEO_PAGE: &str = include_str!("tests/pornoxo_video_page.html");

    /// A WIRING check, not a pattern check: it asserts that
    /// `InfoExtractor::suitable` delegates to `patterns::is_suitable`, which is
    /// what decides whether this extractor claims an operator-supplied URL.
    /// The assertions duplicate `patterns::tests` on purpose — those exercise
    /// the regex directly and would still pass if the trait method were wired
    /// to the wrong predicate, or to `valid_url` alone. Do not delete this as
    /// redundant.
    #[test]
    fn suitable_matches_only_video_urls() {
        let x = PornoxoExtractor::new();
        assert!(InfoExtractor::suitable(
            &x,
            "https://www.pornoxo.com/videos/2928541/slug/"
        ));
        assert!(!InfoExtractor::suitable(
            &x,
            "https://www.pornoxo.com/tags/creampie/"
        ));
    }

    #[tokio::test]
    async fn extracts_three_formats_and_metadata_from_signed_ladder() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        // Master playlist: three variants, absolute-path URIs (as the live site serves).
        let master = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1201052,RESOLUTION=854x480,CODECS=\"mp4a.40.2,avc1.4d4015\"\n\
            /v/480.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=706527,RESOLUTION=426x240,CODECS=\"mp4a.40.2,avc1.4d4015\"\n\
            /v/240.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2835133,RESOLUTION=1280x720,CODECS=\"mp4a.40.2,avc1.4d4015\"\n\
            /v/720.m3u8\n";
        let media = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nseg0.ts\n#EXT-X-ENDLIST\n";

        // Rewrite the fixture's CDN host to the mock server so the whole
        // page -> master -> variant chain is exercised end to end.
        let page = VIDEO_PAGE.replace("https:\\/\\/cdn.pornoxo.com", &base.replace('/', "\\/"));

        let _page_mock = server
            .mock("GET", "/videos/2928541/x/")
            .with_status(200)
            .with_body(&page)
            .create_async()
            .await;
        let _master_mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/key=.*_TPL_\.mp4$".into()),
            )
            .with_status(200)
            .with_body(master)
            .create_async()
            .await;
        let _v480 = server
            .mock("GET", "/v/480.m3u8")
            .with_status(200)
            .with_body(media)
            .create_async()
            .await;
        let _v240 = server
            .mock("GET", "/v/240.m3u8")
            .with_status(200)
            .with_body(media)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v/720.m3u8")
            .with_status(200)
            .with_body(media)
            .create_async()
            .await;

        let ctx = test_ctx();
        let info = PornoxoExtractor::new()
            .extract(&format!("{base}/videos/2928541/x/"), &ctx)
            .await
            .expect("extraction should succeed");

        assert_eq!(info.id, "2928541");
        assert_eq!(
            info.title,
            "He Fucked his Stepsister while she was on the Phone"
        );
        assert_eq!(info.duration, Some(930.0));
        assert_eq!(info.age_limit, Some(18));
        assert_eq!(info.formats.len(), 3, "one Format per signed rendition");
        let mut heights: Vec<_> = info.formats.iter().filter_map(|f| f.height).collect();
        heights.sort_unstable();
        assert_eq!(heights, vec![240, 480, 720]);
        assert!(
            info.formats
                .iter()
                .all(|f| f.protocol == DownloadProtocol::M3u8)
        );
        // `expand_hls_url` inherits the seed's `ext` onto every expanded
        // variant, and `ext` feeds `%(ext)s` in output templates plus the
        // downstream container/remux decisions. The `.mp4` suffix on the
        // signed master describes the PROTOCOL's disguise, not the container
        // the renditions are in — every other HLS extractor seeds `m3u8`.
        assert!(
            info.formats.iter().all(|f| f.ext == "m3u8"),
            "expanded HLS renditions must carry ext=m3u8, got: {:?}",
            info.formats.iter().map(|f| &f.ext).collect::<Vec<_>>()
        );
        assert!(
            info.tags
                .as_ref()
                .is_some_and(|t| t.iter().any(|k| k == "Creampie"))
        );

        // Every metadata field `extract` populates is asserted, with the values
        // the committed fixture carries. Populating five fields and asserting
        // two lets a wiring slip on the rest ship green — and `view_count` is
        // the field whose loss motivated the json_ld cascade fix.
        assert_eq!(
            info.thumbnail.as_deref(),
            Some(
                "https://cdn77-t.pornoxo.com/b-pornoxo/thumbs/pxo-full/2026-08/91/\
                 ab16fa3919a1c1b5aeb6df569c0f938ff.mp4-full-7.jpg"
            )
        );
        // Pins `parse_iso8601_date`'s output SHAPE (YYYYMMDD), which nothing
        // else in this diff exercises — the fixture carries the ISO-8601
        // timestamp "2026-08-07T18:29:02+00:00".
        assert_eq!(info.upload_date.as_deref(), Some("20260807"));
        assert_eq!(info.view_count, Some(21));
        assert!(
            info.description
                .as_deref()
                .is_some_and(|d| d.starts_with("Big Tits, Amateur, Creampie")),
            "description: {:?}",
            info.description
        );
    }

    #[tokio::test]
    async fn rejects_a_master_url_that_fails_security_validation() {
        // A page whose playerConfig points at a private host must be refused
        // before any fetch — the URL is attacker-controlled page JavaScript.
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        let page = r#"<script>var playerConfig = { sources: {"hlsAuto":"http://169.254.169.254/latest/meta-data/"}, };</script>"#;
        let _m = server
            .mock("GET", "/videos/1/x/")
            .with_status(200)
            .with_body(page)
            .create_async()
            .await;

        let err = PornoxoExtractor::new()
            .extract(&format!("{base}/videos/1/x/"), &test_ctx())
            .await
            .expect_err("link-local master URL must be refused");
        assert!(matches!(err, RdlpError::Extraction { .. }));
        // Pin the REASON, not just that something failed: without this the test
        // would pass on any unrelated error (a failed page fetch, a parse slip)
        // and stop guarding the SSRF gate at all.
        assert!(
            err.to_string().contains("master URL rejected"),
            "must fail at the security gate, got: {err}"
        );
    }

    #[tokio::test]
    async fn errors_when_page_has_no_player_config() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        let _m = server
            .mock("GET", "/videos/1/x/")
            .with_status(200)
            .with_body("<html><body>gone</body></html>")
            .create_async()
            .await;

        let err = PornoxoExtractor::new()
            .extract(&format!("{base}/videos/1/x/"), &test_ctx())
            .await
            .expect_err("a page with no playerConfig must error");
        assert!(err.to_string().contains("playerConfig"), "got: {err}");
    }
}

/// Tests for the listing/search path: the three site behaviours a naive
/// implementation gets wrong, driven end to end through `fetch_page` against a
/// mockito origin (the seam established for PornHub in issue #457).
#[cfg(test)]
mod paged_search_tests {
    use super::*;
    use crate::base::common::SearchOrigin;
    use crate::hls::test_support::test_ctx;

    /// Declared here rather than reused from `search.rs`'s test module: a
    /// `const` inside a `#[cfg(test)] mod tests` is not in scope from here.
    const TAG_PAGE: &str = include_str!("tests/pornoxo_tag_page.html");

    /// The last page this fixture's `>>` anchor advertises. Read from the
    /// committed capture, not assumed.
    const FIXTURE_MAX_PAGE: u32 = 37;

    fn query(q: &str, filters: &[(&str, &str)]) -> SearchQuery {
        SearchQuery {
            query: q.to_owned(),
            filters: filters
                .iter()
                .map(|(k, v)| SearchFilter {
                    key: (*k).to_owned(),
                    value: (*v).to_owned(),
                })
                .collect(),
            max_results: None,
            page: None,
        }
    }

    /// One helper for every route rather than a per-route trio: the route is
    /// just a filter, so three near-identical helpers would be three copies of
    /// the same three lines.
    async fn page_against(
        server: &mockito::Server,
        filters: &[(&str, &str)],
        page: u32,
    ) -> Result<SearchPage> {
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");
        PornoxoExtractor::with_origin(origin)
            .fetch_page(&query("creampie", filters), page, &test_ctx())
            .await
    }

    async fn serving(status: usize, body: &str) -> (mockito::ServerGuard, mockito::Mock) {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(status)
            .with_body(body)
            .create_async()
            .await;
        (server, mock)
    }

    /// Behaviour 1. `/search/?q=` answers a nonsense query with HTTP 404 AND a
    /// fully populated 52-row grid of filler. Parsing that body would hand the
    /// operator 52 confident, wrong results, so the status must decide.
    #[tokio::test]
    async fn a_404_with_a_populated_grid_is_an_error_not_52_results() {
        let (server, _m) = serving(404, TAG_PAGE).await;

        let err = page_against(&server, &[], 1)
            .await
            .expect_err("a 404 must not be parsed, however full its body is");
        assert!(
            matches!(err, RdlpError::Http { status: 404, .. }),
            "must fail on the STATUS, not on some later parse step: {err:?}"
        );
    }

    /// Behaviour 2. `?page=999` answers HTTP 200 with page 1's videos, so an
    /// explicitly requested out-of-range page must be refused against the `>>`
    /// bound rather than returned as if it were page 999.
    #[tokio::test]
    async fn refuses_a_page_beyond_the_pager_maximum() {
        let (server, _m) = serving(200, TAG_PAGE).await;

        let err = page_against(&server, &[], 999)
            .await
            .expect_err("an out-of-range page must be refused, not silently clamped");
        let msg = err.to_string();
        assert!(msg.contains("999"), "must name the requested page: {msg}");
        assert!(msg.contains("37"), "must name the bound it exceeded: {msg}");
    }

    /// The boundary itself, both sides. A `>=` slip would still refuse 999 and
    /// pass the test above while wrongly rejecting the real last page, so the
    /// only assertions that pin the comparison are max and max+1.
    #[tokio::test]
    async fn accepts_the_last_page_and_refuses_the_one_after_it() {
        let (server, _m) = serving(200, TAG_PAGE).await;

        let last = page_against(&server, &[], FIXTURE_MAX_PAGE).await;
        assert!(
            last.is_ok(),
            "page {FIXTURE_MAX_PAGE} IS the last page and must be served: {:?}",
            last.err()
        );
        assert!(
            page_against(&server, &[], FIXTURE_MAX_PAGE + 1)
                .await
                .is_err(),
            "page {} is one past the end and must be refused",
            FIXTURE_MAX_PAGE + 1
        );
    }

    /// The low end of the same failure the max-page guard covers. `?page=0`
    /// answers HTTP 200 with page 1's grid, and without a floor
    /// `search_page_response` echoes `SearchPageResponse { page: 0, .. }` —
    /// page 1's videos labelled page 0, which is the confidently-wrong answer
    /// the high-end guard exists to prevent.
    ///
    /// `clamp_page`'s default is identity and a universal `.max(1)` would be
    /// wrong for 0-indexed sites, so this is an opt-in override; ABXXX is the
    /// in-tree precedent (`abxxx/search.rs`). `rdlp-cli` currently hardcodes
    /// `page: Some(1)`, but `SearchQuery` is a public API type and a desktop
    /// or embedder call can supply `Some(0)`.
    #[tokio::test]
    async fn page_zero_is_clamped_to_the_first_page_not_echoed_back() {
        let (server, _m) = serving(200, TAG_PAGE).await;
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");
        let mut q = query("creampie", &[]);
        q.page = Some(0);

        let resp = PornoxoExtractor::with_origin(origin)
            .search_page_response(&q, &test_ctx())
            .await
            .expect("page 0 must be served, not refused");
        assert_eq!(
            resp.page, 1,
            "page 0 must be reported as the page actually served"
        );
        assert_eq!(resp.results.len(), 52);
    }

    /// Behaviour 3, route-specific. The search route is Cloudflare-gated and a
    /// browser-solved clearance transplants into our client, so the operator
    /// needs that advice rather than a bare HTTP 403.
    #[tokio::test]
    async fn a_403_on_the_search_route_names_cookies_from_browser() {
        let (server, _m) = serving(403, "<title>Just a moment...</title>").await;

        let err = page_against(&server, &[("route", "search")], 1)
            .await
            .expect_err("a challenged search must error");
        let msg = err.to_string();
        assert!(msg.contains("--cookies-from-browser"), "got: {msg}");
        assert!(msg.contains("Cloudflare"), "got: {msg}");
        assert!(
            msg.contains("route=tag"),
            "must offer the open route: {msg}"
        );
    }

    /// The same status on the TAG route is not a clearance problem. The tag
    /// route is cookie-free, so claiming otherwise would send the operator
    /// chasing a cookie that would not have helped.
    #[tokio::test]
    async fn a_403_on_the_tag_route_does_not_mention_cookies() {
        let (server, _m) = serving(403, "nope").await;

        let err = page_against(&server, &[("route", "tag")], 1)
            .await
            .expect_err("a 403 must still be an error on the tag route");
        let msg = err.to_string();
        assert!(!msg.contains("--cookies-from-browser"), "got: {msg}");
        assert!(
            matches!(err, RdlpError::Http { status: 403, .. }),
            "the tag route must propagate the raw status: {err:?}"
        );
    }

    #[tokio::test]
    async fn reports_has_more_from_the_next_link() {
        let (server, _m) = serving(200, TAG_PAGE).await;

        let page = page_against(&server, &[], 1)
            .await
            .expect("page 1 must parse");
        assert_eq!(page.results.len(), 52);
        assert!(page.has_more, "page 1 of 37 has a Next anchor");
        assert_eq!(
            page.total_estimate, None,
            "the site publishes no result count"
        );
    }

    /// The DELIVERY pin for issue #658's acceptance criterion, one level above
    /// `fetch_page`.
    ///
    /// Mapping the 403 is worthless if the scaffold then swallows it. Before
    /// the `search_all_pages` fix this returned `Ok(vec![])` and the operator
    /// saw "no results found" for a site that was merely gated, with the
    /// guidance discarded into a `debug!` they had not enabled. Every site
    /// wires `SearchExtractor::search` straight to `search_all_pages` as a
    /// one-line delegation (see `xtits/search.rs`), so this pins the whole
    /// delivery path bar that one line, which lands with the impl in G3.
    #[tokio::test]
    async fn the_cloudflare_guidance_survives_all_the_way_out_of_search() {
        let (server, _m) = serving(403, "<title>Just a moment...</title>").await;
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");

        let err = PornoxoExtractor::with_origin(origin)
            .search_all_pages(&query("creampie", &[]), &test_ctx())
            .await
            .expect_err("a gated search must not be reported as zero results");

        let msg = err.to_string();
        assert!(msg.contains("--cookies-from-browser"), "got: {msg}");
        assert!(msg.contains("Cloudflare"), "got: {msg}");
        assert!(msg.contains("route=tag"), "got: {msg}");
    }

    /// The 404-with-a-full-grid refusal must reach the operator too — the same
    /// swallow hid it.
    #[tokio::test]
    async fn the_404_refusal_survives_all_the_way_out_of_search() {
        let (server, _m) = serving(404, TAG_PAGE).await;
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");

        let err = PornoxoExtractor::with_origin(origin)
            .search_all_pages(&query("creampie", &[]), &test_ctx())
            .await
            .expect_err("a 404 must not be reported as zero results");
        assert!(
            matches!(err, RdlpError::Http { status: 404, .. }),
            "got: {err:?}"
        );
    }

    /// Pins `SearchExtractor::search` itself, not `search_all_pages` one level
    /// down. G2 could only prove the Cloudflare guidance survives
    /// `search_all_pages`, because `SearchExtractor` was not implemented yet;
    /// that gap is exactly how a prior site once wired its `search` entry
    /// point to something that silently swallowed the error into `Ok(vec![])`
    /// while every unit test one level down stayed green. Drives the trait
    /// method an operator's `--search-site pornoxo` actually calls.
    #[tokio::test]
    async fn search_extractor_search_delegates_and_the_cloudflare_guidance_survives() {
        let (server, _m) = serving(403, "<title>Just a moment...</title>").await;
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");

        let err = rdlp_core::SearchExtractor::search(
            &PornoxoExtractor::with_origin(origin),
            &query("creampie", &[]),
            &test_ctx(),
        )
        .await
        .expect_err("a gated search must not be reported as zero results");

        let msg = err.to_string();
        assert!(msg.contains("--cookies-from-browser"), "got: {msg}");
        assert!(msg.contains("Cloudflare"), "got: {msg}");
        assert!(msg.contains("route=tag"), "got: {msg}");
    }

    /// Pins that `SearchExtractor::search` runs the FULL `search_all_pages`
    /// pagination loop, not a single-page adapter. A compatible-but-lossy
    /// wiring — `self.search_page_response(query, ctx).await.map(|r| r.results)`
    /// — compiles, and both the 403-guidance delegation tests above still pass
    /// (a first-page failure propagates identically through either path), but
    /// it silently caps `search()` at one page, discarding pagination and the
    /// `max_results` aggregation loop entirely. `TAG_PAGE` serves the same
    /// 52-row grid regardless of the requested page (`Matcher::Any`) with
    /// `has_next: true` (37-page pager), so requesting `max_results: Some(60)`
    /// forces a second fetch to satisfy the cap — a single-page adapter can
    /// never clear 52 results, only the aggregating loop can reach 60.
    #[tokio::test]
    async fn search_extractor_search_aggregates_results_across_pages_not_just_one() {
        let (server, _m) = serving(200, TAG_PAGE).await;
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");

        let mut q = query("creampie", &[]);
        q.max_results = Some(60);

        let results = rdlp_core::SearchExtractor::search(
            &PornoxoExtractor::with_origin(origin),
            &q,
            &test_ctx(),
        )
        .await
        .expect("a healthy multi-page search must succeed");

        assert_eq!(
            results.len(),
            60,
            "must aggregate a second page to reach the requested cap, not stop at \
             one page's 52 results"
        );
    }

    /// Same pin for the single-page entry point `SearchExtractor::search_page`.
    #[tokio::test]
    async fn search_extractor_search_page_delegates_and_the_cloudflare_guidance_survives() {
        let (server, _m) = serving(403, "<title>Just a moment...</title>").await;
        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");

        let err = rdlp_core::SearchExtractor::search_page(
            &PornoxoExtractor::with_origin(origin),
            &query("creampie", &[]),
            &test_ctx(),
        )
        .await
        .expect_err("a gated search page must not be reported as zero results");

        let msg = err.to_string();
        assert!(msg.contains("--cookies-from-browser"), "got: {msg}");
        assert!(msg.contains("Cloudflare"), "got: {msg}");
        assert!(msg.contains("route=tag"), "got: {msg}");
    }

    /// `search_all_pages` runs filters through `validate_search_filters`, so a
    /// typo is refused before any request is made.
    #[tokio::test]
    async fn an_unknown_filter_is_refused_before_fetching() {
        let mut server = mockito::Server::new_async().await;
        let never = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_body(TAG_PAGE)
            .expect(0)
            .create_async()
            .await;

        let origin = SearchOrigin::new(&server.url()).expect("mockito origin is well formed");
        let err = PornoxoExtractor::with_origin(origin)
            .search_all_pages(&query("x", &[("sort", "newest")]), &test_ctx())
            .await
            .expect_err("an invalid sort must be refused");
        assert!(err.to_string().contains("newest"), "got: {err}");
        never.assert_async().await;
    }
}
