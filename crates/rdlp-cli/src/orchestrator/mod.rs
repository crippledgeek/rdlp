//! Orchestrator module for coordinating extraction, download, and post-processing

mod errors;
mod execution;
mod extraction;
mod paths;
mod playlist;
mod postprocess;
mod resume;
mod selection;
mod state;

#[cfg(test)]
mod tests;

// Public re-exports
pub use errors::{OrchestratorError, Result};
pub use state::{DownloadPhase, DownloadState};

use log::{debug, warn};
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::{Config, ExtractionContext};
use rdlp_downloader::{DownloaderRegistry, DownloaderRegistryTrait};
use rdlp_extractor::{ExtractorRegistry, ExtractorRegistryTrait};
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::SimpleJsEngine;
use rdlp_postprocess::{PostProcessorRegistry, PostProcessorRegistryTrait};
use std::path::PathBuf;
use std::sync::Arc;

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    pub(super) extractor_registry: Arc<dyn ExtractorRegistryTrait>,
    pub(super) downloader_registry: Arc<dyn DownloaderRegistryTrait>,
    pub(super) postprocessor_registry: Option<Arc<dyn PostProcessorRegistryTrait>>,
    pub(super) extraction_context: Arc<ExtractionContext>,
    pub(super) config: Arc<Config>,
}

impl Orchestrator {
    /// Create a new orchestrator with default registries
    pub fn new(config: Config) -> Self {
        let http_client = HttpClientFactory::from_rdlp_config(&config).build_arc();
        let js_engine = Arc::new(SimpleJsEngine::new());
        let cookie_jar = Arc::new(SimpleCookieJar::new());

        // Wrap config in Arc once and share it
        let config = Arc::new(config);

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config), // Cheap Arc clone instead of deep clone
        ));

        // Initialize post-processor registry (optional - graceful degradation if FFmpeg not found)
        let postprocessor_registry = Self::create_postprocessor_registry(&config);

        Self {
            extractor_registry: Arc::new(ExtractorRegistry::new()),
            downloader_registry: Arc::new(DownloaderRegistry::new()),
            postprocessor_registry,
            extraction_context,
            config,
        }
    }

    /// Create post-processor registry with optional FFmpeg location
    ///
    /// Returns None if FFmpeg is not found (graceful degradation)
    fn create_postprocessor_registry(
        config: &Config,
    ) -> Option<Arc<dyn PostProcessorRegistryTrait>> {
        let registry_result = if let Some(ref ffmpeg_path) = config.ffmpeg_location {
            PostProcessorRegistry::with_ffmpeg_location(Some(ffmpeg_path.as_path()))
        } else {
            PostProcessorRegistry::new()
        };

        match registry_result {
            Ok(registry) => {
                debug!("[PostProcess] FFmpeg initialized successfully");
                Some(Arc::new(registry))
            }
            Err(e) => {
                // Warn about FFmpeg not being found (needed for HLS fixup)
                warn!("[PostProcess] FFmpeg NOT found: {e}");
                None
            }
        }
    }

    /// Create a new orchestrator with custom registries (for testing)
    ///
    /// This method is primarily used for integration tests to inject mock registries.
    /// It's public to allow integration tests to use it, but should not be used in
    /// production code.
    #[doc(hidden)]
    pub fn with_registries(
        config: Config,
        extractor_registry: Arc<dyn ExtractorRegistryTrait>,
        downloader_registry: Arc<dyn DownloaderRegistryTrait>,
    ) -> Self {
        let http_client = HttpClientFactory::from_rdlp_config(&config).build_arc();
        let js_engine = Arc::new(SimpleJsEngine::new());
        let cookie_jar = Arc::new(SimpleCookieJar::new());

        let config = Arc::new(config);

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config),
        ));

        // Initialize post-processor registry (optional)
        let postprocessor_registry = Self::create_postprocessor_registry(&config);

        Self {
            extractor_registry,
            downloader_registry,
            postprocessor_registry,
            extraction_context,
            config,
        }
    }

    /// Download a video from URL using state machine pattern
    ///
    /// # State Machine Workflow
    ///
    /// This method implements an explicit state machine for the download workflow:
    /// 1. `Extracting` - Find extractor and extract video metadata
    /// 2. `SelectingFormat` - Choose format (interactive or automatic)
    /// 3. `Preparing` - Detect resume point and prepare for download
    /// 4. `Downloading` - Execute download with progress tracking
    /// 5. `Complete` - Return downloaded file path
    ///
    /// At any point, user can cancel (Ctrl+C or ESC) → `Cancelled` state
    ///
    /// **Note**: This method now auto-detects playlists! If the URL is a playlist,
    /// it will automatically delegate to `download_playlist()` and return the first
    /// video path (or None if playlist was empty/cancelled).
    ///
    /// # Returns
    ///
    /// - `Ok(Some(path))` - Download completed successfully
    /// - `Ok(None)` - User cancelled operation
    /// - `Err` - Error occurred during any phase
    pub async fn download(&self, url: &str, interactive: bool) -> Result<Option<PathBuf>> {
        // Try playlist extraction first to check if this is a playlist
        let infos = self.extract_playlist_info(url).await?;

        // If multiple videos found, this is a playlist
        if infos.len() > 1 {
            return self
                .download_playlist_internal(infos, interactive)
                .await
                .map(|opt| opt.and_then(|paths| paths.into_iter().next()));
        }

        // Single video - use existing state machine
        let mut phase = DownloadPhase::Extracting {
            url: url.to_string(),
        };

        loop {
            phase = phase.advance(self, interactive).await?;

            match phase {
                DownloadPhase::Complete { path } => return Ok(Some(path)),
                DownloadPhase::Cancelled => return Ok(None),
                _ => continue, // Keep advancing through phases
            }
        }
    }

    /// List all available extractors
    pub fn list_extractors(&self) -> Vec<String> {
        self.extractor_registry.list_extractors()
    }

    /// List all available download protocols
    pub fn list_downloaders(&self) -> Vec<String> {
        self.downloader_registry.list_downloaders()
    }
}
