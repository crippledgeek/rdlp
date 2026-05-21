//! DASH (Dynamic Adaptive Streaming over HTTP) downloader module.
//!
//! Static (VoD) MPDs only; live/DRM/multi-period beyond the first period are
//! refused at parse time. See `docs/planning/2026-05-02-dash-protocol-support-design.md`.

// `Duration::from_mins` / `from_hours` (lint's suggested replacements) need Rust 1.95;
// workspace MSRV is 1.85.
#![allow(clippy::duration_suboptimal_units)]

mod download;
pub mod errors;
pub mod manifest;
pub mod segments;
pub mod state;

pub use errors::DashError;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rdlp_core::{DownloadStats, Downloader, ProgressCallback, Result, RetryConfig};
use rdlp_types::Format;

use crate::http::HttpDownloader;

/// DASH downloader.
///
/// Resolves the highest-bandwidth video + audio reprs from the supplied
/// MPD URL, downloads segments in parallel, and muxes the result into a
/// single output container via `FFmpeg` stream-copy.
#[derive(Clone)]
pub struct DashDownloader {
    http_downloader: HttpDownloader,
    concurrent_segments: usize,
    buffer_size: usize,
    retry_config: Arc<RetryConfig>,
    #[allow(dead_code)]
    expected_size: Option<u64>,
    #[allow(dead_code)]
    download_timeout: Duration,
    #[allow(dead_code)]
    merge_timeout: Duration,
    #[allow(dead_code)]
    max_segment_failures: usize,
}

impl DashDownloader {
    /// Construct a new `DashDownloader` with default settings (8 concurrent
    /// segments, 2 MiB buffer, 1 h download timeout, 30 min mux timeout).
    #[must_use]
    pub fn new() -> Self {
        Self {
            http_downloader: HttpDownloader::new(),
            concurrent_segments: 8,
            buffer_size: 2 * 1024 * 1024,
            retry_config: Arc::new(RetryConfig::default_config()),
            expected_size: None,
            download_timeout: Duration::from_secs(3600), // 1 hour
            merge_timeout: Duration::from_secs(1800),    // 30 min
            max_segment_failures: 3,
        }
    }

    /// Replace the inner HTTP downloader (used by the registry to share a
    /// single client / cookie jar across downloaders).
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_http_downloader(mut self, http: HttpDownloader) -> Self {
        self.http_downloader = http;
        self
    }

    /// Set the number of segments fetched in parallel. Floored to 1.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_concurrent_segments(mut self, count: usize) -> Self {
        self.concurrent_segments = count.max(1);
        self
    }

    /// Set the file write buffer size (bytes).
    #[must_use = "builder methods consume self and return a new instance"]
    pub const fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Replace the retry policy used for MPD and segment fetches. Tests
    /// override this with a tight policy to avoid burning multi-minute
    /// backoff cycles on synthetic 503s.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = Arc::new(config);
        self
    }
}

impl Default for DashDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for DashDownloader {
    fn protocol(&self) -> &'static str {
        "dash"
    }

    /// Download a [`Format`] to `output`.
    ///
    /// When `format.fragments` is `Some`, the pre-resolved fragments are
    /// fetched directly without re-fetching or re-parsing the MPD. When
    /// `fragments` is `None`, the legacy MPD-URL path (`download::run`) is
    /// used unchanged.
    ///
    /// # Errors
    ///
    /// Returns `RdlpError::Download` on any I/O or HTTP failure. Security
    /// validation errors are also surfaced as `RdlpError::Download`.
    async fn download_format(
        &self,
        format: &Format,
        output: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<DownloadStats> {
        if let Some(fragments) = format.fragments.as_deref() {
            let base = format.fragment_base_url.as_deref();
            crate::fragments::download_pre_resolved_fragments(
                &self.http_downloader,
                fragments,
                base,
                format.filesize,
                progress.as_deref(),
                output,
                Some(&format.url),
                cancel,
            )
            .await
        } else {
            // F6: cooperative cancellation via HttpDownloader's cancel-aware paths.
            // The legacy MPD-URL branch delegates to the inner HTTP downloader which
            // honours `cancel` via its `download_format` override. Closes #287.
            self.http_downloader
                .download_format(format, output, progress, cancel)
                .await
        }
    }

    fn supports(&self, url: &str) -> bool {
        // Path ends with .mpd, or query string suggests MPD. Both checks
        // are case-insensitive — the URL is lowercased first.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        {
            let lower = url.to_ascii_lowercase();
            let path = lower.split('?').next().unwrap_or("");
            path.ends_with(".mpd") || lower.contains(".mpd?")
        }
    }

    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        download::run(
            &self.http_downloader,
            Arc::clone(&self.retry_config),
            self.concurrent_segments,
            self.buffer_size,
            url,
            path,
            progress,
        )
        .await
    }
}
