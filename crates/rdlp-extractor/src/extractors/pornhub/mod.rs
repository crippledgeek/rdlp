//! PornHub extractor module
//!
//! This module provides extraction support for PornHub videos and playlists.
//!
//! # Architecture
//!
//! The extractor is split into focused submodules:
//! - `patterns` - URL patterns and regex definitions
//! - `formats` - Format extraction from various sources
//! - `playlist` - Playlist pagination and extraction
//! - `search` - Search result parsing and filter validation
//! - `search_patterns` - URL builders and constants for search API
//! - `utils` - Helper functions for parsing and validation
//!
//! # Supported URLs
//!
//! - Videos: `https://www.pornhub.com/view_video.php?viewkey=ph123`
//! - Playlists: `https://www.pornhub.com/playlist/123456`
//! - Embed: `https://www.pornhub.com/embed/ph123`
//! - Thumbzilla: `https://www.thumbzilla.com/video/ph123/title`

mod formats;
mod patterns;
mod playlist;
mod search;
mod search_html;
mod search_patterns;
#[cfg(test)]
mod tests;
mod utils;

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, SearchExtractor};
use rdlp_types::{InfoDict, SearchPageResponse, SearchQuery, SearchResultPreview};
use scraper::Html;

use crate::base::common::{BaseExtractor, PagedSearch, SearchOrigin, SearchPage};
use crate::hls::detect_format_sizes_lazy;

pub use patterns::{PORNHUB_PLAYLIST_URL_PATTERN, PORNHUB_VIDEO_URL_PATTERN};

/// Expected number of results per API page. Used to detect the last page:
/// if a page returns fewer than this, there are no more pages.
const API_RESULTS_PER_PAGE: usize = 20;

/// PornHub extractor
///
/// Supports:
/// - Single videos: `https://www.pornhub.com/view_video.php?viewkey=ph123`
/// - Playlists: `https://www.pornhub.com/playlist/123456`
///
/// # Example
///
/// ```no_run
/// use rdlp_extractor::PornHubExtractor;
/// use rdlp_core::InfoExtractor;
///
/// let extractor = PornHubExtractor::new();
/// assert!(extractor.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
/// ```
pub struct PornHubExtractor {
    /// Search origin (scheme+authority). Production literal by default;
    /// test-injected to a mockito origin via `with_origin` (issue #457).
    origin: SearchOrigin,
}

impl PornHubExtractor {
    /// Create a new PornHub extractor
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: search_patterns::default_origin(),
        }
    }

    /// Test-only: point the search builders at a mockito origin.
    #[cfg(test)]
    pub(crate) fn with_origin(origin: SearchOrigin) -> Self {
        Self { origin }
    }
}

impl Default for PornHubExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl PornHubExtractor {
    /// Fetch and parse a single API search page.
    ///
    /// # Arguments
    /// * `url` - Full API search URL for this page.
    /// * `ctx` - Extraction context with HTTP client.
    ///
    /// # Returns
    /// Parsed `SearchResultPreview` items for this page.
    async fn fetch_api_search_page(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let body = BaseExtractor::fetch_webpage_with_retry(url, ctx).await?;
        search::parse_api_search_results(&body)
    }

    /// Fetch a single HTML search page and parse it.
    ///
    /// # Arguments
    /// * `url` - HTML search URL (`/video/search?search=…&page=…`).
    /// * `ctx` - Extraction context with HTTP client.
    async fn fetch_html_search_page(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let body = BaseExtractor::fetch_webpage_with_retry(url, ctx).await?;
        search_html::parse_html_search_results(&body)
    }
}

impl PagedSearch for PornHubExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[PornHub]"
    }

    fn validate_search_filters(&self, filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        search::validate_search_filters(filters)
    }

    /// Fetch + parse ONE search page (loop semantics): HTML-primary, with an
    /// API fallback ONLY on page 1. `has_more` folds the old
    /// `Termination::UntilEmpty` — a non-empty page keeps the loop going; an
    /// empty page breaks in the shared loop before `has_more` is consulted.
    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let html_url = search_patterns::build_html_search_url(&self.origin, &query.query, page);
        match self.fetch_html_search_page(&html_url, ctx).await {
            Ok(results) if !results.is_empty() => Ok(SearchPage {
                results,
                has_more: true,
                total_estimate: None,
            }),
            outcome => {
                if page == 1 {
                    let base_url = search_patterns::build_api_search_url(
                        &self.origin,
                        &query.query,
                        &query.filters,
                    );
                    match self.fetch_api_search_page(&base_url, ctx).await {
                        Ok(api_results) => {
                            let has_more = !api_results.is_empty();
                            Ok(SearchPage {
                                results: api_results,
                                has_more,
                                total_estimate: None,
                            })
                        }
                        Err(api_err) => {
                            // Preserve the operator-visible WARN (never downgrade
                            // to the shared loop's DEBUG). Then propagate → the
                            // loop breaks with partial results, as today.
                            warn!("[PornHub] API fallback also failed on page 1: {api_err}");
                            Err(api_err)
                        }
                    }
                } else {
                    match outcome {
                        Ok(empty) => Ok(SearchPage {
                            results: empty,
                            has_more: false,
                            total_estimate: None,
                        }),
                        Err(e) => Err(e),
                    }
                }
            }
        }
    }

    /// Single-page semantics differ from the loop: the API fallback runs on
    /// ANY page (with a paged API URL), and `has_more` derives from the API
    /// page-size heuristic. Overrides the shared assembler to preserve this.
    async fn search_page_response(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        search::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1);
        let html_url = search_patterns::build_html_search_url(&self.origin, &query.query, page);

        let page_results = match self.fetch_html_search_page(&html_url, ctx).await {
            Ok(results) if !results.is_empty() => results,
            outcome => {
                let reason = match &outcome {
                    Ok(_) => "HTML returned 0 results".to_string(),
                    Err(e) => format!("{e}"),
                };
                debug!(
                    "[PornHub] HTML search failed/empty on page {page}, falling back to API: {reason}"
                );
                let base_url = search_patterns::build_api_search_url(
                    &self.origin,
                    &query.query,
                    &query.filters,
                );
                let page_url = if page == 1 {
                    base_url
                } else {
                    search_patterns::build_api_search_url_page(&base_url, page)
                };
                self.fetch_api_search_page(&page_url, ctx).await?
            }
        };

        let has_more = page_results.len() >= API_RESULTS_PER_PAGE;
        Ok(SearchPageResponse {
            results: page_results,
            page,
            has_more,
            total_estimate: None,
        })
    }
}

#[async_trait]
impl SearchExtractor for PornHubExtractor {
    fn name(&self) -> &str {
        "PornHub"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        search_patterns::search_filter_descriptors()
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

#[async_trait]
impl InfoExtractor for PornHubExtractor {
    fn name(&self) -> &str {
        "PornHub"
    }

    fn valid_url(&self) -> &regex::Regex {
        &PORNHUB_VIDEO_URL_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let host = utils::extract_host(url);

        // Set age verification cookies
        utils::set_age_cookies(&host, ctx).await?;

        // Fetch the webpage using BaseExtractor (handles errors, verbose logging)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Check for video unavailability errors
        if let Some(error_msg) = utils::detect_video_unavailable(&webpage) {
            return Err(RdlpError::Extraction {
                message: error_msg,
                url: Some(url.to_string().into()),
            });
        }

        // Get video ID
        let video_id = patterns::extract_video_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!(
                "Could not extract video ID: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        })?;

        // Parse HTML and extract all metadata before async operations
        // Extract duration from flashvars (before HTML parsing drops webpage borrow)
        let duration = utils::extract_duration(&webpage);

        let (
            title,
            description,
            thumbnail,
            uploader,
            uploader_url,
            channel,
            channel_url,
            view_count,
            average_rating,
        ) = {
            let html = Html::parse_document(&webpage);
            (
                utils::extract_title(&html, &webpage),
                utils::extract_description(&html),
                utils::extract_thumbnail(&html, &webpage),
                utils::extract_uploader(&html),
                utils::extract_uploader_url(&html),
                utils::extract_channel(&html),
                utils::extract_channel_url(&html),
                utils::extract_view_count(&html),
                utils::extract_rating(&html),
            )
        }; // html is dropped here before await

        // Extract formats with fallback strategies
        let formats = formats::extract_all_formats(&webpage, ctx).await?;

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!(
                    "No video formats found for URL: {}",
                    rdlp_redact::RedactedUrl::new(&url)
                ),
                url: Some(url.to_string().into()),
            });
        }

        // Pre-resolve HLS variant playlists into per-variant Format rows with
        // fragments populated, so the downloader's pre-resolved-fragments path
        // is taken. MUST run before `detect_format_sizes_lazy` (issue #269/#279).
        let formats = crate::hls::expand_hls_in_place(formats, ctx.http_client.clone()).await;

        // Detect file sizes and segment counts
        let extractor_name = InfoExtractor::name(self);
        let (formats_with_size, hls_flags) =
            detect_format_sizes_lazy(formats, ctx, extractor_name).await;

        // Build InfoDict with all metadata
        let mut info = InfoDict::new(video_id, title, extractor_name, url);
        info.description = description;
        info.thumbnail = thumbnail;
        info.uploader = uploader;
        info.uploader_url = uploader_url;
        info.channel = channel;
        info.channel_url = channel_url;
        info.view_count = view_count;
        info.average_rating = average_rating;
        info.duration = duration;
        info.age_limit = Some(18);
        info.formats = formats_with_size;
        info.propagate_duration();

        // Set stream-level flags from HLS detection
        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        Ok(info)
    }

    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        if !patterns::is_playlist_url(url) {
            return Ok(vec![self.extract(url, ctx).await?]);
        }

        playlist::extract_playlist(self, url, ctx).await
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Golden tests pinning the PornHub HTML→API fallback divergence (loop vs
/// single-page) against a mockito origin, per issue #457.
#[cfg(test)]
mod origin_golden {
    use super::*;
    use crate::base::common::SearchOrigin;
    use crate::hls::test_support::test_ctx;
    use mockito::Matcher;
    use rdlp_types::SearchQuery;

    // P1 — loop (search()) API fallback targets the BASE url (no &page=), only on page 1.
    #[tokio::test]
    async fn search_loop_api_fallback_uses_base_url_not_paged() {
        let mut server = mockito::Server::new_async().await;
        // HTML page 1 returns empty -> loop falls back to API base.
        let html = server
            .mock("GET", Matcher::Regex(r"/video/search".into()))
            .with_status(200)
            .with_body("<html></html>") // 0 cards -> empty -> triggers fallback
            .create_async()
            .await;
        // API BASE mock: matches the webmasters path WITHOUT a page param.
        let api_base = server
            .mock("GET", Matcher::Regex(r"/webmasters/search".into()))
            // mockito matches the whole query string as one blob (no per-key
            // presence check), so "no &page=" is expressed as the exact,
            // deterministic base query (empty filters => fixed param order).
            .match_query(Matcher::Exact(
                "search=x&output=json&thumbsize=large".into(),
            ))
            .with_status(200)
            .with_body("{\"videos\":[]}") // empty result set — this test asserts on which URL was hit, not on parse output
            .expect_at_least(1)
            .create_async()
            .await;
        // Paged API mock: must NEVER be hit by the loop.
        let api_paged = server
            .mock("GET", Matcher::Regex(r"/webmasters/search".into()))
            .match_query(Matcher::UrlEncoded("page".into(), "2".into()))
            .with_status(200)
            .with_body("{\"videos\":[]}")
            .expect(0)
            .create_async()
            .await;

        let extractor = PornHubExtractor::with_origin(SearchOrigin::new(&server.url()).unwrap());
        let query = SearchQuery {
            query: "x".into(),
            filters: vec![],
            page: None,
            max_results: Some(50),
        };
        let _ = extractor.search(&query, &test_ctx()).await;

        html.assert_async().await;
        api_base.assert_async().await;
        api_paged.assert_async().await; // expect(0): fails if the loop ever paged the fallback
    }

    // P3 boundary — single (search_page) API fallback: page 1 -> base, page 2 -> &page=2.
    #[tokio::test]
    async fn search_page_api_fallback_pages_beyond_page_one() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", Matcher::Regex(r"/video/search".into()))
            .with_status(200)
            .with_body("<html></html>")
            .create_async()
            .await;
        let paged = server
            .mock("GET", Matcher::Regex(r"/webmasters/search".into()))
            .match_query(Matcher::UrlEncoded("page".into(), "2".into()))
            .with_status(200)
            .with_body("{\"videos\":[]}")
            .expect(1)
            .create_async()
            .await;

        let extractor = PornHubExtractor::with_origin(SearchOrigin::new(&server.url()).unwrap());
        let query = SearchQuery {
            query: "x".into(),
            filters: vec![],
            page: Some(2),
            max_results: None,
        };
        let _ = extractor.search_page(&query, &test_ctx()).await;

        paged.assert_async().await; // single-page path MUST use the paged builder on page 2
    }

    #[tokio::test]
    async fn search_page_api_fallback_page_one_uses_base() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", Matcher::Regex(r"/video/search".into()))
            .with_status(200)
            .with_body("<html></html>")
            .create_async()
            .await;
        let base = server
            .mock("GET", Matcher::Regex(r"/webmasters/search".into()))
            // See the loop-fallback test above: "no &page=" is expressed as
            // the exact deterministic base query, not `Matcher::Missing`
            // (mockito's Missing requires a fully empty query string).
            .match_query(Matcher::Exact(
                "search=x&output=json&thumbsize=large".into(),
            ))
            .with_status(200)
            .with_body("{\"videos\":[]}")
            .expect(1)
            .create_async()
            .await;

        let extractor = PornHubExtractor::with_origin(SearchOrigin::new(&server.url()).unwrap());
        let query = SearchQuery {
            query: "x".into(),
            filters: vec![],
            page: Some(1),
            max_results: None,
        };
        let _ = extractor.search_page(&query, &test_ctx()).await;

        base.assert_async().await;
    }
}
