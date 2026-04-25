//! Video information extraction

use super::{Orchestrator, errors::*};
use log::{debug, info};
use rdlp_types::{SearchFilterDescriptor, SearchPageResponse, SearchQuery, SearchResultPreview};
use tracing::instrument;

impl Orchestrator {
    /// Extract video information from URL
    ///
    /// Finds the appropriate extractor for the given URL and extracts metadata
    /// including title, formats, and other video information.
    ///
    /// # Errors
    /// Returns an error if:
    /// - No extractor is found for the URL
    /// - Extraction fails
    #[instrument(skip(self), fields(url = %url))]
    pub(super) async fn extract_video_info(&self, url: &str) -> Result<rdlp_types::InfoDict> {
        debug!("Finding extractor for URL...");

        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: url.to_owned(),
            }
        })?;

        debug!("Using extractor: {}", extractor.name());
        debug!("Extracting video information...");

        let mut info = extractor
            .extract(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        // Log extracted metadata
        debug!("Title: {}", info.title);

        if let Some(ref uploader) = info.uploader {
            debug!("Uploader: {uploader}");
        }
        if let Some(ref channel) = info.channel {
            debug!("Channel: {channel}");
        }
        if let Some(duration) = info.duration {
            let mins = (duration / 60.0) as u32;
            let secs = (duration % 60.0) as u32;
            debug!("Duration: {mins}:{secs:02}");
        }
        if let Some(views) = info.view_count {
            debug!("Views: {views}");
        }
        if let Some(rating) = info.average_rating {
            debug!("Rating: {rating:.0}%");
        }
        if let Some(ref tags) = info.tags
            && !tags.is_empty()
        {
            debug!("Tags: {}", tags.len());
        }

        debug!("Found {} formats", info.formats.len());

        // Auto-set Referer header on all formats that don't already have one.
        // Many CDNs (PornHub, XHamster, etc.) require a Referer to serve content.
        if !info.webpage_url.is_empty() {
            let referer = info.webpage_url.clone();
            for fmt in &mut info.formats {
                let headers = fmt
                    .http_headers
                    .get_or_insert_with(std::collections::HashMap::new);
                headers
                    .entry("Referer".to_string())
                    .or_insert_with(|| referer.clone());
            }
        }

        Ok(info)
    }

    /// Lightweight format extraction for lazily-resolved playlist entries.
    ///
    /// Uses `extract_lazy()` instead of `extract()` to skip expensive
    /// operations like re-fetching the watch page. Auto-sets `Referer`
    /// headers on all resolved formats.
    pub(super) async fn extract_lazy_formats(&self, url: &str) -> Result<rdlp_types::InfoDict> {
        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: url.to_owned(),
            }
        })?;

        let mut info = extractor
            .extract_lazy(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        debug!(formats = info.formats.len(); "Lazily resolved formats");

        // Auto-set Referer header on all formats
        if !info.webpage_url.is_empty() {
            let referer = info.webpage_url.clone();
            for fmt in &mut info.formats {
                let headers = fmt
                    .http_headers
                    .get_or_insert_with(std::collections::HashMap::new);
                headers
                    .entry("Referer".to_string())
                    .or_insert_with(|| referer.clone());
            }
        }

        Ok(info)
    }

    /// Extract playlist information from URL
    ///
    /// Finds the appropriate extractor and attempts playlist extraction.
    /// Returns a vector of InfoDict - one for each video in the playlist.
    ///
    /// # Returns
    /// - Single video: Vec with one InfoDict
    /// - Playlist: Vec with multiple InfoDict entries
    ///
    /// # Errors
    /// Returns an error if:
    /// - No extractor is found for the URL
    /// - Extraction fails
    #[instrument(skip(self), fields(url = %url))]
    pub(super) async fn extract_playlist_info(
        &self,
        url: &str,
    ) -> Result<Vec<rdlp_types::InfoDict>> {
        debug!("Finding extractor for URL...");

        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: url.to_owned(),
            }
        })?;

        debug!("Using extractor: {}", extractor.name());
        debug!("Extracting playlist information...");

        let infos = extractor
            .extract_playlist(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        if infos.len() == 1 {
            debug!("Single video: {}", infos[0].title);
        } else {
            let playlist_title = infos[0]
                .playlist_title
                .as_deref()
                .unwrap_or("Unnamed Playlist");
            info!("Playlist: {playlist_title}");
            debug!("Found {} videos", infos.len());
        }

        Ok(infos)
    }

    /// Execute a search query using the named search extractor.
    ///
    /// # Arguments
    /// * `extractor_name` - Site name (e.g., "xhamster")
    /// * `query` - Search query with filters and optional max results
    ///
    /// # Errors
    /// Returns an error if the site name is unknown or the search fails.
    pub async fn search(
        &self,
        extractor_name: &str,
        query: &SearchQuery,
    ) -> Result<Vec<SearchResultPreview>> {
        let extractor = self
            .extractor_registry
            .find_search_extractor(extractor_name)
            .ok_or_else(|| {
                let available = self.extractor_registry.list_search_extractors();
                OrchestratorError::Configuration(format!(
                    "Unknown search site: '{}'. Available: {}",
                    extractor_name,
                    available.join(", ")
                ))
            })?;

        info!(site = extractor_name, query = query.query.as_str(); "Starting search");

        let results = tokio::select! {
            res = extractor.search(query, &self.extraction_context) => {
                res.map_err(OrchestratorError::ExtractionFailed)?
            }
            () = self.cancel_token.cancelled() => {
                debug!("Search cancelled by token");
                return Err(OrchestratorError::UserCancelled);
            }
        };

        info!(site = extractor_name, count = results.len(); "Search complete");

        Ok(results)
    }

    /// Execute a paginated search query, returning a single page of results.
    pub async fn search_page(
        &self,
        extractor_name: &str,
        query: &SearchQuery,
    ) -> Result<SearchPageResponse> {
        let extractor = self
            .extractor_registry
            .find_search_extractor(extractor_name)
            .ok_or_else(|| {
                let available = self.extractor_registry.list_search_extractors();
                OrchestratorError::Configuration(format!(
                    "Unknown search site: '{}'. Available: {}",
                    extractor_name,
                    available.join(", ")
                ))
            })?;

        info!(site = extractor_name, query = query.query.as_str(); "Starting paginated search");

        let response = tokio::select! {
            res = extractor.search_page(query, &self.extraction_context) => {
                res.map_err(OrchestratorError::ExtractionFailed)?
            }
            () = self.cancel_token.cancelled() => {
                debug!("Paginated search cancelled by token");
                return Err(OrchestratorError::UserCancelled);
            }
        };

        info!(site = extractor_name, count = response.results.len(), page = response.page; "Search page complete");

        Ok(response)
    }

    /// List names of all search-capable extractors.
    pub fn list_search_extractors(&self) -> Vec<&str> {
        self.extractor_registry.list_search_extractors()
    }

    /// Lazily enrich a single previously-returned `SearchResultPreview`.
    ///
    /// Frontends call this on demand (e.g. when a row scrolls into view)
    /// to fill metadata gaps the cheap search path cannot — at most one
    /// HTTP request to the underlying video page per call. Sites whose
    /// search-card markup is already complete return the input unchanged.
    pub async fn enrich_search_result(
        &self,
        extractor_name: &str,
        preview: SearchResultPreview,
    ) -> Result<SearchResultPreview> {
        let extractor = self
            .extractor_registry
            .find_search_extractor(extractor_name)
            .ok_or_else(|| {
                let available = self.extractor_registry.list_search_extractors();
                OrchestratorError::Configuration(format!(
                    "Unknown search site: '{}'. Available: {}",
                    extractor_name,
                    available.join(", ")
                ))
            })?;

        tokio::select! {
            res = extractor.enrich(preview, &self.extraction_context) => {
                res.map_err(OrchestratorError::ExtractionFailed)
            }
            () = self.cancel_token.cancelled() => {
                debug!("Search-result enrichment cancelled by token");
                Err(OrchestratorError::UserCancelled)
            }
        }
    }

    /// Get filter descriptors for a search extractor.
    ///
    /// # Errors
    /// Returns an error if the site name is unknown.
    pub fn search_filters(&self, extractor_name: &str) -> Result<Vec<SearchFilterDescriptor>> {
        let extractor = self
            .extractor_registry
            .find_search_extractor(extractor_name)
            .ok_or_else(|| {
                OrchestratorError::Configuration(format!("Unknown search site: '{extractor_name}'"))
            })?;
        Ok(extractor.supported_filters())
    }
}
