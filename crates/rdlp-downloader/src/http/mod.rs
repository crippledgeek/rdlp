//! HTTP/HTTPS downloader implementation
//!
//! Provides HTTP downloading with parallel chunk support, resume capability,
//! and automatic retry logic using the backon crate.

mod config;
mod parallel;
mod trait_impl;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use parallel::download_chunk_with_retry;

use backon::Retryable;
use futures::StreamExt;
use log::warn;
use rdlp_core::{
    DownloadProgress, DownloadStats, ProgressCallback, RdlpError, Result, RetryConfig,
    check_http_response, is_retryable_error,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use wreq::header::{HeaderMap, HeaderName, HeaderValue};

use crate::chunking::ChunkSizeStrategy;
use config::{DownloaderConfig, PROGRESS_UPDATE_INTERVAL};
use rdlp_ratelimit::RateLimiter;

/// Convert optional `HashMap` headers to wreq `HeaderMap`
fn to_header_map(headers: Option<&HashMap<String, String>>) -> HeaderMap {
    let Some(headers) = headers else {
        return HeaderMap::new();
    };
    let mut map = HeaderMap::new();
    for (key, value) in headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.insert(name, val);
        }
    }
    map
}

/// Execute an async operation with retry logic
async fn with_retry<F, Fut, T>(
    retry_config: &RetryConfig,
    context: &'static str,
    operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let backoff = retry_config.to_backoff();
    operation
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!(delay:? = dur; "{context} failed, retrying: {err}");
        })
        .await
}

/// Parse the `Content-Range` header's total-bytes field.
///
/// Accepts `bytes 0-N/TOTAL` (returns `Some(TOTAL)`) and returns `None` for
/// `bytes 0-N/*` (server signalled unknown total per RFC 7233 §4.2), any
/// missing header, or any unparseable total.
pub(crate) fn parse_content_range_total(headers: &wreq::header::HeaderMap) -> Option<u64> {
    headers
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split('/').nth(1))
        .and_then(|s| s.parse::<u64>().ok())
}

/// HTTP/HTTPS downloader
///
/// **Clone performance:** O(1) - both client and config use Arc internally
#[derive(Clone)]
pub struct HttpDownloader {
    client: wreq::Client,
    pub(crate) config: Arc<DownloaderConfig>,
    pub(crate) rate_limiter: Option<Arc<RateLimiter>>,
    extra_headers: HeaderMap,
}

impl HttpDownloader {
    /// Create a new HTTP downloader
    #[must_use]
    pub fn new() -> Self {
        // Route through HttpClientFactory so the default browser emulation
        // profile (ChromeLatest) is applied — otherwise this constructor
        // would hand back a wreq client with no JA4 / JA4H emulation,
        // bypassing the Phase 2 fingerprint guarantee (spec §6.8).
        let client =
            rdlp_http::HttpClientFactory::from_config(&rdlp_http::HttpClientConfig::default())
                .build();
        Self::with_client(client)
    }

    /// Create with custom client
    #[must_use]
    pub fn with_client(client: wreq::Client) -> Self {
        Self {
            client,
            config: Arc::new(DownloaderConfig::default()),
            rate_limiter: None,
            extra_headers: HeaderMap::new(),
        }
    }

    /// Get reference to the HTTP client
    #[must_use]
    pub const fn client(&self) -> &wreq::Client {
        &self.client
    }

    /// Set buffer size for downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        Arc::make_mut(&mut self.config).buffer_size = size;
        self
    }

    /// Set retry configuration
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        Arc::make_mut(&mut self.config).retry_config = config;
        self
    }

    /// Set number of concurrent fragment downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_concurrent_fragments(mut self, count: usize) -> Self {
        Arc::make_mut(&mut self.config).concurrent_fragments = count.max(1);
        self
    }

    /// Return the configured concurrent-fragments limit.
    ///
    /// Used by `download_pre_resolved_fragments` to size the `buffered(N)` parallel
    /// fetch stream and to set `AdaptiveConfig::max_connections`.
    #[must_use]
    pub fn concurrent_fragments(&self) -> usize {
        self.config.concurrent_fragments
    }

    /// Set the minimum file size in bytes at which the downloader switches
    /// to parallel chunked mode. Below this, sequential I/O is used.
    /// Default: `DEFAULT_PARALLEL_THRESHOLD_BYTES` (10 MiB).
    ///
    /// `bytes` is clamped to a floor of 1 to mirror the `Config::validate()`
    /// lower bound and prevent threshold = 0 from amplifying HEAD-probe
    /// traffic on every download.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_parallel_threshold(mut self, bytes: u64) -> Self {
        Arc::make_mut(&mut self.config).parallel_threshold = bytes.max(1);
        self
    }

    /// Set chunk size strategy.
    ///
    /// When `Fixed` or `Legacy` is used, adaptive mode is forced off because
    /// the caller has explicitly chosen a predictable chunk sizing scheme.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_chunk_strategy(mut self, strategy: ChunkSizeStrategy) -> Self {
        let cfg = Arc::make_mut(&mut self.config);
        if !matches!(strategy, ChunkSizeStrategy::Auto) {
            cfg.adaptive = false;
        }
        cfg.chunk_strategy = strategy;
        self
    }

    /// Enable or disable adaptive chunk sizing and connection tuning.
    ///
    /// When `false`, the downloader uses the static `chunk_strategy` with a
    /// fixed connection count. Automatically forced to `false` when
    /// `chunk_strategy` is not `Auto`.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_adaptive(mut self, adaptive: bool) -> Self {
        Arc::make_mut(&mut self.config).adaptive = adaptive;
        self
    }

    /// Set per-read idle timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        Arc::make_mut(&mut self.config).read_timeout = timeout;
        self
    }

    /// Set total download timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_download_timeout(mut self, timeout: Duration) -> Self {
        Arc::make_mut(&mut self.config).download_timeout = timeout;
        self
    }

    /// Set merge operation timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_merge_timeout(mut self, timeout: Duration) -> Self {
        Arc::make_mut(&mut self.config).merge_timeout = timeout;
        self
    }

    /// Set the rate limiter for bandwidth throttling
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_rate_limiter(mut self, limiter: Option<Arc<RateLimiter>>) -> Self {
        self.rate_limiter = limiter;
        self
    }

    /// Set extra HTTP headers sent with every download request (e.g. Referer for CDN auth)
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_extra_headers(mut self, headers: Option<&HashMap<String, String>>) -> Self {
        self.extra_headers = to_header_map(headers);
        self
    }

    /// Get a clone of extra headers for use in closures
    #[must_use]
    pub fn headers(&self) -> HeaderMap {
        self.extra_headers.clone()
    }

    /// Check if server supports range requests
    async fn supports_ranges(&self, url: &str) -> Result<bool> {
        let client = self.client.clone();
        let url = url.to_string();
        let hdrs = self.headers();

        let response = with_retry(&self.config.retry_config, "HTTP HEAD (range check)", || {
            let client = client.clone();
            let url = url.clone();
            let hdrs = hdrs.clone();
            async move {
                client
                    .head(&url)
                    .headers(hdrs)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network {
                        message: format!("HEAD request failed: {e}"),
                        url: Some(url.clone()),
                    })
            }
        })
        .await?;

        Ok(response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != "none"))
    }

    /// Download a specific byte range with shared progress tracking
    pub(crate) async fn download_range_with_progress(
        &self,
        url: &str,
        start: u64,
        end: u64,
        chunk_path: &Path,
        progress_counter: Option<Arc<std::sync::atomic::AtomicU64>>,
    ) -> Result<u64> {
        let client = self.client.clone();
        let url = url.to_string();
        let hdrs = self.headers();

        let response = with_retry(&self.config.retry_config, "HTTP GET (range)", || {
            let client = client.clone();
            let url = url.clone();
            let hdrs = hdrs.clone();
            async move {
                let response = client
                    .get(&url)
                    .headers(hdrs)
                    .header("Range", format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network {
                        message: format!("Range request failed: {e}"),
                        url: Some(url.clone()),
                    })?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .await?;

        let file = File::create(chunk_path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to create chunk file '{}': {e}",
                    chunk_path.display()
                ),
            ))
        })?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let read_timeout = self.config.read_timeout;

        while let Some(chunk_result) = tokio::time::timeout(read_timeout, stream.next())
            .await
            .map_err(|_| RdlpError::Network {
                message: format!("Read timed out (no data for {}s)", read_timeout.as_secs()),
                url: Some(url.clone()),
            })?
        {
            let chunk = chunk_result.map_err(|e| RdlpError::Network {
                message: format!("Failed to read chunk body from {url}: {e}"),
                url: Some(url.clone()),
            })?;

            writer.write_all(&chunk).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to write to chunk file '{}': {e}",
                        chunk_path.display()
                    ),
                ))
            })?;
            downloaded += chunk.len() as u64;

            if let Some(ref counter) = progress_counter {
                counter.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }

            if let Some(ref limiter) = self.rate_limiter {
                limiter.acquire(chunk.len()).await;
            }
        }

        writer.flush().await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to flush chunk file '{}': {e}", chunk_path.display()),
            ))
        })?;
        Ok(downloaded)
    }

    /// Sequential download (original method)
    async fn download_sequential(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let progress: Option<Arc<dyn ProgressCallback>> = progress.map(Arc::from);
        let start_time = Instant::now();
        let client = self.client.clone();
        let url_string: Arc<str> = Arc::from(url);
        let hdrs = self.headers();

        let response = with_retry(&self.config.retry_config, "HTTP GET", || {
            let client = client.clone();
            let url = url_string.clone();
            let hdrs = hdrs.clone();
            async move {
                let response = client.get(&*url).headers(hdrs).send().await.map_err(|e| {
                    RdlpError::Network {
                        message: format!("GET request failed: {e}"),
                        url: Some(url.to_string()),
                    }
                })?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .await?;

        let total_size = response.content_length();
        let file = File::create(path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to create output file '{}': {e}", path.display()),
            ))
        })?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_update = Instant::now();
        let update_interval = PROGRESS_UPDATE_INTERVAL;
        let read_timeout = self.config.read_timeout;

        while let Some(chunk_result) = tokio::time::timeout(read_timeout, stream.next())
            .await
            .map_err(|_| RdlpError::Network {
                message: format!("Read timed out (no data for {}s)", read_timeout.as_secs()),
                url: Some(url_string.to_string()),
            })?
        {
            let chunk = chunk_result.map_err(|e| RdlpError::Network {
                message: format!("Failed to read response body from {url_string}: {e}"),
                url: Some(url_string.to_string()),
            })?;

            writer.write_all(&chunk).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!("failed to write to output file '{}': {e}", path.display()),
                ))
            })?;
            downloaded += chunk.len() as u64;

            if let Some(ref callback) = progress {
                let now = Instant::now();
                if now.duration_since(last_update) >= update_interval {
                    let elapsed = now.duration_since(start_time).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        downloaded as f64 / elapsed
                    } else {
                        0.0
                    };

                    let progress_info = DownloadProgress::new(downloaded, total_size, speed);
                    callback.on_progress(&progress_info);
                    last_update = now;
                }
            }

            if let Some(ref limiter) = self.rate_limiter {
                limiter.acquire(chunk.len()).await;
            }
        }

        writer.flush().await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to flush output file '{}': {e}", path.display()),
            ))
        })?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(downloaded, duration, 0);

        if let Some(callback) = progress {
            callback.on_complete(&stats);
        }

        Ok(stats)
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod content_range_tests {
    use super::*;
    use wreq::header::HeaderMap;

    fn make_headers(content_range: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(cr) = content_range {
            h.insert("content-range", cr.parse().unwrap());
        }
        h
    }

    #[test]
    fn parses_total_from_content_range_206() {
        let h = make_headers(Some("bytes 0-262143/1048576"));
        assert_eq!(parse_content_range_total(&h), Some(1048576));
    }

    #[test]
    fn returns_none_when_header_missing() {
        let h = make_headers(None);
        assert_eq!(parse_content_range_total(&h), None);
    }

    #[test]
    fn returns_none_when_header_malformed_no_slash() {
        let h = make_headers(Some("bytes 0-262143"));
        assert_eq!(parse_content_range_total(&h), None);
    }

    #[test]
    fn returns_none_when_total_is_star() {
        let h = make_headers(Some("bytes 0-262143/*"));
        assert_eq!(parse_content_range_total(&h), None);
    }

    #[test]
    fn returns_none_when_total_unparseable() {
        let h = make_headers(Some("bytes 0-262143/notanumber"));
        assert_eq!(parse_content_range_total(&h), None);
    }
}
