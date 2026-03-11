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
mod search_patterns;
mod utils;

use std::time::Duration;

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{
    ExponentialBuilder, ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result, Retryable,
    SearchExtractor, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use scraper::Html;

use crate::base::common::{BaseExtractor, MAX_PLAYLIST_SIZE};
use crate::hls::detect_format_sizes;

pub use patterns::{PORNHUB_PLAYLIST_URL_PATTERN, PORNHUB_VIDEO_URL_PATTERN};

/// Rate limit between search page fetches (milliseconds).
const SEARCH_RATE_LIMIT_MS: u64 = 500;

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
        let response = (|| async { ctx.http_client.get(url).send().await })
            .retry(
                ExponentialBuilder::default()
                    .with_max_times(2)
                    .with_min_delay(Duration::from_millis(500)),
            )
            .when(|e| e.is_timeout() || e.is_connect())
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch PornHub search API: {e}")))?;

        rdlp_core::check_http_response(&response)?;

        let body = response
            .text()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to read PornHub API response: {e}")))?;

        search::parse_api_search_results(&body)
    }

    /// Fetch a single HTML search page (fallback).
    ///
    /// HTML parsing is not yet implemented — returns an error so callers
    /// propagate the original API failure instead of silently returning
    /// empty results. A full HTML parser can be added later if the API
    /// becomes unreliable.
    ///
    /// # Arguments
    /// * `_url` - HTML search URL (unused until HTML parsing is implemented).
    /// * `_ctx` - Extraction context (unused until HTML parsing is implemented).
    async fn fetch_html_search_page(
        &self,
        _url: &str,
        _ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        Err(RdlpError::Extraction(
            "PornHub HTML search fallback not yet implemented".to_string(),
        ))
    }

    /// Paginated search across all pages, collecting up to `max_results` results.
    ///
    /// Uses the JSON API as the primary source with a 500ms rate limit between pages.
    /// Falls back to the HTML search page on page 1 API failures.
    ///
    /// # Arguments
    /// * `query` - Search query with optional filters and result cap.
    /// * `ctx` - Extraction context with HTTP client.
    async fn search_all_pages(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        search::validate_search_filters(&query.filters)?;

        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut page = 1_u32;

        let base_url = search_patterns::build_api_search_url(&query.query, &query.filters);

        loop {
            let page_url = if page == 1 {
                base_url.clone()
            } else {
                search_patterns::build_api_search_url_page(&base_url, page)
            };

            debug!(page, url:? = rdlp_security::sanitize_for_logging(&page_url); "[PornHub] Fetching search page");

            let page_results = match self.fetch_api_search_page(&page_url, ctx).await {
                Ok(results) => results,
                Err(e) => {
                    if page == 1 {
                        debug!("[PornHub] API search failed, trying HTML fallback: {e}");
                        let html_url = search_patterns::build_html_search_url(&query.query, 1);
                        match self.fetch_html_search_page(&html_url, ctx).await {
                            Ok(results) => results,
                            Err(html_err) => {
                                warn!("[PornHub] HTML fallback also failed: {html_err}");
                                break;
                            }
                        }
                    } else {
                        debug!(
                            page;
                            "[PornHub] Failed to fetch search page, returning partial results: {e}"
                        );
                        break;
                    }
                }
            };

            if page_results.is_empty() {
                debug!(page; "[PornHub] No results on page, stopping pagination");
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(SEARCH_RATE_LIMIT_MS)).await;
        }

        debug!(
            count = all_results.len(), pages = page;
            "[PornHub] Search complete"
        );

        Ok(all_results)
    }
}

#[async_trait]
impl SearchExtractor for PornHubExtractor {
    fn name(&self) -> &str {
        "PornHub"
    }

    fn supported_filters(&self) -> Vec<rdlp_core::SearchFilterDescriptor> {
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
        let base_url = search_patterns::build_api_search_url(&query.query, &query.filters);

        let page_url = if page == 1 {
            base_url
        } else {
            search_patterns::build_api_search_url_page(&base_url, page)
        };

        let page_results = match self.fetch_api_search_page(&page_url, ctx).await {
            Ok(results) => results,
            Err(e) => {
                if page == 1 {
                    debug!("[PornHub] API search failed, trying HTML fallback: {e}");
                    let html_url = search_patterns::build_html_search_url(&query.query, 1);
                    self.fetch_html_search_page(&html_url, ctx).await?
                } else {
                    return Err(e);
                }
            }
        };

        let has_more = !page_results.is_empty() && page_results.len() >= API_RESULTS_PER_PAGE;

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
            return Err(RdlpError::Extraction(error_msg));
        }

        // Get video ID
        let video_id = patterns::extract_video_id(url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract video ID: {url}")))?;

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
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Detect file sizes and segment counts
        let extractor_name = InfoExtractor::name(self);
        let (formats_with_size, hls_flags) =
            detect_format_sizes(formats, ctx, extractor_name).await;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = PornHubExtractor::new();
        assert_eq!(InfoExtractor::name(&extractor), "PornHub");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = PornHubExtractor::new();

        // Video URLs
        assert!(extractor.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
        assert!(extractor.suitable("https://www.pornhub.com/embed/ph456"));
        assert!(extractor.suitable("https://de.pornhub.com/view_video.php?viewkey=ph789"));

        // Playlist URLs
        assert!(extractor.suitable("https://www.pornhub.com/playlist/123456"));

        // Invalid URLs
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_pornhub_implements_search_extractor() {
        let extractor = PornHubExtractor::new();
        let filters =
            <PornHubExtractor as rdlp_core::SearchExtractor>::supported_filters(&extractor);
        assert!(!filters.is_empty());
        assert_eq!(
            <PornHubExtractor as rdlp_core::SearchExtractor>::name(&extractor),
            "PornHub"
        );
    }

    #[test]
    fn test_search_filters_have_ordering() {
        let extractor = PornHubExtractor::new();
        let filters =
            <PornHubExtractor as rdlp_core::SearchExtractor>::supported_filters(&extractor);
        let ordering = filters.iter().find(|f| f.key == "ordering");
        assert!(ordering.is_some());
        assert_eq!(ordering.unwrap().allowed_values.len(), 4);
    }

    #[test]
    fn test_search_filters_have_period() {
        let extractor = PornHubExtractor::new();
        let filters =
            <PornHubExtractor as rdlp_core::SearchExtractor>::supported_filters(&extractor);
        let period = filters.iter().find(|f| f.key == "period");
        assert!(period.is_some());
        assert_eq!(period.unwrap().allowed_values.len(), 3);
    }

    #[test]
    fn test_search_filters_have_category() {
        let extractor = PornHubExtractor::new();
        let filters =
            <PornHubExtractor as rdlp_core::SearchExtractor>::supported_filters(&extractor);
        let category = filters.iter().find(|f| f.key == "category");
        assert!(category.is_some());
        assert!(!category.unwrap().allowed_values.is_empty());
    }
}
