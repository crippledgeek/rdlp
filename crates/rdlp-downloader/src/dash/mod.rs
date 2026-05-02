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
use rdlp_types::{Format, Fragment};

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

impl DashDownloader {
    /// Download a `Format` to `output`.
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
    pub async fn download_format(
        &self,
        format: &Format,
        output: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        if let Some(fragments) = format.fragments.as_deref() {
            let base = format.fragment_base_url.as_deref();
            self.download_pre_resolved_fragments(fragments, base, output)
                .await
        } else {
            self.download_to_file(&format.url, output, progress).await
        }
    }

    /// Fetch a pre-resolved list of fragment URLs and concatenate them into
    /// `output` in order.
    ///
    /// Each URL is security-validated before being fetched. Fragments are
    /// written sequentially — no intermediate files or `FFmpeg` mux step is
    /// required because the extractor already resolved both streams into
    /// separate `Format` entries.
    ///
    /// # Errors
    ///
    /// Returns `RdlpError::Download` if any fragment fetch fails, if a URL
    /// fails security validation, or if the output file cannot be created.
    async fn download_pre_resolved_fragments(
        &self,
        fragments: &[Fragment],
        base_url: Option<&str>,
        output: &Path,
    ) -> Result<DownloadStats> {
        use std::time::Instant;
        use tokio::io::AsyncWriteExt as _;

        let started = Instant::now();

        let mut out_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output)
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("create output: {e}"),
                url: Some(output.display().to_string()),
            })?;

        let mut total_bytes: u64 = 0;

        for frag in fragments {
            let resolved_url = resolve_fragment_url(&frag.url, base_url)?;
            let bytes = fetch_fragment_bytes(&self.http_downloader, &resolved_url).await?;

            out_file
                .write_all(&bytes)
                .await
                .map_err(|e| rdlp_core::RdlpError::Download {
                    message: format!("write fragment: {e}"),
                    url: Some(output.display().to_string()),
                })?;

            total_bytes += bytes.len() as u64;
        }

        out_file
            .flush()
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("flush output: {e}"),
                url: Some(output.display().to_string()),
            })?;

        let elapsed = started.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let avg = if elapsed.as_secs_f64() > 0.0 {
            total_bytes as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        Ok(DownloadStats {
            bytes_downloaded: total_bytes,
            duration: elapsed,
            average_speed: avg,
            retries: 0,
            fragments: Some(fragments.len()),
        })
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

    async fn get_size(&self, _url: &str) -> Result<Option<u64>> {
        Ok(None)
    }
}

/// Resolve a fragment URL against an optional base URL.
///
/// When `base_url` is `Some`, the fragment URL is joined against it (handles
/// relative paths). When `base_url` is `None`, the fragment URL is used as-is
/// (it must be absolute).
fn resolve_fragment_url(fragment_url: &str, base_url: Option<&str>) -> Result<String> {
    match base_url {
        Some(base) => {
            let base_parsed =
                url::Url::parse(base).map_err(|e| rdlp_core::RdlpError::Download {
                    message: format!("invalid fragment_base_url: {e}"),
                    url: Some(base.to_string()),
                })?;
            let resolved =
                base_parsed
                    .join(fragment_url)
                    .map_err(|e| rdlp_core::RdlpError::Download {
                        message: format!("resolve fragment url: {e}"),
                        url: Some(fragment_url.to_string()),
                    })?;
            Ok(resolved.to_string())
        }
        None => Ok(fragment_url.to_string()),
    }
}

/// Fetch a single fragment URL and return its body as bytes.
async fn fetch_fragment_bytes(http: &HttpDownloader, url: &str) -> Result<Vec<u8>> {
    use std::time::Duration;

    let client = http.client().clone();
    let headers = http.headers();
    let resp = client
        .get(url)
        .headers(headers)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| rdlp_core::RdlpError::Network {
            message: format!("fragment fetch failed: {e}"),
            url: Some(url.to_string()),
        })?;
    if !resp.status().is_success() {
        return Err(rdlp_core::RdlpError::Http {
            status: resp.status().as_u16(),
            reason: format!("fragment HTTP {}", resp.status()),
        });
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| rdlp_core::RdlpError::Network {
            message: format!("fragment read error: {e}"),
            url: Some(url.to_string()),
        })?;
    Ok(bytes.to_vec())
}
