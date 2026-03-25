//! TNAFlix network extractor
//!
//! Supports sites in the TNAFlix network:
//! - TNAFlix: `https://www.tnaflix.com/category/title/video123456`
//! - EMPFlix: `https://www.empflix.com/videos/title-123`
//! - MovieFap: `https://www.moviefap.com/videos/abc123/title.html`
//!
//! ## Module Structure
//!
//! - `patterns` - URL regex patterns for each site
//! - `ajax` - AJAX/XML data fetching for EMPFlix and MovieFap
//! - `search` - HTML search result parsing for TNAFlix
//! - `search_patterns` - Search URL builders and filter descriptors

mod ajax;
mod moviefap_search;
mod moviefap_search_patterns;
mod patterns;
mod search;
mod search_patterns;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, SearchExtractor};
use rdlp_types::{InfoDict, SearchPageResponse};
use regex::Regex;
use scraper::Html;
use std::time::Duration;

use crate::base::common::{BaseExtractor, MAX_PLAYLIST_SIZE};
use crate::base::tnaflix_network::TnaFlixNetworkBase;
use patterns::{EMPFLIX_URL_PATTERN, MOVIEFAP_URL_PATTERN, TNAFLIX_URL_PATTERN};

/// Rate limit delay between search page fetches (500 ms)
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// TNAFlix network extractor (supports TNAFlix, EMPFlix, MovieFap)
///
/// Uses [`TnaFlixNetworkBase`] for shared extraction logic.
pub struct TNAFlixExtractor {
    name: &'static str,
    url_pattern: &'static Regex,
    base: TnaFlixNetworkBase,
}

impl TNAFlixExtractor {
    /// Create extractor for TNAFlix
    #[must_use]
    pub fn tnaflix() -> Self {
        Self {
            name: "TNAFlix",
            url_pattern: &TNAFLIX_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Create extractor for EMPFlix
    #[must_use]
    pub fn empflix() -> Self {
        Self {
            name: "EMPFlix",
            url_pattern: &EMPFLIX_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Create extractor for MovieFap
    #[must_use]
    pub fn moviefap() -> Self {
        Self {
            name: "MovieFap",
            url_pattern: &MOVIEFAP_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Extract video ID from URL using BaseExtractor utility
    fn extract_id(&self, url: &str) -> Option<String> {
        // Try each capture group in order (different URL patterns)
        BaseExtractor::extract_id_positional(url, self.url_pattern, &[1, 2, 3])
    }
}

#[async_trait]
impl InfoExtractor for TNAFlixExtractor {
    fn name(&self) -> &str {
        self.name
    }

    fn valid_url(&self) -> &Regex {
        self.url_pattern
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Fetch the webpage using BaseExtractor (handles errors, verbose logging)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Get video ID using BaseExtractor
        let video_id = self.extract_id(url).ok_or_else(|| {
            RdlpError::Extraction(format!("Could not extract video ID from URL: {url}"))
        })?;

        // Check if this is MovieFap (uses different video loading mechanism)
        let is_moviefap = url.contains("moviefap.com");

        // Extract all data from HTML before any async operations
        let (metadata, cdn_url_opt) = {
            let html = Html::parse_document(&webpage);

            // Extract metadata using base (includes title, description, uploader, thumbnail, and enhanced JSON-LD fields)
            let metadata = self.base.extract_metadata(&html)?;

            // For MovieFap, extract cdn.php URL using base
            let cdn_url_opt = if is_moviefap {
                self.base.extract_cdn_url(&webpage)
            } else {
                None
            };

            (metadata, cdn_url_opt)
        }; // html is dropped here

        // Parse video data based on site type
        let video_data = if is_moviefap {
            // MovieFap: fetch XML from cdn.php
            let cdn_url = cdn_url_opt.ok_or_else(|| {
                RdlpError::Extraction(format!(
                    "Could not find cdn.php URL in MovieFap page: {url}"
                ))
            })?;

            BaseExtractor::log_if_verbose(ctx, "MovieFap", &format!("cdn.php URL: {cdn_url}"));

            ajax::parse_moviefap_xml(&self.base, &cdn_url, ctx).await?
        } else {
            // TNAFlix/EMPFlix: try HTML <source> tags first, fallback to AJAX
            let video_data = {
                let html = Html::parse_document(&webpage);
                self.base.parse_video_sources(&html)
            }; // html is dropped here

            // EMPFlix fallback: if no sources found, try AJAX endpoint
            let video_data = if video_data.is_empty() && url.contains("empflix.com") {
                BaseExtractor::log_if_verbose(
                    ctx,
                    "EMPFlix",
                    "No sources in HTML, trying AJAX endpoint...",
                );
                ajax::parse_empflix_ajax(&self.base, &video_id, url, ctx).await?
            } else {
                video_data
            };

            // Return error if still no sources found
            if video_data.is_empty() {
                return Err(RdlpError::Extraction(format!(
                    "No video source tags found in HTML. Video may be unavailable. URL: {url}"
                )));
            }

            video_data
        };

        // Build formats and fetch filesizes using base (asynchronous)
        let formats = self.base.build_formats(video_data, ctx).await;

        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Build InfoDict with all extracted metadata
        let mut info = InfoDict::new(video_id, metadata.title, self.name, url);
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
        info.formats = formats;

        Ok(info)
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// TNAFlix search extractor
///
/// Provides keyword search across TNAFlix with optional ordering filters.
/// Supports both single-page (`search_page`) and collect-all (`search`) modes.
pub struct TNAFlixSearchExtractor;

impl TNAFlixSearchExtractor {
    /// Create a new TNAFlix search extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Build the URL for a given page, dispatching to browse or search URL builders.
    ///
    /// When a `category` filter is present, builds a browse URL (ignoring the query text).
    /// Otherwise, builds a search URL from the query.
    ///
    /// # Arguments
    /// * `query` - The search query with optional filters.
    /// * `page` - 1-based page number.
    fn build_page_url(query: &rdlp_types::SearchQuery, page: usize) -> String {
        if let Some(cat) = query.filters.iter().find(|f| f.key == "category") {
            if page <= 1 {
                search_patterns::build_browse_url(&cat.value)
            } else {
                search_patterns::build_browse_url_page(&cat.value, page)
            }
        } else {
            search_patterns::build_search_url_page(query, page)
        }
    }

    /// Fetch a single search results page and return `(results, max_page_number)`.
    async fn fetch_single_search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, usize)> {
        let page_url = Self::build_page_url(query, page);
        debug!(page; "[TNAFlix] Fetching search page: {}", rdlp_security::sanitize_for_logging(&page_url));

        let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let page_results = search::parse_search_results(&webpage);
        let max_pages = search::parse_pagination(&webpage).unwrap_or(1);

        debug!(
            count = page_results.len(),
            max_pages;
            "[TNAFlix] Search page {page} returned {} results",
            page_results.len()
        );

        Ok((page_results, max_pages))
    }

    /// Collect all pages up to `MAX_PLAYLIST_SIZE` results.
    async fn search_all_pages(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        search::validate_search_filters(&query.filters)?;

        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);

        let mut all_results = Vec::new();
        let mut page = 1usize;

        loop {
            let (page_results, max_pages) = match self
                .fetch_single_search_page(query, page, ctx)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    debug!(page; "[TNAFlix] Failed to fetch search page, returning partial results: {e}");
                    break;
                }
            };

            if page_results.is_empty() {
                debug!(page; "[TNAFlix] No results on page, stopping pagination");
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if page >= max_pages {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(count = all_results.len(), pages = page; "[TNAFlix] Search complete");

        Ok(all_results)
    }
}

impl Default for TNAFlixSearchExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchExtractor for TNAFlixSearchExtractor {
    fn name(&self) -> &str {
        "TNAFlix"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        search_patterns::search_filter_descriptors()
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
        search::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1) as usize;
        let (page_results, max_pages) = self.fetch_single_search_page(query, page, ctx).await?;

        let has_more = page < max_pages && !page_results.is_empty();

        Ok(SearchPageResponse {
            results: page_results,
            page: page as u32,
            has_more,
            total_estimate: None,
        })
    }
}

/// EMPFlix search extractor
///
/// EMPFlix shares the same HTML structure as TNAFlix.  This extractor reuses
/// the same HTML parser (`search::parse_search_results` /
/// `search::parse_pagination`) but targets `empflix.com` URLs.
pub struct EMPFlixSearchExtractor;

impl EMPFlixSearchExtractor {
    /// Create a new EMPFlix search extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// EMPFlix base URL.
    const BASE_URL: &'static str = "https://www.empflix.com";

    /// Build the URL for a given page, dispatching to browse or search URL builders.
    fn build_page_url(query: &rdlp_types::SearchQuery, page: usize) -> String {
        if let Some(cat) = query.filters.iter().find(|f| f.key == "category") {
            if page <= 1 {
                search_patterns::build_browse_url_for(Self::BASE_URL, &cat.value)
            } else {
                search_patterns::build_browse_url_page_for(Self::BASE_URL, &cat.value, page)
            }
        } else {
            search_patterns::build_search_url_page_for(Self::BASE_URL, query, page)
        }
    }

    /// Fetch a single search results page and return `(results, max_page_number)`.
    async fn fetch_single_search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, usize)> {
        let page_url = Self::build_page_url(query, page);
        debug!(page; "[EMPFlix] Fetching search page: {}", rdlp_security::sanitize_for_logging(&page_url));

        let webpage = crate::base::common::BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let page_results = search::parse_search_results(&webpage);
        let max_pages = search::parse_pagination(&webpage).unwrap_or(1);

        debug!(
            count = page_results.len(),
            max_pages;
            "[EMPFlix] Search page {page} returned {} results",
            page_results.len()
        );

        Ok((page_results, max_pages))
    }

    /// Collect all pages up to `MAX_PLAYLIST_SIZE` results.
    async fn search_all_pages(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        search::validate_search_filters(&query.filters)?;

        let max_results = query
            .max_results
            .unwrap_or(crate::base::common::MAX_PLAYLIST_SIZE);

        let mut all_results = Vec::new();
        let mut page = 1usize;

        loop {
            let (page_results, max_pages) = match self
                .fetch_single_search_page(query, page, ctx)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    debug!(page; "[EMPFlix] Failed to fetch search page, returning partial results: {e}");
                    break;
                }
            };

            if page_results.is_empty() {
                debug!(page; "[EMPFlix] No results on page, stopping pagination");
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if page >= max_pages {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(count = all_results.len(), pages = page; "[EMPFlix] Search complete");

        Ok(all_results)
    }
}

impl Default for EMPFlixSearchExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl rdlp_core::SearchExtractor for EMPFlixSearchExtractor {
    fn name(&self) -> &str {
        "EMPFlix"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        // EMPFlix uses the same filter set as TNAFlix
        search_patterns::search_filter_descriptors()
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
    ) -> Result<rdlp_types::SearchPageResponse> {
        search::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1) as usize;
        let (page_results, max_pages) = self.fetch_single_search_page(query, page, ctx).await?;

        let has_more = page < max_pages && !page_results.is_empty();

        Ok(rdlp_types::SearchPageResponse {
            results: page_results,
            page: page as u32,
            has_more,
            total_estimate: None,
        })
    }
}

/// MovieFap search extractor
///
/// Parses MovieFap's distinct HTML search result structure.
/// Uses `moviefap_search` and `moviefap_search_patterns` internally.
pub struct MovieFapSearchExtractor;

impl MovieFapSearchExtractor {
    /// Create a new MovieFap search extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Fetch a single search results page and return `(results, max_page_number)`.
    async fn fetch_single_search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, usize)> {
        let page_url = moviefap_search_patterns::build_search_url(query, page);
        debug!(page; "[MovieFap] Fetching search page: {}", rdlp_security::sanitize_for_logging(&page_url));

        let webpage = crate::base::common::BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let page_results = moviefap_search::parse_search_results(&webpage);
        let max_pages = moviefap_search::parse_pagination(&webpage).unwrap_or(1);

        debug!(
            count = page_results.len(),
            max_pages;
            "[MovieFap] Search page {page} returned {} results",
            page_results.len()
        );

        Ok((page_results, max_pages))
    }

    /// Collect all pages up to `MAX_PLAYLIST_SIZE` results.
    async fn search_all_pages(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        moviefap_search::validate_search_filters(&query.filters)?;

        let max_results = query
            .max_results
            .unwrap_or(crate::base::common::MAX_PLAYLIST_SIZE);

        let mut all_results = Vec::new();
        let mut page = 1usize;

        loop {
            let (page_results, max_pages) = match self
                .fetch_single_search_page(query, page, ctx)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    debug!(page; "[MovieFap] Failed to fetch search page, returning partial results: {e}");
                    break;
                }
            };

            if page_results.is_empty() {
                debug!(page; "[MovieFap] No results on page, stopping pagination");
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if page >= max_pages {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(count = all_results.len(), pages = page; "[MovieFap] Search complete");

        Ok(all_results)
    }
}

impl Default for MovieFapSearchExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl rdlp_core::SearchExtractor for MovieFapSearchExtractor {
    fn name(&self) -> &str {
        "MovieFap"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        moviefap_search_patterns::search_filter_descriptors()
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
    ) -> Result<rdlp_types::SearchPageResponse> {
        moviefap_search::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1) as usize;
        let (page_results, max_pages) = self.fetch_single_search_page(query, page, ctx).await?;

        let has_more = page < max_pages && !page_results.is_empty();

        Ok(rdlp_types::SearchPageResponse {
            results: page_results,
            page: page as u32,
            has_more,
            total_estimate: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    /// Shared test fixtures (compiled once, reused across all tests)
    ///
    /// Performance: Prevents unnecessary regex compilation in tests:
    /// - Without lazy: ~50μs × 5 test instances = 250μs wasted
    /// - With lazy: ~0.01μs access after first initialization
    static TEST_TNAFLIX: LazyLock<TNAFlixExtractor> = LazyLock::new(TNAFlixExtractor::tnaflix);
    static TEST_EMPFLIX: LazyLock<TNAFlixExtractor> = LazyLock::new(TNAFlixExtractor::empflix);
    static TEST_MOVIEFAP: LazyLock<TNAFlixExtractor> = LazyLock::new(TNAFlixExtractor::moviefap);

    #[test]
    fn test_tnaflix_url_suitable() {
        let extractor = &*TEST_TNAFLIX;
        assert!(extractor.suitable("https://www.tnaflix.com/hd-videos/test/video123456"));
        assert!(extractor.suitable("https://tnaflix.com/amateur-porn/title/video999"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_empflix_url_suitable() {
        let extractor = &*TEST_EMPFLIX;
        assert!(extractor.suitable("https://www.empflix.com/videos/title-123"));
        assert!(extractor.suitable("https://empflix.com/view/123"));
        assert!(extractor.suitable(
            "https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093"
        ));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_empflix_extract_id() {
        let extractor = &*TEST_EMPFLIX;

        // Test /videos/title-ID format
        let id1 = extractor.extract_id("https://www.empflix.com/videos/title-123");
        assert_eq!(id1, Some("123".to_string()));

        // Test /category/ID format
        let id2 = extractor.extract_id("https://empflix.com/view/456");
        assert_eq!(id2, Some("456".to_string()));

        // Test /category/title/videoID format
        let id3 = extractor.extract_id(
            "https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093",
        );
        assert_eq!(id3, Some("3715093".to_string()));
    }

    #[test]
    fn test_moviefap_url_suitable() {
        let extractor = &*TEST_MOVIEFAP;
        assert!(extractor.suitable("https://www.moviefap.com/videos/abc123def/title.html"));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_tnaflix_extract_id() {
        let extractor = &*TEST_TNAFLIX;
        let id = extractor.extract_id("https://www.tnaflix.com/hd-videos/test/video123456");
        assert_eq!(id, Some("123456".to_string()));
    }

    #[test]
    fn test_extractor_names() {
        assert_eq!(TEST_TNAFLIX.name(), "TNAFlix");
        assert_eq!(TEST_EMPFLIX.name(), "EMPFlix");
        assert_eq!(TEST_MOVIEFAP.name(), "MovieFap");
    }

    #[test]
    fn test_extractor_priority() {
        assert_eq!(TEST_TNAFLIX.priority(), 0);
        assert_eq!(TEST_EMPFLIX.priority(), 0);
        assert_eq!(TEST_MOVIEFAP.priority(), 0);
    }

    #[test]
    fn test_search_extractor_name() {
        let extractor = TNAFlixSearchExtractor::new();
        assert_eq!(SearchExtractor::name(&extractor), "TNAFlix");
    }

    #[test]
    fn test_search_extractor_supported_filters() {
        let extractor = TNAFlixSearchExtractor::new();
        let filters = extractor.supported_filters();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].key, "ordering");
    }

    // --- EMPFlix search extractor tests ---

    #[test]
    fn test_empflix_search_extractor_name() {
        let extractor = EMPFlixSearchExtractor::new();
        assert_eq!(rdlp_core::SearchExtractor::name(&extractor), "EMPFlix");
    }

    #[test]
    fn test_empflix_search_extractor_supported_filters() {
        let extractor = EMPFlixSearchExtractor::new();
        let filters = extractor.supported_filters();
        // EMPFlix shares the TNAFlix filter set (ordering + category)
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[1].key, "category");
    }

    #[test]
    fn test_empflix_search_url_uses_empflix_domain() {
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = EMPFlixSearchExtractor::build_page_url(&query, 1);
        assert!(url.contains("empflix.com"), "URL must use empflix.com");
        assert!(!url.contains("tnaflix.com"), "URL must NOT use tnaflix.com");
    }

    #[test]
    fn test_empflix_search_url_page_2() {
        let query = rdlp_types::SearchQuery {
            query: "test query".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = EMPFlixSearchExtractor::build_page_url(&query, 2);
        assert!(url.contains("empflix.com"));
        assert!(url.contains("page=2"));
    }

    #[test]
    fn test_empflix_search_url_with_filter() {
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![rdlp_types::SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = EMPFlixSearchExtractor::build_page_url(&query, 1);
        assert!(url.contains("empflix.com"));
        assert!(url.contains("ordering=newest"));
    }

    #[test]
    fn test_empflix_browse_url_uses_empflix_domain() {
        let query = rdlp_types::SearchQuery {
            query: String::new(),
            filters: vec![rdlp_types::SearchFilter {
                key: "category".to_string(),
                value: "teen-porn".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = EMPFlixSearchExtractor::build_page_url(&query, 1);
        assert!(
            url.contains("empflix.com"),
            "Browse URL must use empflix.com"
        );
        assert!(
            url.contains("teen-porn"),
            "Browse URL must contain category slug"
        );
        assert!(
            !url.contains("tnaflix.com"),
            "Browse URL must NOT use tnaflix.com"
        );
    }

    // --- MovieFap search extractor tests ---

    #[test]
    fn test_moviefap_search_extractor_name() {
        let extractor = MovieFapSearchExtractor::new();
        assert_eq!(rdlp_core::SearchExtractor::name(&extractor), "MovieFap");
    }

    #[test]
    fn test_moviefap_search_extractor_supported_filters() {
        let extractor = MovieFapSearchExtractor::new();
        let filters = extractor.supported_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[0].allowed_values.len(), 5);
    }

    #[test]
    fn test_moviefap_search_extractor_default_ordering() {
        let extractor = MovieFapSearchExtractor::new();
        let filters = extractor.supported_filters();
        assert_eq!(filters[0].default, Some("relevance".to_string()));
    }
}
