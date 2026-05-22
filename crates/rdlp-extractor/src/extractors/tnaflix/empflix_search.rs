//! EMPFlix search extractor.

use super::*;

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

    /// Fetch a single search results page and return `(results, max_page_number)`.
    async fn fetch_search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<rdlp_types::SearchResultPreview>, usize)> {
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

        Ok((page_results, max_pages))
    }

    /// Collect all pages up to `MAX_PLAYLIST_SIZE` results.
    async fn search_all_pages(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        tnaflix_search_helpers::validate_search_filters(&query.filters)?;

        let max_results = query
            .max_results
            .unwrap_or(crate::base::common::MAX_PLAYLIST_SIZE);

        let mut all_results = Vec::new();
        let mut page = 1usize;

        loop {
            let (page_results, max_pages) = match self.fetch_search_page(query, page, ctx).await {
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
        tnaflix_search_helpers::validate_search_filters(&query.filters)?;

        let page = query.page.unwrap_or(1) as usize;
        let (page_results, max_pages) = self.fetch_search_page(query, page, ctx).await?;

        let has_more = page < max_pages && !page_results.is_empty();

        Ok(rdlp_types::SearchPageResponse {
            results: page_results,
            page: page as u32,
            has_more,
            total_estimate: None,
        })
    }
}
