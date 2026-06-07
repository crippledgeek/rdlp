//! RedTube extractor
//!
//! Supports URLs like:
//! - https://www.redtube.com/123456
//! - https://www.redtube.com.br/123456
//! - https://embed.redtube.com/?id=123456
//!
//! Uses the RedTube public API (`api.redtube.com`) as the primary source for
//! video metadata, with HTML scraping as a fallback. Video format URLs are
//! always extracted from the webpage since the API does not expose them.
//!
//! ## Module Structure
//!
//! - `patterns` - URL and extraction regex patterns, API URL builders
//! - `formats` - Format extraction from JavaScript sources, mediaDefinition, and API
//! - `search` - Search result parsing and filter validation

mod filters;
mod formats;
mod patterns;
mod search;

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{
    ExponentialBuilder, ExtractionContext, InfoExtractor, RdlpError, Result, Retryable,
    SearchExtractor,
};
use rdlp_types::{InfoDict, SearchPageResponse};
use regex::Regex;
use scraper::Html;
use std::sync::LazyLock;
use std::time::Duration;

use crate::base::common::{BaseExtractor, MAX_PLAYLIST_SIZE};
use crate::base::tnaflix_network::TnaFlixNetworkBase;
use crate::hls::detect_format_sizes_lazy;
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

    /// Fetch video metadata from the RedTube API (`getVideoById`).
    ///
    /// Returns parsed metadata on success, or an error if the API is
    /// unreachable or returns an unexpected response.
    async fn fetch_api_video_info(
        &self,
        video_id: &str,
        ctx: &ExtractionContext,
    ) -> Result<formats::ApiVideoMetadata> {
        let api_url = patterns::build_api_video_url(video_id);

        debug!(
            "[RedTube] Fetching API video info: {}",
            rdlp_redact::RedactedUrl::new(&api_url)
        );

        let response =
            ctx.http_client
                .get(&api_url)
                .send()
                .await
                .map_err(|e| RdlpError::Network {
                    message: format!("Failed to fetch RedTube video API: {e}"),
                    url: Some(api_url.clone().into()),
                })?;

        rdlp_core::check_http_response(&response)?;

        let body = response.text().await.map_err(|e| RdlpError::Network {
            message: format!("Failed to read RedTube video API response: {e}"),
            url: Some(api_url.into()),
        })?;

        BaseExtractor::log_content_if_verbose(ctx, "RedTube", "API video response", &body, 500);

        formats::parse_api_video_response(&body)
    }

    /// Build `InfoDict` using API metadata as the primary source.
    ///
    /// Falls back to HTML scraping for fields the API does not provide
    /// (description, uploader, channel, like_count, average_rating, categories).
    fn build_info_from_api(
        &self,
        video_id: &str,
        url: &str,
        api_meta: formats::ApiVideoMetadata,
        webpage: &str,
        formats: Vec<rdlp_types::Format>,
        hls_flags: &crate::hls::HlsStreamFlags,
    ) -> InfoDict {
        let mut info = InfoDict::new(video_id, &api_meta.title, InfoExtractor::name(self), url);
        info.thumbnail = api_meta.thumbnail;
        info.thumbnails = api_meta.thumbnails;
        info.duration = api_meta.duration;
        info.upload_date = api_meta.upload_date;
        info.view_count = api_meta.view_count;
        info.tags = api_meta.tags;
        info.age_limit = Some(18);
        info.formats = formats;

        // Supplement with HTML-scraped fields the API does not return
        {
            let html = Html::parse_document(webpage);
            if let Ok(html_meta) = self.base.extract_metadata(&html) {
                info.description = html_meta.description;
                info.uploader = html_meta.uploader;
                info.uploader_id = html_meta.uploader_id;
                info.uploader_url = html_meta.uploader_url;
                info.channel = html_meta.channel;
                info.channel_id = html_meta.channel_id;
                info.channel_url = html_meta.channel_url;
                info.like_count = html_meta.like_count;
                info.average_rating = html_meta.average_rating;
                info.categories = html_meta.categories;
            }
            info.actors = extract_performers(&html);
        }

        info.propagate_duration();

        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        info
    }

    /// Build `InfoDict` entirely from HTML-scraped metadata (fallback path).
    fn build_info_from_html(
        &self,
        video_id: &str,
        url: &str,
        webpage: &str,
        formats: Vec<rdlp_types::Format>,
        hls_flags: &crate::hls::HlsStreamFlags,
    ) -> Result<InfoDict> {
        let (metadata, actors) = {
            let html = Html::parse_document(webpage);
            (
                self.base.extract_metadata(&html)?,
                extract_performers(&html),
            )
        };

        let mut info = InfoDict::new(video_id, metadata.title, InfoExtractor::name(self), url);
        info.actors = actors;
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
        info.age_limit = Some(18);
        info.formats = formats;
        info.propagate_duration();

        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        Ok(info)
    }
}

/// Extract performer/pornstar names from RedTube video page HTML.
///
/// Looks for `<a href="/pornstar/name">` links within the `.performers-list` section.
fn extract_performers(html: &Html) -> Vec<String> {
    static PERFORMER_SELECTOR: LazyLock<scraper::Selector> =
        crate::static_selector!(".performers-list a[href*=\"/pornstar/\"]");

    html.select(&PERFORMER_SELECTOR)
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
        // Get video ID using BaseExtractor
        let video_id = self.extract_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!(
                "Could not extract video ID from URL: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        })?;

        // Try API first for metadata
        let api_metadata = self.fetch_api_video_info(&video_id, ctx).await;

        // Always fetch webpage for format extraction (API does not return video URLs)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

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
            return Err(RdlpError::Extraction {
                message: format!(
                    "No video sources found in JavaScript or HTML. \
                     Video may be unavailable. URL: {}",
                    rdlp_redact::RedactedUrl::new(url)
                ),
                url: Some(url.to_string().into()),
            });
        }

        // Convert relative URLs to absolute using utility
        for format in &mut formats {
            if !format.url.starts_with("http://") && !format.url.starts_with("https://") {
                format.url = make_absolute_url(url, &format.url);
            }
        }

        // Pre-resolve HLS variant playlists into per-variant Format rows with
        // fragments populated, so the downloader's pre-resolved-fragments path
        // is taken. MUST run before `detect_format_sizes_lazy` (issue #269/#279).
        let formats = crate::hls::expand_hls_in_place(formats, ctx.http_client.clone()).await;

        // Fetch sizes/segments for all formats in parallel
        let (formats, hls_flags) =
            detect_format_sizes_lazy(formats, ctx, InfoExtractor::name(self)).await;

        // Build InfoDict — prefer API metadata, fall back to HTML scrape
        let info = if let Ok(api_meta) = api_metadata {
            debug!("[RedTube] Using API metadata for video {video_id}");
            self.build_info_from_api(&video_id, url, api_meta, &webpage, formats, &hls_flags)
        } else {
            debug!("[RedTube] API unavailable, using HTML metadata for video {video_id}");
            self.build_info_from_html(&video_id, url, &webpage, formats, &hls_flags)?
        };

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
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
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
                        debug!("[RedTube] API search failed, falling back to HTML: {e}");
                        let html_url = patterns::build_html_search_url(&query.query);
                        match self.fetch_html_search_page(&html_url, ctx).await {
                            Ok(results) => results,
                            Err(html_err) => {
                                warn!("[RedTube] HTML fallback also failed: {html_err}");
                                break;
                            }
                        }
                    } else {
                        debug!(
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

        debug!(
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
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, Option<u64>)> {
        let response = (|| async { ctx.http_client.get(url).send().await })
            .retry(
                ExponentialBuilder::default()
                    .with_max_times(2)
                    .with_min_delay(Duration::from_millis(500)),
            )
            .when(|e| e.is_timeout() || e.is_connect())
            .await
            .map_err(|e| RdlpError::Network {
                message: format!("Failed to fetch search API: {e}"),
                url: Some(url.to_string().into()),
            })?;

        rdlp_core::check_http_response(&response)?;

        let body = response.text().await.map_err(|e| RdlpError::Network {
            message: format!("Failed to read search API response: {e}"),
            url: Some(url.to_string().into()),
        })?;

        search::parse_api_search_results(&body)
    }

    /// Fetch and parse a single HTML search page (fallback).
    async fn fetch_html_search_page(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
        search::parse_html_search_results(&webpage)
    }
}

#[async_trait]
impl SearchExtractor for RedTubeExtractor {
    fn name(&self) -> &str {
        "RedTube"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        patterns::search_filter_descriptors()
    }

    async fn search(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        self.search_all_pages(query, ctx).await
    }

    async fn search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let descriptors = patterns::search_filter_descriptors();
        search::validate_search_filters(&query.filters, &descriptors)?;

        let page = query.page.unwrap_or(1);
        let base_url = patterns::build_api_search_url(&query.query, &query.filters);

        let page_url = if page == 1 {
            base_url
        } else {
            patterns::build_api_search_url_page(&base_url, page)
        };

        let (page_results, total_count) = match self.fetch_api_search_page(&page_url, ctx).await {
            Ok(result) => result,
            Err(e) => {
                if page == 1 {
                    debug!("[RedTube] API search failed, falling back to HTML: {e}");
                    let html_url = patterns::build_html_search_url(&query.query);
                    let results = self.fetch_html_search_page(&html_url, ctx).await?;
                    (results, None)
                } else {
                    return Err(e);
                }
            }
        };

        let has_more = if let Some(total) = total_count {
            let fetched_through = u64::from(page) * u64::from(patterns::API_RESULTS_PER_PAGE);
            fetched_through < total && !page_results.is_empty()
        } else {
            false
        };

        Ok(SearchPageResponse {
            results: page_results,
            page,
            has_more,
            total_estimate: total_count,
        })
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
