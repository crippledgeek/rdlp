//! DASH (Dynamic Adaptive Streaming over HTTP) downloader module.
//!
//! Static (VoD) MPDs only; live/DRM/multi-period beyond the first period are
//! refused at parse time. See `docs/planning/2026-05-02-dash-protocol-support-design.md`.

// `Duration::from_mins` / `from_hours` (lint's suggested replacements) need Rust 1.95;
// workspace MSRV is 1.85.
#![allow(clippy::duration_suboptimal_units)]

pub mod errors;
pub mod manifest;
pub mod segments;
mod download;
mod state;

pub use errors::DashError;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rdlp_core::{Downloader, DownloadStats, ProgressCallback, Result, RetryConfig};

use crate::http::HttpDownloader;

/// DASH downloader.
///
/// Resolves the highest-bandwidth video + audio reprs from the supplied
/// MPD URL, downloads segments in parallel, and muxes the result into a
/// single output container via `FFmpeg` stream-copy.
#[derive(Clone)]
pub struct DashDownloader {
    #[allow(dead_code)]
    http_downloader: HttpDownloader,
    #[allow(dead_code)]
    concurrent_segments: usize,
    #[allow(dead_code)]
    buffer_size: usize,
    #[allow(dead_code)]
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
        _url: &str,
        _path: &Path,
        _progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // Implemented in Task 5 (download.rs::run).
        Err(rdlp_core::RdlpError::Download {
            message: "DashDownloader::download_to_file not yet implemented".to_string(),
            url: None,
        })
    }

    async fn get_size(&self, _url: &str) -> Result<Option<u64>> {
        Ok(None)
    }
}
