//! Orchestrator module for coordinating extraction, download, and post-processing

mod archive;
mod download;
mod errors;
mod execution;
mod extraction;
mod interactive;
mod paths;
mod playlist;
mod postprocess;
mod resume;
mod selection;
mod session_state;
mod state;
mod subtitle;
mod subtitle_pipeline;
mod template;
mod thumbnail;

#[cfg(test)]
mod tests;

// Public re-exports
pub use errors::{OrchestratorError, Result};
pub use interactive::InteractiveCallback;
pub use state::DownloadPhase;

use crate::events::Event;
use crate::handle::DownloadId;
use log::{debug, info, warn};
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::{Config, ExtractionContext};
use rdlp_downloader::{DownloaderRegistry, DownloaderRegistryTrait};
use rdlp_extractor::{ExtractorRegistry, ExtractorRegistryTrait};
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::BoaJsEngine;
use rdlp_postprocess::{PostProcessorRegistry, PostProcessorRegistryTrait};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    pub(super) extractor_registry: Arc<dyn ExtractorRegistryTrait>,
    pub(super) downloader_registry: Arc<dyn DownloaderRegistryTrait>,
    pub(super) postprocessor_registry: Option<Arc<dyn PostProcessorRegistryTrait>>,
    pub(super) extraction_context: Arc<ExtractionContext>,
    pub(super) config: Arc<Config>,
    /// Event sender for download lifecycle events
    pub(super) event_tx: mpsc::Sender<Event>,
    /// Download identifier for events
    pub(super) download_id: DownloadId,
    /// Cancellation token for cooperative shutdown
    pub(super) cancel_token: CancellationToken,
    /// Optional interactive callback for user input
    pub(super) interactive: Option<Arc<dyn InteractiveCallback>>,
}

impl Orchestrator {
    /// Create a new orchestrator with default registries
    ///
    /// # Arguments
    /// * `config` - Download configuration
    /// * `event_tx` - Channel sender for lifecycle events
    /// * `download_id` - Unique identifier for this download
    /// * `cancel_token` - Token for cooperative cancellation
    /// * `interactive` - Optional callback for interactive user input
    #[must_use]
    pub fn new(
        config: Config,
        event_tx: mpsc::Sender<Event>,
        download_id: DownloadId,
        cancel_token: CancellationToken,
        interactive: Option<Arc<dyn InteractiveCallback>>,
    ) -> Self {
        let cookie_jar = Arc::new(SimpleCookieJar::new());
        let raw_jar = cookie_jar.jar(); // Capture before cookie_jar moves into ExtractionContext
        let http_client =
            HttpClientFactory::from_rdlp_config(&config).build_arc_with_cookies(raw_jar.clone());
        let js_engine = Arc::new(BoaJsEngine::new());

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
            downloader_registry: Arc::new(DownloaderRegistry::with_config_and_cookies(
                &config, raw_jar,
            )),
            postprocessor_registry,
            extraction_context,
            config,
            event_tx,
            download_id,
            cancel_token,
            interactive,
        }
    }

    /// Create post-processor registry with optional FFmpeg location
    ///
    /// Returns None if FFmpeg is not found (graceful degradation)
    fn create_postprocessor_registry(
        config: &Config,
    ) -> Option<Arc<dyn PostProcessorRegistryTrait>> {
        let registry_result =
            PostProcessorRegistry::with_ffmpeg_location(config.ffmpeg_location.as_deref());

        match registry_result {
            Ok(registry) => {
                debug!("FFmpeg initialized successfully");
                // Set FFmpeg log level based on verbose mode
                rdlp_ffmpeg::set_verbose(config.verbose);
                Some(Arc::new(registry))
            }
            Err(e) => {
                // Warn about FFmpeg not being found (needed for HLS fixup)
                warn!("FFmpeg NOT found: {e}");
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
    #[allow(dead_code)]
    pub fn with_registries(
        config: Config,
        extractor_registry: Arc<dyn ExtractorRegistryTrait>,
        downloader_registry: Arc<dyn DownloaderRegistryTrait>,
        event_tx: mpsc::Sender<Event>,
        download_id: DownloadId,
        cancel_token: CancellationToken,
        interactive: Option<Arc<dyn InteractiveCallback>>,
    ) -> Self {
        let cookie_jar = Arc::new(SimpleCookieJar::new());
        let http_client =
            HttpClientFactory::from_rdlp_config(&config).build_arc_with_cookies(cookie_jar.jar());
        let js_engine = Arc::new(BoaJsEngine::new());

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
            event_tx,
            download_id,
            cancel_token,
            interactive,
        }
    }

    /// Emit an event to the event channel, ignoring send failures.
    ///
    /// Uses `try_send` to avoid blocking on a full channel.
    pub(super) fn emit(&self, event: Event) {
        let _ = self.event_tx.try_send(event);
    }

    /// Load cookies from file or browser if configured.
    ///
    /// Should be called after construction and before any downloads.
    pub async fn load_cookies(&self) -> Result<()> {
        let cookie_jar = &self.extraction_context.cookie_jar;

        if let Some(ref path) = self.config.cookies_file {
            let count = cookie_jar.load_from_file(path).await.map_err(|e| {
                OrchestratorError::Configuration(format!(
                    "Failed to load cookies from {}: {e}",
                    path.display()
                ))
            })?;
            info!(count, file = path.display().to_string().as_str(); "Loaded cookies from file");
        }

        if let Some(browser) = self.config.cookies_from_browser {
            let count = cookie_jar.load_from_browser(browser).await.map_err(|e| {
                OrchestratorError::Configuration(format!(
                    "Failed to load cookies from {browser}: {e}"
                ))
            })?;
            info!(count, browser = browser.to_string().as_str(); "Loaded cookies from browser");
        }

        Ok(())
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
    /// At any point, user can cancel via `CancellationToken` → `Cancelled` state
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
    #[instrument(skip(self), fields(url = %url))]
    pub async fn download(&self, url: &str, interactive: bool) -> Result<Option<PathBuf>> {
        // Try playlist extraction first to check if this is a playlist
        let infos = self.extract_playlist_info(url).await?;

        // Load archive once at start
        let archive = self.load_archive_if_configured();

        // If multiple videos found, this is a playlist.
        // Playlist prints its own summary, so return None to suppress
        // the single-video "Success!" message in main.
        if infos.len() > 1 {
            return self
                .download_playlist_internal(infos, interactive, archive)
                .await
                .map(|_| None);
        }

        // Single video — check archive before downloading
        if let Some(ref archive_set) = archive {
            let info = &infos[0];
            if archive::is_in_archive(archive_set, &info.extractor, &info.id) {
                info!(
                    id = info.id.as_str(),
                    extractor = info.extractor.as_str();
                    "Already in archive, skipping"
                );
                return Ok(None);
            }
        }

        // Single video - use existing state machine
        let mut phase = DownloadPhase::Extracting {
            url: url.to_string(),
        };

        loop {
            phase = phase.advance(self, interactive).await?;

            match phase {
                DownloadPhase::Complete { ref path } => {
                    let result_path = path.clone();
                    // Record in archive after successful download
                    self.record_in_archive(&infos[0].extractor, &infos[0].id);
                    return Ok(Some(result_path));
                }
                DownloadPhase::Cancelled => return Ok(None),
                _ => continue, // Keep advancing through phases
            }
        }
    }

    /// Extract metadata without downloading (for --dump-json / --print / --simulate)
    pub async fn extract_info(&self, url: &str) -> Result<Vec<rdlp_core::InfoDict>> {
        self.extract_playlist_info(url).await
    }

    /// Download only subtitles (no video) for `--list-subs-only` mode.
    ///
    /// Shows interactive subtitle selection, downloads selected subtitle
    /// files, and returns their paths.
    ///
    /// # Arguments
    /// * `info` - Pre-extracted video metadata
    ///
    /// # Returns
    /// - `Ok(Some(paths))` - Downloaded subtitle file paths
    /// - `Ok(None)` - User cancelled
    pub async fn download_subtitles_only(
        &self,
        info: &rdlp_core::InfoDict,
    ) -> Result<Option<Vec<PathBuf>>> {
        self.download_subtitles_standalone(info).await
    }

    /// List all available extractors
    #[must_use]
    pub fn list_extractors(&self) -> Vec<&str> {
        self.extractor_registry.list_extractors()
    }

    /// List all available download protocols
    #[must_use]
    pub fn list_downloaders(&self) -> Vec<&str> {
        self.downloader_registry.list_downloaders()
    }

    /// Load archive if configured, returning `None` if not configured.
    fn load_archive_if_configured(&self) -> Option<HashSet<String>> {
        self.config
            .download_archive
            .as_ref()
            .map(|path| archive::load_archive(path))
    }

    /// Record a completed download in the archive (no-op if not configured).
    fn record_in_archive(&self, extractor: &str, id: &str) {
        if let Some(ref path) = self.config.download_archive {
            if let Err(e) = archive::record_in_archive(path, extractor, id) {
                warn!("Failed to write to download archive: {e}");
            }
        }
    }
}
