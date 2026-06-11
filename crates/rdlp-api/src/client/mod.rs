//! Primary API client and builder.
//!
//! [`RdlpClient`] is the main entry point for all frontends (CLI, Tauri,
//! Leptos). It is thread-safe and cheaply cloneable.
//!
//! # Usage
//!
//! ```no_run
//! use rdlp_api::client::RdlpClient;
//! use rdlp_api::request::DownloadRequest;
//! use rdlp_types::Config;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = RdlpClient::new(Config::default())?;
//! let mut handle = client.download(DownloadRequest::new("https://example.com/video"));
//!
//! while let Some(event) = handle.events().recv().await {
//!     println!("{event:?}");
//! }
//!
//! let result = handle.wait().await?;
//! println!("Downloaded to: {:?}", result.output_files);
//! # Ok(())
//! # }
//! ```

use crate::errors::RdlpApiError;
use crate::events::Event;
use crate::handle::{DownloadHandle, DownloadId};
use crate::merge::MergeOverrides;
use crate::orchestrator::{InteractiveCallback, Orchestrator, SharedHttpClient};
use crate::plugin_bootstrap::build_registry_with_plugins;
use crate::request::DownloadRequest;
use crate::result::DownloadResult;
use log::{error, warn};
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::DownloadStats;
use rdlp_extractor::ExtractorRegistry;
use rdlp_http::HttpClientFactory;
use rdlp_postprocess::TempRegistry;
use rdlp_types::{Config, InfoDict};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Maximum number of re-extraction retries after a reextractable error.
const MAX_REEXTRACT_RETRIES: u32 = 2;

/// Delay (in seconds) before each re-extraction attempt.
const REEXTRACT_DELAYS_SECS: [u64; 2] = [5, 15];

#[cfg(test)]
mod tests;

/// Returns `true` when `net` overrides a field that
/// `rdlp_http::HttpClientConfig::from_rdlp_config` bakes into the wreq `Client`
/// at build time. Such a request cannot reuse the `RdlpClient`-level shared
/// client and must get its own (PRD 2026-06-02 item 2).
///
/// MUST stay in sync with the client-level fields in `rdlp_http::HttpClientConfig::from_rdlp_config`:
/// proxy, connect timeout (`socket_timeout`), read timeout, pool idle timeout, and
/// cookie source. Registry/downloader-level fields (`rate_limit`,
/// `concurrent_fragments`, `retries`, format/output/postprocess) are applied
/// per-download by the registry and do NOT force a dedicated client.
pub(crate) const fn request_needs_dedicated_client(net: &crate::request::NetworkOptions) -> bool {
    net.proxy.is_some()
        || net.timeout_secs.is_some()
        || net.read_timeout_secs.is_some()
        || net.pool_idle_timeout_secs.is_some()
        || net.cookies_from_browser.is_some()
        || net.cookies_file.is_some()
}

/// Map a terminal download error to the event that should be emitted for it.
///
/// A user cancellation MUST surface as [`Event::Cancelled`] — never
/// [`Event::Failed`] — so the UI buckets a deliberate cancel as cancelled
/// rather than a failure. This matters most for cancels during post-processing
/// (e.g. aborting a slow recode): the pipeline returns
/// [`RdlpApiError::UserCancelled`] up the SAME error path as a genuine failure,
/// so without this branch a cancelled recode lands in the "Failed" bucket.
/// Every other error is a real failure.
fn terminal_event(id: DownloadId, error: &RdlpApiError) -> Event {
    if matches!(error, RdlpApiError::UserCancelled) {
        Event::Cancelled { id }
    } else {
        Event::Failed {
            id,
            error: error.clone(),
        }
    }
}

/// Primary entry point for the rdlp download engine.
///
/// Created via [`RdlpClient::builder()`] or [`RdlpClient::new()`].
/// Thread-safe and cheaply cloneable — all internal state is `Arc`-wrapped.
#[derive(Clone)]
pub struct RdlpClient {
    config: Arc<Config>,
    interactive: Option<Arc<dyn InteractiveCallback>>,
    /// Shared registry for crash-safe temp file cleanup.
    ///
    /// A single registry is shared across all pipeline instances created
    /// by this client. On clean exit, call `temp_registry().cleanup_all()`
    /// to remove any files that were not cleaned up by a completed pipeline
    /// run (e.g. if the process was killed mid-download).
    temp_registry: Arc<TempRegistry>,
    /// Pre-built extractor registry (built-ins + any plugins loaded at
    /// client construction time). Shared across all downloads from this client.
    extractor_registry: Arc<ExtractorRegistry>,
    /// Pre-built downloader registry — exposed only so `list_downloaders`
    /// can return borrowed `&str` names per the project rule. The
    /// orchestrator builds its own registry per download (with cookies +
    /// per-config customization), so this field is for the listing API
    /// surface only.
    downloader_registry: Arc<rdlp_downloader::DownloaderRegistry>,
    /// One shared HTTP client reused across `download()` calls whose requests
    /// don't override a client-level network field (PRD 2026-06-02 item 2).
    shared_client: Arc<wreq::Client>,
    /// Cookie jar baked into `shared_client`.
    shared_cookie_jar: Arc<SimpleCookieJar>,
}

// The Debug impl intentionally omits `temp_registry`, `extractor_registry`, and
// `downloader_registry` to avoid printing thousands of lines of internals.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for RdlpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RdlpClient")
            .field("config", &self.config)
            .field(
                "interactive",
                &self.interactive.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

impl RdlpClient {
    /// Create a client with default registries and the given config.
    ///
    /// # Errors
    ///
    /// Returns [`RdlpApiError::BuilderError`] if config validation fails.
    pub fn new(config: Config) -> Result<Self, RdlpApiError> {
        Self::builder().config(config).build()
    }

    /// Return the shared [`TempRegistry`] for this client.
    ///
    /// Callers (desktop, CLI) should call `cleanup_all()` on this registry
    /// when the process is about to exit so any in-flight temp files created
    /// by aborted downloads are removed.
    #[must_use]
    pub const fn temp_registry(&self) -> &Arc<TempRegistry> {
        &self.temp_registry
    }

    /// Create a new builder.
    #[must_use]
    pub fn builder() -> RdlpClientBuilder {
        RdlpClientBuilder::new()
    }

    /// Start a download, returning a handle immediately.
    ///
    /// The download runs in a background Tokio task. Use the returned
    /// [`DownloadHandle`] to receive events, cancel, or wait for completion.
    ///
    /// # Arguments
    ///
    /// * `request` - Download configuration for this specific download.
    #[must_use]
    #[allow(clippy::too_many_lines)] // single async state machine; extracting sub-tasks would hide the retry logic
    pub fn download(&self, request: DownloadRequest) -> DownloadHandle {
        let id = DownloadId::next();
        let (tx, rx) = mpsc::channel::<Event>(256);
        let cancel_token = CancellationToken::new();

        let cancel_disposition = crate::cancel::CancelDisposition::new();
        let shared_http = self.shared_http_for(&request);
        let config = Arc::new(self.build_config(&request));
        let interactive_flag = request.format.interactive;
        let url = request.url;
        let interactive_cb = self.interactive.clone();
        let token = cancel_token.clone();
        let disposition_for_task = cancel_disposition.clone();
        let registry = Arc::clone(&self.temp_registry);
        let extractor_registry = Arc::clone(&self.extractor_registry);

        // Capture whether the user explicitly requested cookies.
        // When explicit, cookie loading failure must be fatal.
        let cookies_explicitly_requested =
            config.cookies_from_browser.is_some() || config.cookies_file.is_some();

        let join_handle = tokio::spawn(async move {
            let orchestrator = Orchestrator::new_with_shared(
                config,
                tx.clone(),
                id,
                token,
                interactive_cb,
                Some(registry),
                Some(extractor_registry),
                shared_http,
                disposition_for_task,
            );

            // Load cookies: fatal when explicitly requested, non-fatal otherwise
            if let Err(e) = orchestrator.load_cookies().await {
                if cookies_explicitly_requested {
                    // Sanitize the error message to avoid leaking DPAPI
                    // errors, decryption keys, or raw cookie values.
                    // On Windows, Chrome locks its cookie DB exclusively
                    // via LockFileEx — the file cannot be copied while
                    // Chrome is running (see yt-dlp/yt-dlp#7271).
                    let safe_msg = "Failed to load cookies: browser cookie extraction \
                         failed. Close the browser completely (including background \
                         processes), then retry. Alternatively, export cookies to a \
                         Netscape file and use --cookies instead."
                        .to_string();
                    error!("Cookie loading failed (explicit request): {e}");
                    let api_err = RdlpApiError::IoError { message: safe_msg };
                    // intentional: receiver may have disconnected
                    let _ = tx
                        .send(Event::Failed {
                            id,
                            error: api_err.clone(),
                        })
                        .await;
                    return Err(api_err);
                }
                // intentional: receiver may have disconnected
                let _ = tx
                    .send(Event::Warning {
                        id,
                        message: format!("Cookie loading failed: {e}"),
                    })
                    .await;
            }

            let mut last_err: Option<RdlpApiError> = None;
            for attempt in 0..=MAX_REEXTRACT_RETRIES {
                if attempt > 0 {
                    let delay = REEXTRACT_DELAYS_SECS
                        .get((attempt - 1) as usize)
                        .copied()
                        .unwrap_or(15); // fallback: use max delay if index out of range
                    let reason = match &last_err {
                        Some(RdlpApiError::NetworkError {
                            status: Some(403), ..
                        }) => "HTTP 403 Forbidden",
                        Some(RdlpApiError::NetworkError {
                            status: Some(503), ..
                        }) => "HTTP 503 Service Unavailable",
                        _ => "stale CDN URLs",
                    };
                    warn!(
                        "Re-extracting fresh URLs (attempt {}/{}, {})",
                        attempt + 1,
                        MAX_REEXTRACT_RETRIES + 1,
                        reason,
                    );
                    // intentional: receiver may have disconnected
                    let _ = tx
                        .send(Event::Retrying {
                            id,
                            attempt,
                            max_attempts: MAX_REEXTRACT_RETRIES + 1,
                            reason: format!("Re-extracting fresh URLs ({reason})"),
                        })
                        .await;
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                }

                let download_result = if interactive_flag {
                    orchestrator.download_interactive(&url).await
                } else {
                    orchestrator.download(&url).await
                };
                match download_result {
                    Ok(Some(path)) => {
                        // Stdout mode returns "-" sentinel — don't
                        // expose it as a real file path to API consumers.
                        let output_files = if path.as_os_str() == "-" {
                            vec![]
                        } else {
                            vec![path]
                        };
                        let result = DownloadResult {
                            id,
                            output_files,
                            info: InfoDict::new("", "", "", &url),
                            stats: DownloadStats::new(0, Duration::ZERO, 0),
                        };
                        // intentional: receiver may have disconnected
                        let _ = tx
                            .send(Event::Completed {
                                id,
                                output_files: result.output_files.clone(),
                            })
                            .await;
                        return Ok(result);
                    }
                    Ok(None) => {
                        // User cancelled — exit immediately
                        return Err(RdlpApiError::UserCancelled);
                    }
                    Err(e) => {
                        let api_err = RdlpApiError::from(e);
                        if attempt < MAX_REEXTRACT_RETRIES && api_err.is_reextractable() {
                            last_err = Some(api_err);
                            continue;
                        }
                        // Final failure — emit and return (cancellation
                        // surfaces as Event::Cancelled, not Event::Failed).
                        // intentional: receiver may have disconnected
                        let _ = tx.send(terminal_event(id, &api_err)).await;
                        return Err(api_err);
                    }
                }
            }
            // All retries exhausted (unreachable in practice, but
            // satisfies the compiler)
            let err = last_err.unwrap_or_else(|| RdlpApiError::NetworkError {
                message: "All re-extraction attempts failed".into(),
                status: None,
            });
            // intentional: receiver may have disconnected
            let _ = tx.send(terminal_event(id, &err)).await;
            Err(err)
        });

        DownloadHandle::new(id, rx, cancel_token, cancel_disposition, join_handle)
    }

    /// Extract metadata without downloading.
    ///
    /// # Arguments
    ///
    /// * `url` - URL to extract metadata from.
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails or no extractor matches.
    pub async fn extract_info(&self, url: &str) -> Result<Vec<InfoDict>, RdlpApiError> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(16);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            None,
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator
            .load_cookies()
            .await
            .map_err(RdlpApiError::from)?;
        orchestrator.extract_info(url).await.map_err(Into::into)
    }

    /// Download only subtitles (no video) for the given metadata.
    ///
    /// Shows interactive subtitle selection if a callback is configured,
    /// downloads selected subtitle files, and returns their paths.
    ///
    /// # Arguments
    ///
    /// * `info` - Pre-extracted video metadata.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(paths))` — Downloaded subtitle file paths.
    /// - `Ok(None)` — User cancelled the selection.
    ///
    /// # Errors
    ///
    /// Returns an error if subtitle download fails.
    pub async fn download_subtitles_only(
        &self,
        info: &InfoDict,
    ) -> Result<Option<Vec<std::path::PathBuf>>, RdlpApiError> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(16);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            self.interactive.clone(),
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator
            .load_cookies()
            .await
            .map_err(RdlpApiError::from)?;
        orchestrator
            .download_subtitles_only(info)
            .await
            .map_err(Into::into)
    }

    /// List available extractors.
    ///
    /// Returns `Vec<&str>` borrowed from the client's pre-built
    /// `ExtractorRegistry` (per the project rule: registry methods return
    /// borrowed names, not allocated `String`s). The reference is tied
    /// to `&self` so the returned slice lives as long as the client.
    #[must_use]
    pub fn list_extractors(&self) -> Vec<&str> {
        self.extractor_registry.list_extractors()
    }

    /// List sites that support search.
    ///
    /// # Returns
    /// A list of `SearchSiteInfo` with name and display name.
    #[must_use]
    pub fn list_search_sites(&self) -> Vec<rdlp_types::SearchSiteInfo> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            None,
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator
            .list_search_extractors()
            .into_iter()
            .map(|name| rdlp_types::SearchSiteInfo {
                name: name.to_lowercase(),
                display_name: name.to_string(),
            })
            .collect()
    }

    /// Get available search filters for a site.
    ///
    /// # Arguments
    /// * `site` - Site name as returned by `list_search_sites()`.
    ///
    /// # Errors
    /// Returns error if the site name is not recognized.
    pub fn search_filters(
        &self,
        site: &str,
    ) -> Result<Vec<rdlp_types::SearchFilterDescriptor>, RdlpApiError> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            None,
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator
            .search_filters(site)
            .map_err(|e| RdlpApiError::InvalidInput {
                message: e.to_string(),
            })
    }

    /// Execute a keyword search on a site.
    ///
    /// # Arguments
    /// * `site` - Site name (e.g., "xhamster").
    /// * `query` - Search query with optional filters.
    ///
    /// # Returns
    /// Lightweight result previews. Call `extract_info()` on each
    /// `SearchResultPreview::video_url` for full metadata.
    ///
    /// # Errors
    /// Returns error if site unknown, filters invalid, or network fails.
    pub async fn search(
        &self,
        site: &str,
        query: &rdlp_types::SearchQuery,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>, RdlpApiError> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(16);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            None,
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator.search(site, query).await.map_err(Into::into)
    }

    /// Execute a paginated search on a site, returning a single page.
    ///
    /// # Arguments
    /// * `site` - Site name (e.g., "xhamster").
    /// * `query` - Search query with page number set.
    ///
    /// # Returns
    /// A single page of results with pagination metadata.
    ///
    /// # Errors
    /// Returns error if site unknown, filters invalid, or network fails.
    pub async fn search_page(
        &self,
        site: &str,
        query: &rdlp_types::SearchQuery,
    ) -> Result<rdlp_types::SearchPageResponse, RdlpApiError> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(16);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            None,
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator
            .search_page(site, query)
            .await
            .map_err(Into::into)
    }

    /// Execute a search and return results as lightweight previews.
    ///
    /// Alias for [`search()`](Self::search) — only scrapes search result
    /// pages, does not fetch individual video pages.
    ///
    /// # Errors
    ///
    /// Returns error if site unknown, filters invalid, or network fails.
    pub async fn search_preview(
        &self,
        site: &str,
        query: &rdlp_types::SearchQuery,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>, RdlpApiError> {
        self.search(site, query).await
    }

    /// Lazily enrich a single previously-returned `SearchResultPreview`.
    ///
    /// Frontends call this on demand (e.g. when a row scrolls into view)
    /// to fill metadata gaps the cheap search path could not — at most one
    /// HTTP request to the underlying video page per call. Sites whose
    /// search-card markup is already complete return the preview unchanged.
    ///
    /// # Errors
    /// Returns error if site unknown, network fails, or the user cancels.
    pub async fn enrich_search_result(
        &self,
        site: &str,
        preview: rdlp_types::SearchResultPreview,
    ) -> Result<rdlp_types::SearchResultPreview, RdlpApiError> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new_with_registry(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            None,
            None,
            Some(Arc::clone(&self.extractor_registry)),
        );
        orchestrator
            .enrich_search_result(site, preview)
            .await
            .map_err(Into::into)
    }

    /// List available download protocols.
    ///
    /// Returns `Vec<&str>` borrowed from the client's pre-built
    /// `DownloaderRegistry` (per the project rule: registry methods
    /// return borrowed names, not allocated `String`s).
    #[must_use]
    pub fn list_downloaders(&self) -> Vec<&str> {
        self.downloader_registry.list_downloaders()
    }

    /// Post-process a local file without downloading.
    ///
    /// Applies the configured post-processing pipeline (normalization, remux,
    /// audio extraction, etc.) to an existing local file. Returns a
    /// [`DownloadHandle`] with the same event/progress interface as `download()`.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the local file to process.
    #[must_use]
    pub fn process_local_file(&self, path: std::path::PathBuf) -> DownloadHandle {
        let id = DownloadId::next();
        let (tx, rx) = mpsc::channel::<Event>(256);
        let cancel_token = CancellationToken::new();
        let config = Arc::clone(&self.config);
        let token = cancel_token.clone();
        let registry = Arc::clone(&self.temp_registry);

        let join_handle = tokio::spawn(async move {
            let orchestrator = Orchestrator::new_with_registry(
                config,
                tx.clone(),
                id,
                token,
                None,
                Some(registry),
                None, // local file processing does not need plugin extractors
            );

            // Build a minimal InfoDict from the file path
            let file_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let info = rdlp_types::InfoDict::new(
                &file_name,
                &file_name,
                "local",
                format!("file://{}", path.display()),
            );
            // intentional: receiver may have disconnected
            let _ = tx
                .send(Event::PostProcessing {
                    id,
                    stage: "Starting post-processing".to_string(),
                })
                .await;

            if !path.exists() {
                let err = RdlpApiError::IoError {
                    message: format!("File not found: {}", path.display()),
                };
                // intentional: receiver may have disconnected
                let _ = tx
                    .send(Event::Failed {
                        id,
                        error: err.clone(),
                    })
                    .await;
                return Err(err);
            }

            match orchestrator
                .run_postprocessing(&info, vec![path], false, true) // keep_inputs=true: user's source is borrowed, never deleted (#414)
                .await
            {
                Ok(output_files) => {
                    // Pipeline returns temp-named survivors (#406 Option X);
                    // finalize each to its clean name before reporting.
                    let mut finalized = Vec::with_capacity(output_files.len());
                    for f in output_files {
                        match crate::orchestrator::naming::finalize_to_clean(&f) {
                            Some(clean) => {
                                crate::orchestrator::naming::finalize_part(&f, &clean)
                                    .await
                                    .map_err(|e| RdlpApiError::IoError {
                                        message: format!("{e:#}"),
                                    })?;
                                finalized.push(clean);
                            }
                            None => finalized.push(f),
                        }
                    }
                    let result = crate::DownloadResult {
                        id,
                        output_files: finalized,
                        info,
                        stats: rdlp_core::DownloadStats::new(0, std::time::Duration::ZERO, 0),
                    };
                    // intentional: receiver may have disconnected
                    let _ = tx
                        .send(Event::Completed {
                            id,
                            output_files: result.output_files.clone(),
                        })
                        .await;
                    Ok(result)
                }
                Err(e) => {
                    let err = RdlpApiError::from(e);
                    // intentional: receiver may have disconnected
                    // (cancellation surfaces as Event::Cancelled, not Failed).
                    let _ = tx.send(terminal_event(id, &err)).await;
                    Err(err)
                }
            }
        });

        DownloadHandle::new(
            id,
            rx,
            cancel_token,
            crate::cancel::CancelDisposition::new(),
            join_handle,
        )
    }

    /// Returns the shared client+jar to reuse for `request`, or `None` if it
    /// overrides a client-level network field and needs its own client.
    fn shared_http_for(&self, request: &DownloadRequest) -> Option<SharedHttpClient> {
        if request_needs_dedicated_client(&request.network) {
            None
        } else {
            Some(SharedHttpClient {
                client: Arc::clone(&self.shared_client),
                cookie_jar: Arc::clone(&self.shared_cookie_jar),
            })
        }
    }

    /// Merge request options into a Config, applying overrides from the request.
    fn build_config(&self, request: &DownloadRequest) -> Config {
        let mut config = (*self.config).clone();
        request.output.merge_into(&mut config);
        request.format.merge_into(&mut config);
        request.subtitles.merge_into(&mut config);
        request.postprocess.merge_into(&mut config);
        request.network.merge_into(&mut config);
        if let Some(v) = request.verbose {
            config.verbose = v;
        }
        config
    }
}

/// Builder for [`RdlpClient`].
pub struct RdlpClientBuilder {
    config: Option<Config>,
    interactive: Option<Arc<dyn InteractiveCallback>>,
    temp_registry: Option<Arc<TempRegistry>>,
}

impl RdlpClientBuilder {
    /// Create a new builder with no defaults set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            interactive: None,
            temp_registry: None,
        }
    }

    /// Set the base configuration.
    ///
    /// Request-level overrides (via [`DownloadRequest`]) are applied on
    /// top of this config for each download.
    #[must_use]
    pub fn config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the interactive callback for user input (format selection, etc.).
    ///
    /// If not set, interactive mode will fall back to automatic selection
    /// even when `FormatOptions::interactive` is `true`.
    #[must_use]
    pub fn interactive(mut self, callback: Arc<dyn InteractiveCallback>) -> Self {
        self.interactive = Some(callback);
        self
    }

    /// Provide a pre-existing [`TempRegistry`] to share across all pipeline
    /// instances created by this client.
    ///
    /// When not set, a fresh registry is created during [`build`](Self::build).
    /// This method is intended for desktop/CLI callers that need to hold the
    /// registry reference independently (for `cleanup_all()` on exit).
    #[must_use]
    pub fn temp_registry(mut self, registry: Arc<TempRegistry>) -> Self {
        self.temp_registry = Some(registry);
        self
    }

    /// Build the client.
    ///
    /// # Errors
    ///
    /// Returns [`RdlpApiError::BuilderError`] if `config` was not provided.
    pub fn build(self) -> Result<RdlpClient, RdlpApiError> {
        let config = self.config.ok_or_else(|| RdlpApiError::BuilderError {
            message: "config is required".into(),
        })?;

        let temp_registry = self
            .temp_registry
            .unwrap_or_else(|| Arc::new(TempRegistry::new()));

        // Bootstrap plugins at client construction time. This runs once per
        // client instance (not once per download). Fail-soft: broken plugin
        // directories emit warnings and are skipped; built-in extractors
        // are always present in the returned registry.
        let extractor_registry = Arc::new(build_registry_with_plugins(&config));
        // Listing-only registry — see field doc on RdlpClient.
        let downloader_registry = Arc::new(rdlp_downloader::DownloaderRegistry::new());

        // One client + jar reused across download() calls that don't override a
        // client-level network field (see `request_needs_dedicated_client`). The
        // jar starts empty; each shared-path download calls `load_cookies()` into
        // it. When the base config carries a cookie source it is therefore
        // re-read per download (idempotent — cookies are domain-scoped and wreq's
        // Jar is Sync); loading base cookies once is a deferred optimization
        // (PRD 2026-06-02 item 2 follow-up).
        let shared_cookie_jar = Arc::new(SimpleCookieJar::new());
        let shared_client = HttpClientFactory::from_rdlp_config(&config)
            .build_arc_with_cookies(shared_cookie_jar.jar());

        Ok(RdlpClient {
            config: Arc::new(config),
            interactive: self.interactive,
            temp_registry,
            extractor_registry,
            downloader_registry,
            shared_client,
            shared_cookie_jar,
        })
    }
}

impl Default for RdlpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod shared_client_tests {
    use super::*;
    use crate::request::{DownloadRequest, NetworkOptions};
    use rdlp_types::BrowserType;
    use std::path::PathBuf;

    #[test]
    fn empty_network_options_do_not_need_dedicated_client() {
        assert!(!request_needs_dedicated_client(&NetworkOptions::default()));
    }

    #[test]
    fn registry_level_overrides_stay_on_shared_client() {
        let net = NetworkOptions {
            rate_limit: Some(1_000_000),
            concurrent_fragments: Some(8),
            retries: Some(5),
            ..NetworkOptions::default()
        };
        assert!(!request_needs_dedicated_client(&net));
    }

    #[test]
    fn each_client_level_override_forces_dedicated_client() {
        let cases = [
            NetworkOptions {
                proxy: Some("http://p:3128".into()),
                ..Default::default()
            },
            NetworkOptions {
                timeout_secs: Some(15),
                ..Default::default()
            },
            NetworkOptions {
                read_timeout_secs: Some(45),
                ..Default::default()
            },
            NetworkOptions {
                pool_idle_timeout_secs: Some(30),
                ..Default::default()
            },
            NetworkOptions {
                cookies_from_browser: Some(BrowserType::Chrome),
                ..Default::default()
            },
            NetworkOptions {
                cookies_file: Some(PathBuf::from("c.txt")),
                ..Default::default()
            },
        ];
        for net in cases {
            assert!(
                request_needs_dedicated_client(&net),
                "client-level override must force a dedicated client: {net:?}"
            );
        }
    }

    #[test]
    fn network_options_fields_are_classified_for_client_sharing() {
        // Compile-time tripwire keeping `request_needs_dedicated_client` in sync
        // with the per-request override surface. `NetworkOptions` is the ONLY way
        // a download() request can diverge from the base config, so a field that
        // is not here cannot be overridden per-request. If a field is added to
        // `NetworkOptions`, this exhaustive destructure stops compiling — forcing
        // a decision: is the new field client-build-only (add it to
        // `request_needs_dedicated_client`) or registry/downloader-level (leave
        // it out)? See also the doc-note coupling the predicate to
        // `rdlp_http::HttpClientConfig::from_rdlp_config`.
        let NetworkOptions {
            // client-build-only — MUST appear in request_needs_dedicated_client:
            proxy: _,
            timeout_secs: _,
            read_timeout_secs: _,
            pool_idle_timeout_secs: _,
            cookies_from_browser: _,
            cookies_file: _,
            // registry/downloader-level — intentionally NOT in the predicate
            // (operation timeouts + downloader knobs, not baked into the client):
            retries: _,
            concurrent_fragments: _,
            rate_limit: _,
            download_timeout_secs: _,
            merge_timeout_secs: _,
        } = NetworkOptions::default();
    }

    #[test]
    fn default_requests_reuse_one_shared_client() {
        let client = crate::RdlpClient::new(rdlp_types::Config::default()).unwrap();
        // Any two distinct URLs with default network options — `shared_http_for`
        // never fetches, so the host is irrelevant; only the (empty) NetworkOptions matter.
        let a = client
            .shared_http_for(&DownloadRequest::new("https://example.com/a"))
            .expect("shared");
        let b = client
            .shared_http_for(&DownloadRequest::new("https://example.org/b"))
            .expect("shared");
        assert!(std::sync::Arc::ptr_eq(&a.client, &b.client));
        assert!(std::sync::Arc::ptr_eq(&a.client, &client.shared_client));
    }

    #[test]
    fn client_level_override_takes_dedicated_path() {
        let client = crate::RdlpClient::new(rdlp_types::Config::default()).unwrap();
        let mut req = DownloadRequest::new("https://example.com/a");
        req.network.proxy = Some("http://p:3128".into());
        assert!(client.shared_http_for(&req).is_none());
    }
}

#[cfg(test)]
mod terminal_event_tests {
    use super::*;

    /// A user cancellation during ANY phase (including post-processing/recode)
    /// MUST surface as [`Event::Cancelled`], never [`Event::Failed`] — otherwise
    /// the UI buckets a deliberate cancel as a failure. Regression guard for the
    /// recode-cancel → "Failed" bug.
    #[test]
    fn user_cancelled_maps_to_cancelled_event() {
        let id = DownloadId::next();
        let event = terminal_event(id, &RdlpApiError::UserCancelled);
        assert!(
            matches!(event, Event::Cancelled { id: eid } if eid == id),
            "UserCancelled must map to Event::Cancelled, got {event:?}",
        );
    }

    /// Every non-cancel terminal error stays a genuine [`Event::Failed`].
    #[test]
    fn non_cancel_errors_map_to_failed_event() {
        let id = DownloadId::next();
        let cases = [
            RdlpApiError::NetworkError {
                message: "boom".into(),
                status: Some(503),
            },
            RdlpApiError::IoError {
                message: "disk full".into(),
            },
            RdlpApiError::FfmpegError {
                message: "encoder error".into(),
            },
        ];
        for err in cases {
            let event = terminal_event(id, &err);
            assert!(
                matches!(event, Event::Failed { .. }),
                "non-cancel error {err:?} must map to Event::Failed, got {event:?}",
            );
        }
    }

    /// The Failed event preserves the originating error verbatim (the patch must
    /// not drop the error detail when routing through the helper).
    #[test]
    fn failed_event_preserves_error() {
        let id = DownloadId::next();
        let err = RdlpApiError::NetworkError {
            message: "stale cdn".into(),
            status: Some(403),
        };
        match terminal_event(id, &err) {
            Event::Failed {
                error: RdlpApiError::NetworkError { status, .. },
                ..
            } => assert_eq!(status, Some(403)),
            other => panic!("expected Event::Failed(NetworkError), got {other:?}"),
        }
    }
}
