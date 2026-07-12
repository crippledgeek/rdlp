//! MovieFap search extractor.
//!
//! Parses MovieFap's distinct HTML search result structure.
//! Uses `moviefap_search_helpers` and `moviefap_search_patterns` internally.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result};

use super::{moviefap_search_helpers, moviefap_search_patterns};
use crate::base::common::{PaginatedSearch, Termination};

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
}

impl PaginatedSearch for MovieFapSearchExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[MovieFap]"
    }

    fn validate_search_filters(&self, filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        moviefap_search_helpers::validate_search_filters(filters)
    }

    /// Fetch a single search results page and return `(results, Termination)`.
    async fn fetch_search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, Termination)> {
        let page_url = moviefap_search_patterns::build_search_url(query, page);
        debug!(page; "[MovieFap] Fetching search page: {}", rdlp_security::sanitize_for_logging(&page_url));

        let webpage = crate::base::common::BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let page_results = moviefap_search_helpers::parse_search_results(&webpage);
        let max_pages = moviefap_search_helpers::parse_pagination(&webpage).unwrap_or(1);

        debug!(
            count = page_results.len(),
            max_pages;
            "[MovieFap] Search page {page} returned {} results",
            page_results.len()
        );

        Ok((page_results, Termination::Pages(max_pages)))
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
        self.search_page_response(query, ctx).await
    }
}
