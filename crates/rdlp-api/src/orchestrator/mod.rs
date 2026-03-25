//! Orchestrator module for coordinating extraction, download, and post-processing

mod archive;
mod container_resolver;
mod download;
mod errors;
mod execution;
mod extraction;
mod interactive;
mod merge_download;
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
use rdlp_core::ExtractionContext;
use rdlp_downloader::{DownloaderRegistry, DownloaderRegistryTrait};
use rdlp_extractor::{ExtractorRegistry, ExtractorRegistryTrait};
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::BoaJsEngine;
use rdlp_postprocess::{
    AudioExtractStage, MergeStage, MetadataStage, NormalizeStage, Pipeline, RecodeStage,
    RemuxStage, SubtitleStage, TempRegistry, ThumbnailStage,
};
use rdlp_types::{Config, Format};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

/// Plan for how to download the selected format(s).
///
/// `Single` downloads one combined format.
/// `Merge` downloads video-only and audio-only in parallel, then
/// delegates muxing to the `FFmpegMerger` postprocessor.
#[derive(Debug, Clone)]
// Format is large but DownloadPlan is always stored as Box<DownloadPlan>
// in DownloadPhase, so the enum's stack size doesn't affect the state machine.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DownloadPlan {
    /// Download a single combined format
    Single(Format),
    /// Download video and audio separately, then merge
    Merge {
        /// Video-only format
        video: Format,
        /// Audio-only format
        audio: Format,
    },
}

impl std::fmt::Display for DownloadPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(format) => write!(f, "single format {}", format.format_id),
            Self::Merge { video, audio } => {
                write!(
                    f,
                    "merge video {} + audio {}",
                    video.format_id, audio.format_id
                )
            }
        }
    }
}

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    pub(super) extractor_registry: Arc<dyn ExtractorRegistryTrait>,
    pub(super) downloader_registry: Arc<dyn DownloaderRegistryTrait>,
    /// Channel-based post-processing pipeline.
    pub(super) pipeline: Option<Arc<Pipeline>>,
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
    /// * `config` - Download configuration (Arc-wrapped for cheap sharing)
    /// * `event_tx` - Channel sender for lifecycle events
    /// * `download_id` - Unique identifier for this download
    /// * `cancel_token` - Token for cooperative cancellation
    /// * `interactive` - Optional callback for interactive user input
    #[must_use]
    pub fn new(
        config: Arc<Config>,
        event_tx: mpsc::Sender<Event>,
        download_id: DownloadId,
        cancel_token: CancellationToken,
        interactive: Option<Arc<dyn InteractiveCallback>>,
    ) -> Self {
        Self::new_with_registry(
            config,
            event_tx,
            download_id,
            cancel_token,
            interactive,
            None,
        )
    }

    /// Create a new orchestrator, sharing the given `TempRegistry` across all
    /// pipeline instances produced by this orchestrator.
    ///
    /// When `temp_registry` is `None` a fresh registry is created (same
    /// behaviour as [`new`](Self::new)).
    #[must_use]
    pub fn new_with_registry(
        config: Arc<Config>,
        event_tx: mpsc::Sender<Event>,
        download_id: DownloadId,
        cancel_token: CancellationToken,
        interactive: Option<Arc<dyn InteractiveCallback>>,
        temp_registry: Option<Arc<TempRegistry>>,
    ) -> Self {
        let cookie_jar = Arc::new(SimpleCookieJar::new());
        let raw_jar = cookie_jar.jar(); // Capture before cookie_jar moves into ExtractionContext
        let http_client =
            HttpClientFactory::from_rdlp_config(&config).build_arc_with_cookies(raw_jar.clone());
        let js_engine = Arc::new(BoaJsEngine::new());

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config), // Cheap Arc clone instead of deep clone
        ));

        let registry = temp_registry.unwrap_or_else(|| Arc::new(TempRegistry::new()));
        let pipeline = Self::create_pipeline(&config, registry);

        // Cache extractor registry across orchestrator instances — it's stateless
        // and immutable, so constructing it once saves ~5ms per API call.
        static EXTRACTOR_REGISTRY: std::sync::OnceLock<Arc<ExtractorRegistry>> =
            std::sync::OnceLock::new();
        let extractor_registry =
            Arc::clone(EXTRACTOR_REGISTRY.get_or_init(|| Arc::new(ExtractorRegistry::new())));

        Self {
            extractor_registry,
            downloader_registry: Arc::new(DownloaderRegistry::with_config_and_cookies(
                &config, raw_jar,
            )),
            pipeline,
            extraction_context,
            config,
            event_tx,
            download_id,
            cancel_token,
            interactive,
        }
    }

    /// Build the channel-based post-processing pipeline.
    ///
    /// Returns `None` if FFmpeg is not available (graceful degradation).
    fn create_pipeline(config: &Config, temp_registry: Arc<TempRegistry>) -> Option<Arc<Pipeline>> {
        let ffmpeg =
            match rdlp_ffmpeg::FFmpegRunner::with_location(config.ffmpeg_location.as_deref()) {
                Ok(f) => {
                    debug!("FFmpeg initialized successfully");
                    rdlp_ffmpeg::set_verbose(config.verbose);
                    Arc::new(f)
                }
                Err(e) => {
                    warn!("FFmpeg NOT found: {e}");
                    return None;
                }
            };

        // Stage order: 0→Merge 1→AudioExtract 2→Normalize 3→Remux 4→Recode 5→Subtitle 6→Metadata 7→Thumbnail
        let stages: Vec<Arc<dyn rdlp_postprocess::pipeline::PipelineStage>> = vec![
            Arc::new(MergeStage::new(Arc::clone(&ffmpeg))),
            Arc::new(AudioExtractStage::new(Arc::clone(&ffmpeg))),
            Arc::new(NormalizeStage::new(Arc::clone(&ffmpeg))),
            Arc::new(RemuxStage::new(Arc::clone(&ffmpeg))),
            Arc::new(RecodeStage::new(Arc::clone(&ffmpeg))),
            Arc::new(SubtitleStage::new(Arc::clone(&ffmpeg))),
            Arc::new(MetadataStage::new(Arc::clone(&ffmpeg))),
            Arc::new(ThumbnailStage::new(ffmpeg)),
        ];

        Some(Arc::new(Pipeline::new(stages, temp_registry, 2)))
    }

    /// Create a new orchestrator with custom registries (for testing)
    ///
    /// This method is primarily used for integration tests to inject mock registries.
    /// It's public to allow integration tests to use it, but should not be used in
    /// production code.
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn with_registries(
        config: Arc<Config>,
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

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config),
        ));

        let pipeline = Self::create_pipeline(&config, Arc::new(TempRegistry::new()));

        Self {
            extractor_registry,
            downloader_registry,
            pipeline,
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
            debug!(count, file = path.display().to_string().as_str(); "Loaded cookies from file");
        }

        if let Some(browser) = self.config.cookies_from_browser {
            let count = cookie_jar.load_from_browser(browser).await.map_err(|e| {
                OrchestratorError::Configuration(format!(
                    "Failed to load cookies from {browser}: {e}"
                ))
            })?;
            debug!(count, browser = browser.to_string().as_str(); "Loaded cookies from browser");
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
    /// 3. `SelectingSubtitles` - Choose subtitles (interactive or config-based)
    /// 4. `Preparing` - Detect resume point and prepare for download
    /// 5. `Downloading` - Execute download with progress tracking
    /// 6. `Complete` - Return downloaded file path
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
            if self.config.output_to_stdout {
                return Err(OrchestratorError::Configuration(
                    "Playlists are not supported with -o - (stdout output); \
                     provide a single video URL instead"
                        .to_string(),
                ));
            }
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
            url: url.to_owned(),
        };

        loop {
            phase = phase.advance(self, interactive).await?;

            match phase {
                DownloadPhase::Complete { path } => {
                    // Record in archive after successful download
                    self.record_in_archive(&infos[0].extractor, &infos[0].id);
                    return Ok(Some(path));
                }
                DownloadPhase::Cancelled => return Ok(None),
                _ => continue, // Keep advancing through phases
            }
        }
    }

    /// Extract metadata without downloading (for --dump-json / --print / --simulate)
    pub async fn extract_info(&self, url: &str) -> Result<Vec<rdlp_types::InfoDict>> {
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
        info: &rdlp_types::InfoDict,
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
        if let Some(ref path) = self.config.download_archive
            && let Err(e) = archive::record_in_archive(path, extractor, id)
        {
            warn!("Failed to write to download archive: {e}");
        }
    }
}

#[cfg(test)]
mod download_plan_tests {
    use super::*;
    use rdlp_types::{DownloadProtocol, Format};

    fn make_format(id: &str) -> Format {
        Format::new(id, format!("url_{id}"), "mp4", DownloadProtocol::Https)
    }

    #[test]
    fn test_download_plan_single() {
        let fmt = make_format("f1");
        let plan = DownloadPlan::Single(fmt);
        assert!(matches!(plan, DownloadPlan::Single(f) if f.format_id == "f1"));
    }

    #[test]
    fn test_download_plan_merge() {
        let video = make_format("v1");
        let audio = make_format("a1");
        let plan = DownloadPlan::Merge { video, audio };
        match plan {
            DownloadPlan::Merge { video: v, audio: a } => {
                assert_eq!(v.format_id, "v1");
                assert_eq!(a.format_id, "a1");
            }
            _ => panic!("Expected Merge variant"),
        }
    }

    #[test]
    fn test_download_plan_display_single() {
        let fmt = make_format("f1");
        let plan = DownloadPlan::Single(fmt);
        let s = format!("{plan}");
        assert!(s.contains("f1"));
    }

    #[test]
    fn test_download_plan_display_merge() {
        let video = make_format("v1");
        let audio = make_format("a1");
        let plan = DownloadPlan::Merge { video, audio };
        let s = format!("{plan}");
        assert!(s.contains("v1"));
        assert!(s.contains("a1"));
    }
}
