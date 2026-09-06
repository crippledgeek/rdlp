//! Video information extraction

use super::{
    Orchestrator,
    errors::{OrchestratorError, Result},
};
use log::{debug, info};
use rdlp_redact::RedactedUrlBuf;
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
    #[instrument(level = "debug", skip(self), fields(url = %rdlp_redact::RedactedUrl::new(url)))]
    pub(super) async fn extract_video(&self, url: &str) -> Result<rdlp_types::InfoDict> {
        debug!("Finding extractor for URL...");

        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: RedactedUrlBuf::from(url),
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
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // duration is a non-negative seconds value; values up to ~136 years fit u32
            let mins = (duration / 60.0) as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
                url: RedactedUrlBuf::from(url),
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
    /// Returns a vector of `InfoDict` - one for each video in the playlist.
    ///
    /// # Returns
    /// - Single video: Vec with one `InfoDict`
    /// - Playlist: Vec with multiple `InfoDict` entries
    ///
    /// # Errors
    /// Returns an error if:
    /// - No extractor is found for the URL
    /// - Extraction fails
    #[instrument(level = "debug", skip(self), fields(url = %rdlp_redact::RedactedUrl::new(url)))]
    pub(super) async fn extract_playlist(&self, url: &str) -> Result<Vec<rdlp_types::InfoDict>> {
        debug!("Finding extractor for URL...");

        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: RedactedUrlBuf::from(url),
            }
        })?;

        debug!("Using extractor: {}", extractor.name());
        debug!("Extracting playlist information...");

        let infos = extractor
            .extract_playlist(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        if infos.len() == 1 {
            if let Some(first) = infos.first() {
                debug!("Single video: {}", first.title);
            }
        } else {
            let playlist_title = infos
                .first()
                .and_then(|i| i.playlist_title.as_deref())
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;
    use crate::handle::DownloadId;
    use rdlp_types::Config;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use tracing_subscriber::fmt::MakeWriter;

    /// The span field carries a redacted URL, not the raw one.
    ///
    /// `#[instrument(fields(url = %url))]` interpolated a raw `&str` into a
    /// record that persists to the desktop's `LogDir`. The redaction gate
    /// cannot see this shape — its rule A1 matches a structured-kv sigil
    /// left of the `=`, while an instrument field puts the sigil on the
    /// right (`url = %url`).
    #[test]
    fn the_extraction_span_field_is_redacted() {
        let raw = "https://user:hunter2@example.com/v";
        let shown = rdlp_redact::RedactedUrl::new(raw).to_string();
        assert!(!shown.contains("hunter2"), "got: {shown}");
    }

    /// A writer that appends into a shared buffer, so the test can inspect
    /// exactly what `tracing_subscriber::fmt` rendered for the span.
    #[derive(Clone, Default)]
    struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CapturedOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedOutput {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn test_orchestrator() -> Orchestrator {
        let config = Arc::new(Config::default());
        let (tx, _rx) = mpsc::channel::<crate::events::Event>(64);
        let id = DownloadId::next();
        let token = CancellationToken::new();
        Orchestrator::new(config, tx, id, token, None)
    }

    /// Exercises the real `tracing` -> subscriber pipeline (not just the
    /// redaction helper in isolation), so a regression that puts the raw
    /// `url` back into `fields(...)`, or that reverts the span to INFO,
    /// fails this test. This is the load-bearing check the brief's Step 4
    /// calls a "live run" — captured here instead of shelling out to the
    /// CLI, so it runs in the normal suite and in CI.
    #[test]
    fn the_extraction_span_is_debug_and_redacted_in_real_output() {
        let captured = CapturedOutput::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            // The function body logs via the `log` facade (no `LogTracer`
            // bridge is installed here), so nothing inside the span would
            // otherwise reach this subscriber. Recording span creation
            // itself is what surfaces the `url` field and the span's level.
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
            .finish();

        let orchestrator = test_orchestrator();
        let raw = "https://user:hunter2@example.invalid/v";

        tracing::subscriber::with_default(subscriber, || {
            // `with_default` is thread-local; a multi-thread runtime would
            // run the instrumented future on a worker thread that never saw
            // it, so use `current_thread` to keep everything on this one.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            // No extractor matches this host; the call errors, but the span
            // is opened (and its fields recorded) before that happens.
            rt.block_on(async {
                let _ = orchestrator.extract_video(raw).await;
            });
        });

        let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("extract_video"),
            "expected the span in the captured output: {output}"
        );
        assert!(
            !output.contains("hunter2"),
            "raw credential leaked into the span's recorded output: {output}"
        );
        assert!(
            !output.contains("INFO extract_video") && !output.contains("INFO rdlp_api"),
            "expected the span at DEBUG, found an INFO record: {output}"
        );
        assert!(
            output.contains("DEBUG"),
            "expected at least one DEBUG-level record for the span: {output}"
        );
    }
}
