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
//! use rdlp_core::Config;
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
use crate::orchestrator::{InteractiveCallback, Orchestrator};
use crate::request::DownloadRequest;
use crate::result::DownloadResult;
use rdlp_core::{Config, DownloadStats, InfoDict};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Primary entry point for the rdlp download engine.
///
/// Created via [`RdlpClient::builder()`] or [`RdlpClient::new()`].
/// Thread-safe and cheaply cloneable — all internal state is `Arc`-wrapped.
#[derive(Clone)]
pub struct RdlpClient {
    config: Arc<Config>,
    interactive: Option<Arc<dyn InteractiveCallback>>,
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

        let config = self.build_config(&request);
        let interactive_flag = request.format.interactive;
        let url = request.url.clone();
        let interactive_cb = self.interactive.clone();
        let token = cancel_token.clone();

        let join_handle = tokio::spawn(async move {
            let orchestrator = Orchestrator::new(config, tx.clone(), id, token, interactive_cb);

            // Load cookies (non-fatal on failure — warn but continue)
            if let Err(e) = orchestrator.load_cookies().await {
                let _ = tx
                    .send(Event::Warning {
                        id,
                        message: format!("Cookie loading failed: {e}"),
                    })
                    .await;
            }

            match orchestrator.download(&url, interactive_flag).await {
                Ok(Some(path)) => {
                    let result = DownloadResult {
                        id,
                        output_files: vec![path],
                        info: InfoDict::new("", "", "", &url),
                        stats: DownloadStats::new(0, Duration::ZERO, 0),
                    };
                    let _ = tx
                        .send(Event::Completed {
                            id,
                            result: Box::new(result.clone()),
                        })
                        .await;
                    Ok(result)
                }
                Ok(None) => {
                    // Already emitted Cancelled from the state machine
                    Err(RdlpApiError::UserCancelled)
                }
                Err(e) => {
                    let api_err = RdlpApiError::from(e);
                    let _ = tx
                        .send(Event::Failed {
                            id,
                            error: api_err.clone(),
                        })
                        .await;
                    Err(api_err)
                }
            }
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
        let config = (*self.config).clone();

        let orchestrator = Orchestrator::new(config, tx, id, cancel_token, None);
        orchestrator
            .load_cookies()
            .await
            .map_err(RdlpApiError::from)?;
        orchestrator.extract_info(url).await.map_err(Into::into)
    }

    /// List available extractors.
    #[must_use]
    pub fn list_extractors(&self) -> Vec<String> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();
        let config = (*self.config).clone();

        let orchestrator = Orchestrator::new(config, tx, id, cancel_token, None);
        orchestrator
            .list_extractors()
            .into_iter()
            .map(String::from)
            .collect()
    }

    /// List available download protocols.
    #[must_use]
    pub fn list_downloaders(&self) -> Vec<String> {
        let id = DownloadId::next();
        let (tx, _rx) = mpsc::channel::<Event>(1);
        let cancel_token = CancellationToken::new();
        let config = (*self.config).clone();

        let orchestrator = Orchestrator::new(config, tx, id, cancel_token, None);
        orchestrator
            .list_downloaders()
            .into_iter()
            .map(String::from)
            .collect()
    }

    /// Merge request options into a Config, applying overrides from the request.
    fn build_config(&self, request: &DownloadRequest) -> Config {
        let mut config = (*self.config).clone();

        // Output options
        if let Some(ref dir) = request.output.output_dir {
            config.output_directory = dir.clone();
        }
        if let Some(ref template) = request.output.template {
            config.output_template = template.clone();
        }

        // Format options
        if let Some(ref selector) = request.format.selector {
            config.format = selector.clone();
        }

        // Subtitle options
        config.write_subtitles = request.subtitles.write_subs;
        config.write_auto_subtitles = request.subtitles.write_auto_subs;
        if !request.subtitles.sub_langs.is_empty() {
            config.subtitle_langs = request.subtitles.sub_langs.clone();
        }
        config.subtitle_format = request.subtitles.sub_format;
        config.embed_subtitles = request.subtitles.embed_subs;
        config.strict_subs = request.subtitles.strict_subs;

        // Post-processing options
        config.remux_container = request.postprocess.remux;
        if let Some(audio_fmt) = request.postprocess.extract_audio {
            config.extract_audio = true;
            config.audio_format = Some(audio_fmt);
        }
        config.embed_metadata = request.postprocess.embed_metadata;
        config.embed_thumbnail = request.postprocess.embed_thumbnail;
        config.write_thumbnail = request.postprocess.write_thumbnail;
        config.normalize_audio = request.postprocess.normalize_audio;
        config.loudnorm = request.postprocess.loudnorm;
        if let Some(ref preset) = request.postprocess.loudnorm_preset {
            config.loudnorm_preset = Some(preset.clone());
        }

        // Network options
        config.retries = request.network.retries as usize;
        if let Some(timeout) = Some(request.network.timeout_secs) {
            config.socket_timeout = Some(timeout);
        }
        config.concurrent_fragments = request.network.concurrent_fragments as usize;
        config.rate_limit = request.network.rate_limit;
        config.cookies_from_browser = request.network.cookies_from_browser;
        if let Some(ref path) = request.network.cookies_file {
            config.cookies_file = Some(path.clone());
        }

        config
    }
}

/// Builder for [`RdlpClient`].
pub struct RdlpClientBuilder {
    config: Option<Config>,
    interactive: Option<Arc<dyn InteractiveCallback>>,
}

impl RdlpClientBuilder {
    /// Create a new builder with no defaults set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            interactive: None,
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

    /// Build the client.
    ///
    /// # Errors
    ///
    /// Returns [`RdlpApiError::BuilderError`] if `config` was not provided.
    pub fn build(self) -> Result<RdlpClient, RdlpApiError> {
        let config = self.config.ok_or_else(|| RdlpApiError::BuilderError {
            message: "config is required".into(),
        })?;

        Ok(RdlpClient {
            config: Arc::new(config),
            interactive: self.interactive,
        })
    }
}

impl Default for RdlpClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_requires_config() {
        let result = RdlpClient::builder().build();
        assert!(result.is_err());
        match result.unwrap_err() {
            RdlpApiError::BuilderError { message } => {
                assert!(message.contains("config"));
            }
            other => panic!("Expected BuilderError, got: {other:?}"),
        }
    }

    #[test]
    fn test_new_with_default_config() {
        let result = RdlpClient::new(Config::default());
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_is_clone() {
        let client = RdlpClient::new(Config::default()).unwrap();
        let _clone = client.clone();
    }

    #[test]
    fn test_builder_default() {
        let builder = RdlpClientBuilder::default();
        assert!(builder.config.is_none());
        assert!(builder.interactive.is_none());
    }

    #[tokio::test]
    async fn test_download_returns_handle() {
        let client = RdlpClient::new(Config::default()).unwrap();
        let handle = client.download(DownloadRequest::new("http://example.com"));
        assert!(handle.id().as_u64() > 0);
        // Cancel immediately so the task doesn't attempt a real download
        handle.cancel();
    }

    #[test]
    fn test_build_config_applies_overrides() {
        let client = RdlpClient::new(Config::default()).unwrap();
        let request = DownloadRequest {
            url: "http://example.com".into(),
            output: crate::request::OutputOptions {
                output_dir: Some(std::path::PathBuf::from("/tmp/test")),
                template: Some("%(title)s.%(ext)s".into()),
                ..Default::default()
            },
            format: crate::request::FormatOptions {
                selector: Some("best".into()),
                ..Default::default()
            },
            network: crate::request::NetworkOptions {
                retries: 7,
                concurrent_fragments: 8,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = client.build_config(&request);
        assert_eq!(
            config.output_directory,
            std::path::PathBuf::from("/tmp/test")
        );
        assert_eq!(config.output_template, "%(title)s.%(ext)s");
        assert_eq!(config.format, "best");
        assert_eq!(config.retries, 7);
        assert_eq!(config.concurrent_fragments, 8);
    }

    #[test]
    fn test_build_config_preserves_base_when_no_override() {
        let mut base_config = Config::default();
        base_config.verbose = true;
        base_config.overwrite = true;

        let client = RdlpClient::builder().config(base_config).build().unwrap();
        let request = DownloadRequest::new("http://example.com");
        let config = client.build_config(&request);

        assert!(config.verbose);
        assert!(config.overwrite);
    }

    #[test]
    fn test_list_extractors() {
        let client = RdlpClient::new(Config::default()).unwrap();
        let extractors = client.list_extractors();
        assert!(!extractors.is_empty());
    }

    #[test]
    fn test_list_downloaders() {
        let client = RdlpClient::new(Config::default()).unwrap();
        let downloaders = client.list_downloaders();
        assert!(!downloaders.is_empty());
    }
}
