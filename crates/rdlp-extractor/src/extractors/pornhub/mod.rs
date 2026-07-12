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

use crate::base::common::{BaseExtractor, PaginatedSearch, Termination};
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
pub struct PornHubExtractor;

impl PornHubExtractor {
    /// Create a new PornHub extractor
    #[must_use]
    pub fn new() -> Self {
        Self
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

impl PaginatedSearch for PornHubExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[PornHub]"
    }

    fn validate_search_filters(&self, filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        search::validate_search_filters(filters)
    }

    async fn fetch_search_page(
        &self,
        query: &SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<SearchResultPreview>, Termination)> {
        let html_url = search_patterns::build_html_search_url(&query.query, page as u32);
        match self.fetch_html_search_page(&html_url, ctx).await {
            Ok(results) if !results.is_empty() => Ok((results, Termination::UntilEmpty)),
            outcome => {
                if page == 1 {
                    let base_url =
                        search_patterns::build_api_search_url(&query.query, &query.filters);
                    match self.fetch_api_search_page(&base_url, ctx).await {
                        Ok(api_results) => Ok((api_results, Termination::UntilEmpty)),
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
                        Ok(empty) => Ok((empty, Termination::UntilEmpty)),
                        Err(e) => Err(e),
                    }
                }
            }
        }
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
        search::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1);
        let html_url = search_patterns::build_html_search_url(&query.query, page);

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
                let base_url = search_patterns::build_api_search_url(&query.query, &query.filters);
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
