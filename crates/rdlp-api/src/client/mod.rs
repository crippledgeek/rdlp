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
use crate::orchestrator::{InteractiveCallback, Orchestrator};
use crate::request::DownloadRequest;
use crate::result::DownloadResult;
use log::{error, warn};
use rdlp_core::DownloadStats;
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
}

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
    pub fn temp_registry(&self) -> &Arc<TempRegistry> {
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
    pub fn download(&self, request: DownloadRequest) -> DownloadHandle {
        let id = DownloadId::next();
        let (tx, rx) = mpsc::channel::<Event>(256);
        let cancel_token = CancellationToken::new();

        let config = Arc::new(self.build_config(&request));
        let interactive_flag = request.format.interactive;
        let url = request.url.clone();
        let interactive_cb = self.interactive.clone();
        let token = cancel_token.clone();
        let registry = Arc::clone(&self.temp_registry);

        // Capture whether the user explicitly requested cookies.
        // When explicit, cookie loading failure must be fatal.
        let cookies_explicitly_requested =
            config.cookies_from_browser.is_some() || config.cookies_file.is_some();

        let join_handle = tokio::spawn(async move {
            let orchestrator = Orchestrator::new_with_registry(
                config,
                tx.clone(),
                id,
                token,
                interactive_cb,
                Some(registry),
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
                    let delay = REEXTRACT_DELAYS_SECS[(attempt - 1) as usize];
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

                match orchestrator.download(&url, interactive_flag).await {
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
                                result: Box::new(result.clone()),
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
                        // Final failure — emit and return
                        // intentional: receiver may have disconnected
                        let _ = tx
                            .send(Event::Failed {
                                id,
                                error: api_err.clone(),
                            })
                            .await;
                        return Err(api_err);
                    }
                }
            }
            // All retries exhausted (unreachable in practice, but
            // satisfies the compiler)
            let err = last_err.unwrap_or(RdlpApiError::NetworkError {
                message: "All re-extraction attempts failed".into(),
                status: None,
            });
            // intentional: receiver may have disconnected
            let _ = tx
                .send(Event::Failed {
                    id,
                    error: err.clone(),
                })
                .await;
            Err(err)
        });

        DownloadHandle::new(id, rx, cancel_token, join_handle)
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

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
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

        let orchestrator = Orchestrator::new(
            Arc::clone(&self.config),
            tx,
            id,
            cancel_token,
            self.interactive.clone(),
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
    #[must_use]
    pub fn list_extractors(&self) -> Vec<String> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
        orchestrator
            .list_extractors()
            .into_iter()
            .map(String::from)
            .collect()
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

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
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

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
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

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
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

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
        orchestrator
            .search_page(site, query)
            .await
            .map_err(Into::into)
    }

    /// Execute a search and return results as lightweight previews.
    ///
    /// Alias for [`search()`](Self::search) — only scrapes search result
    /// pages, does not fetch individual video pages.
    pub async fn search_preview(
        &self,
        site: &str,
        query: &rdlp_types::SearchQuery,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>, RdlpApiError> {
        self.search(site, query).await
    }

    /// List available download protocols.
    #[must_use]
    pub fn list_downloaders(&self) -> Vec<String> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();

        let orchestrator = Orchestrator::new(Arc::clone(&self.config), tx, id, cancel_token, None);
        orchestrator
            .list_downloaders()
            .into_iter()
            .map(String::from)
            .collect()
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
                .run_postprocessing(&info, vec![path], false)
                .await
            {
                Ok(output_files) => {
                    let result = crate::DownloadResult {
                        id,
                        output_files,
                        info,
                        stats: rdlp_core::DownloadStats::new(0, std::time::Duration::ZERO, 0),
                    };
                    // intentional: receiver may have disconnected
                    let _ = tx
                        .send(Event::Completed {
                            id,
                            result: Box::new(result.clone()),
                        })
                        .await;
                    Ok(result)
                }
                Err(e) => {
                    let err = RdlpApiError::from(e);
                    // intentional: receiver may have disconnected
                    let _ = tx
                        .send(Event::Failed {
                            id,
                            error: err.clone(),
                        })
                        .await;
                    Err(err)
                }
            }
        });

        DownloadHandle::new(id, rx, cancel_token, join_handle)
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
    pub fn interactive(mut self, cb: Arc<dyn InteractiveCallback>) -> Self {
        self.interactive = Some(cb);
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

        Ok(RdlpClient {
            config: Arc::new(config),
            interactive: self.interactive,
            temp_registry,
        })
    }
}

impl Default for RdlpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}
