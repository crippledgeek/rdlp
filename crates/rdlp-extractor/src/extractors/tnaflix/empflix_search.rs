//! EMPFlix search extractor.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result};

use super::{search_patterns, tnaflix_search_helpers};
use crate::base::common::{PaginatedSearch, Termination};

/// EMPFlix search extractor
///
/// EMPFlix shares the same HTML structure as TNAFlix.  This extractor reuses
/// the same HTML parser (`tnaflix_search_helpers::parse_search_results` /
/// `tnaflix_search_helpers::parse_pagination`) but targets `empflix.com` URLs.
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
    pub(super) fn build_page_url(query: &rdlp_types::SearchQuery, page: usize) -> String {
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
}

#[async_trait]
impl PaginatedSearch for EMPFlixSearchExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[EMPFlix]"
    }

    fn validate_search_filters(&self, filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        tnaflix_search_helpers::validate_search_filters(filters)
    }

    /// Fetch a single search results page and return `(results, Termination)`.
    async fn fetch_search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, Termination)> {
        let page_url = Self::build_page_url(query, page);
        debug!(page; "[EMPFlix] Fetching search page: {}", rdlp_security::sanitize_for_logging(&page_url));

        let webpage = crate::base::common::BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let page_results = tnaflix_search_helpers::parse_search_results(&webpage);
        let max_pages = tnaflix_search_helpers::parse_pagination(&webpage).unwrap_or(1);

        debug!(
            count = page_results.len(),
            max_pages;
            "[EMPFlix] Search page {page} returned {} results",
            page_results.len()
        );

        Ok((page_results, Termination::Pages(max_pages)))
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
        tnaflix_search_helpers::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1) as usize;
        let (page_results, termination) = self.fetch_search_page(query, page, ctx).await?;

        let has_more = !page_results.is_empty() && termination.has_more(page);

        Ok(rdlp_types::SearchPageResponse {
            results: page_results,
            page: page as u32,
            has_more,
            total_estimate: None,
        })
    }
}
