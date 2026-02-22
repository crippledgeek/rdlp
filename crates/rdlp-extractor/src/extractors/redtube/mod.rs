//! RedTube extractor
//!
//! Supports URLs like:
//! - https://www.redtube.com/123456
//! - https://www.redtube.com.br/123456
//! - https://embed.redtube.com/?id=123456
//!
//! RedTube embeds video sources in JavaScript objects rather than HTML `<source>` tags,
//! so this extractor uses regex to extract JSON from the page source.
//!
//! ## Module Structure
//!
//! - `patterns` - URL and extraction regex patterns
//! - `formats` - Format extraction from JavaScript sources and mediaDefinition
//! - `search` - Search result parsing and filter validation

mod formats;
mod patterns;
mod search;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result, SearchExtractor};
use regex::Regex;
use scraper::Html;
use std::time::Duration;

use crate::base::common::{BaseExtractor, MAX_PLAYLIST_SIZE};
use crate::base::tnaflix_network::TnaFlixNetworkBase;
use crate::hls::detect_format_sizes;
use crate::utils::make_absolute_url;
use patterns::REDTUBE_URL_PATTERN;

/// Rate limit delay between search page fetches (500ms).
const SEARCH_RATE_LIMIT_MS: u64 = 500;

/// RedTube extractor
pub struct RedTubeExtractor {
    base: TnaFlixNetworkBase,
}

impl RedTubeExtractor {
    /// Create a new RedTube extractor
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Extract video ID from URL using BaseExtractor utility
    fn extract_id(&self, url: &str) -> Option<String> {
        BaseExtractor::extract_id_from_url(url, &REDTUBE_URL_PATTERN, "id")
    }
}

impl Default for RedTubeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for RedTubeExtractor {
    fn name(&self) -> &str {
        "RedTube"
    }

    fn valid_url(&self) -> &Regex {
        &REDTUBE_URL_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Fetch the webpage using BaseExtractor (handles errors, verbose logging)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Get video ID using BaseExtractor
        let video_id = self.extract_id(url).ok_or_else(|| {
            RdlpError::Extraction(format!("Could not extract video ID from URL: {url}"))
        })?;

        // Extract all data from HTML before any async operations
        let metadata = {
            let html = Html::parse_document(&webpage);
            self.base.extract_metadata(&html)?
        }; // html is dropped here

        // Try to extract video formats from JavaScript sources
        let mut formats = formats::extract_from_sources(&webpage);

        // If sources didn't work, try mediaDefinition
        if formats.is_empty() {
            formats = formats::extract_from_media_definition(&webpage, ctx).await;
        }

        // If both JavaScript methods failed, fall back to HTML <source> tags
        if formats.is_empty() {
            let video_data = {
                let html = Html::parse_document(&webpage);
                self.base.parse_video_sources(&html)
            };

            if !video_data.is_empty() {
                formats = self.base.build_formats(video_data, ctx).await;
            }
        }

        // Return error if still no sources found
        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video sources found in JavaScript or HTML. Video may be unavailable. URL: {url}"
            )));
        }

        // Convert relative URLs to absolute using utility
        for format in &mut formats {
            if !format.url.starts_with("http://") && !format.url.starts_with("https://") {
                format.url = make_absolute_url(url, &format.url);
            }
        }

        // Fetch sizes/segments for all formats in parallel
        let (formats, hls_flags) =
            detect_format_sizes(formats, ctx, InfoExtractor::name(self)).await;

        // Build InfoDict with all extracted metadata
        let mut info = InfoDict::new(video_id, metadata.title, InfoExtractor::name(self), url);
        info.description = metadata.description;
        info.uploader = metadata.uploader;
        info.uploader_id = metadata.uploader_id;
        info.uploader_url = metadata.uploader_url;
        info.channel = metadata.channel;
        info.channel_id = metadata.channel_id;
        info.channel_url = metadata.channel_url;
        info.thumbnail = metadata.thumbnail;
        info.thumbnails = metadata.thumbnails;
        info.duration = metadata.duration;
        info.upload_date = metadata.upload_date;
        info.view_count = metadata.view_count;
        info.like_count = metadata.like_count;
        info.average_rating = metadata.average_rating;
        info.tags = metadata.tags;
        info.categories = metadata.categories;
        info.age_limit = Some(18); // RedTube is adult content
        info.formats = formats;
        info.propagate_duration();

        // Set stream-level flags from HLS detection
        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        Ok(info)
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl RedTubeExtractor {
    /// Perform paginated search across all pages, collecting results.
    ///
    /// Uses the JSON API as the primary source. Falls back to HTML scraping
    /// if the API request fails. Rate-limits at 500ms between page fetches.
    /// Caps results at `max_results` or `MAX_PLAYLIST_SIZE`.
    async fn search_all_pages(
        &self,
        query: &rdlp_core::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_core::SearchResultPreview>> {
        let descriptors = patterns::search_filter_descriptors();
        search::validate_search_filters(&query.filters, &descriptors)?;

        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut page = 1_u32;
        let mut total_count: Option<u64> = None;

        let base_url = patterns::build_api_search_url(&query.query, &query.filters);

        loop {
            let page_url = if page == 1 {
                base_url.clone()
            } else {
                patterns::build_api_search_url_page(&base_url, page)
            };

            debug!(
                page;
                "[RedTube] Fetching search page"
            );

            // Try JSON API first
            let page_results = match self.fetch_api_search_page(&page_url, ctx).await {
                Ok((results, count)) => {
                    if total_count.is_none() {
                        total_count = count;
                    }
                    results
                }
                Err(e) => {
                    if page == 1 {
                        // First page failed, try HTML fallback
                        warn!("[RedTube] API search failed, falling back to HTML: {e}");
                        let html_url = patterns::build_html_search_url(&query.query);
                        match self.fetch_html_search_page(&html_url, ctx).await {
                            Ok(results) => results,
                            Err(html_err) => {
                                warn!("[RedTube] HTML fallback also failed: {html_err}");
                                break;
                            }
                        }
                    } else {
                        warn!(
                            page;
                            "[RedTube] Failed to fetch search page, returning partial results: {e}"
                        );
                        break;
                    }
                }
            };

            if page_results.is_empty() {
                debug!(page; "[RedTube] No results on page, stopping pagination");
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            // Check if we have more pages based on total count
            if let Some(total) = total_count {
                let fetched_so_far = page as u64 * u64::from(patterns::API_RESULTS_PER_PAGE);
                if fetched_so_far >= total {
                    break;
                }
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(SEARCH_RATE_LIMIT_MS)).await;
        }

        info!(
            count = all_results.len(), pages = page;
            "[RedTube] Search complete"
        );

        Ok(all_results)
    }

    /// Fetch and parse a single API search page.
    async fn fetch_api_search_page(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_core::SearchResultPreview>, Option<u64>)> {
        let response = ctx
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch search API: {e}")))?;

        rdlp_core::check_http_response(&response)?;

        let body = response
            .text()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to read search API response: {e}")))?;

        search::parse_api_search_results(&body)
    }

    /// Fetch and parse a single HTML search page (fallback).
    async fn fetch_html_search_page(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_core::SearchResultPreview>> {
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
        search::parse_html_search_results(&webpage)
    }
}

#[async_trait]
impl SearchExtractor for RedTubeExtractor {
    fn name(&self) -> &str {
        "RedTube"
    }

    fn supported_filters(&self) -> Vec<rdlp_core::SearchFilterDescriptor> {
        patterns::search_filter_descriptors()
    }

    async fn search(
        &self,
        query: &rdlp_core::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_core::SearchResultPreview>> {
        self.search_all_pages(query, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    /// Shared test fixture (compiled once, reused across all tests)
    static TEST_REDTUBE: LazyLock<RedTubeExtractor> = LazyLock::new(RedTubeExtractor::new);

    #[test]
    fn test_redtube_url_pattern() {
        let extractor = &*TEST_REDTUBE;
        assert!(extractor.suitable("https://www.redtube.com/123456"));
        assert!(extractor.suitable("https://redtube.com/12345678"));
        assert!(extractor.suitable("https://www.redtube.com.br/987654"));
        assert!(extractor.suitable("https://embed.redtube.com/?id=123456"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://www.tnaflix.com/video/123"));
    }

    #[test]
    fn test_extract_id() {
        let extractor = &*TEST_REDTUBE;

        let id1 = extractor.extract_id("https://www.redtube.com/123456");
        assert_eq!(id1, Some("123456".to_string()));

        let id2 = extractor.extract_id("https://redtube.com/12345678");
        assert_eq!(id2, Some("12345678".to_string()));

        let id3 = extractor.extract_id("https://www.redtube.com.br/987654");
        assert_eq!(id3, Some("987654".to_string()));

        let id4 = extractor.extract_id("https://embed.redtube.com/?id=555555");
        assert_eq!(id4, Some("555555".to_string()));
    }

    #[test]
    fn test_extractor_name() {
        let extractor = &*TEST_REDTUBE;
        assert_eq!(InfoExtractor::name(extractor), "RedTube");
    }

    #[test]
    fn test_extractor_priority() {
        let extractor = &*TEST_REDTUBE;
        assert_eq!(extractor.priority(), 0);
    }

    #[test]
    fn test_redtube_implements_search_extractor() {
        let extractor = &*TEST_REDTUBE;
        let filters =
            <RedTubeExtractor as rdlp_core::SearchExtractor>::supported_filters(extractor);
        assert!(!filters.is_empty());
        assert_eq!(
            <RedTubeExtractor as rdlp_core::SearchExtractor>::name(extractor),
            "RedTube"
        );
    }

    #[test]
    fn test_search_filters_have_ordering() {
        let extractor = &*TEST_REDTUBE;
        let filters =
            <RedTubeExtractor as rdlp_core::SearchExtractor>::supported_filters(extractor);
        let ordering = filters.iter().find(|f| f.key == "ordering");
        assert!(ordering.is_some());
        assert_eq!(ordering.unwrap().allowed_values.len(), 5);
    }

    #[test]
    fn test_search_filters_have_period() {
        let extractor = &*TEST_REDTUBE;
        let filters =
            <RedTubeExtractor as rdlp_core::SearchExtractor>::supported_filters(extractor);
        let period = filters.iter().find(|f| f.key == "period");
        assert!(period.is_some());
        assert_eq!(period.unwrap().allowed_values.len(), 3);
    }
}
