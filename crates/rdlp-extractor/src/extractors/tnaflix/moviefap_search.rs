//! MovieFap search extractor.
//!
//! Parses MovieFap's distinct HTML search result structure.
//! Uses `moviefap_search_helpers` and `moviefap_search_patterns` internally.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result};

use super::{moviefap_search_helpers, moviefap_search_patterns};
use crate::base::common::{PagedSearch, SearchPage, Termination};

/// The one spelling of this site's display name (#756); `name()`,
/// `InfoDict::new`, log tags and filter errors all read it. Also read by
/// `TNAFlixExtractor::moviefap()` in `info.rs` (the InfoExtractor side of
/// this same site).
pub(super) const NAME: rdlp_types::ExtractorName = rdlp_types::ExtractorName::MovieFap;

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

impl PagedSearch for MovieFapSearchExtractor {
    fn validate_search_filters(&self, filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        moviefap_search_helpers::validate_search_filters(filters)
    }

    /// Fetch + parse ONE search page. `has_more` is computed here from the
    /// site's reported page count (the `Termination` helper), so the shared
    /// loop stays conditional-free.
    async fn fetch_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let page_url = moviefap_search_patterns::build_search_url(query, page as usize);
        debug!(page; "[{NAME}] Fetching search page: {}", rdlp_security::sanitize_for_logging(&page_url));

        let webpage = crate::base::common::BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let page_results = moviefap_search_helpers::parse_search_results(&webpage);
        let max_pages = moviefap_search_helpers::parse_pagination(&webpage).unwrap_or(1);

        debug!(
            count = page_results.len(),
            max_pages;
            "[{NAME}] Search page {page} returned {} results",
            page_results.len()
        );

        let has_more =
            !page_results.is_empty() && Termination::Pages(max_pages).has_more(page as usize);
        Ok(SearchPage {
            results: page_results,
            has_more,
            total_estimate: None,
        })
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
        NAME.as_str()
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
