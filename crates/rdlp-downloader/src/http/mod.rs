//! HTTP/HTTPS downloader implementation
//!
//! Provides HTTP downloading with parallel chunk support, resume capability,
//! and automatic retry logic using the backon crate.

mod config;
mod parallel;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use backon::Retryable;
use futures::StreamExt;
use log::{debug, info, warn};
use rdlp_core::{
    DownloadProgress, DownloadStats, Downloader, ProgressCallback, RdlpError, Result, RetryConfig,
    check_http_response, is_retryable_error,
};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::chunking::ChunkSizeStrategy;
use config::DownloaderConfig;

/// HTTP/HTTPS downloader
///
/// **Clone performance:** O(1) - both client and config use Arc internally
#[derive(Clone)]
pub struct HttpDownloader {
    client: reqwest::Client,
    pub(crate) config: Arc<DownloaderConfig>,
}

impl HttpDownloader {
    /// Create a new HTTP downloader
    pub fn new() -> Self {
        Self::with_client(reqwest::Client::new())
    }

    /// Create with custom client
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            config: Arc::new(DownloaderConfig::default()),
        }
    }

    /// Get reference to the HTTP client
    pub fn client(&self) -> &reqwest::Client {
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

    /// Set chunk size strategy
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_chunk_strategy(mut self, strategy: ChunkSizeStrategy) -> Self {
        Arc::make_mut(&mut self.config).chunk_strategy = strategy;
        self
    }

    /// Check if server supports range requests
    async fn supports_ranges(&self, url: &str) -> Result<bool> {
        let client = self.client.clone();
        let url = url.to_string();
        let backoff = self.config.retry_config.to_backoff();

        let response = (|| {
            let client = client.clone();
            let url = url.clone();
            async move {
                client
                    .head(&url)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("HEAD request failed: {e}")))
            }
        })
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!("HTTP HEAD (range check) failed, retrying in {dur:?}: {err}");
        })
        .await?;

        Ok(response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .map(|v| v != "none")
            .unwrap_or(false))
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
        let backoff = self.config.retry_config.to_backoff();

        let response = (|| {
            let client = client.clone();
            let url = url.clone();
            async move {
                let response = client
                    .get(&url)
                    .header("Range", format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Range request failed: {e}")))?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!("HTTP GET (range) failed, retrying in {dur:?}: {err}");
        })
        .await?;

        let file = File::create(chunk_path).await.map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk).await.map_err(RdlpError::Io)?;
            downloaded += chunk.len() as u64;

            if let Some(ref counter) = progress_counter {
                counter.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }
        }

        writer.flush().await.map_err(RdlpError::Io)?;
        Ok(downloaded)
    }

    /// Sequential download (original method)
    async fn download_sequential(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();
        let client = self.client.clone();
        let url_string = url.to_string();
        let backoff = self.config.retry_config.to_backoff();

        let response = (|| {
            let client = client.clone();
            let url = url_string.clone();
            async move {
                let response = client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("GET request failed: {e}")))?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!("HTTP GET failed, retrying in {dur:?}: {err}");
        })
        .await?;

        let total_size = response.content_length();
        let file = File::create(path).await.map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_update = Instant::now();
        let update_interval = Duration::from_millis(100);

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk).await.map_err(RdlpError::Io)?;
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
        }

        writer.flush().await.map_err(RdlpError::Io)?;

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

#[async_trait]
impl Downloader for HttpDownloader {
    fn protocol(&self) -> &str {
        "http"
    }

    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let size = self.get_size(url).await.ok().flatten();
        let supports_ranges = self.supports_ranges(url).await.unwrap_or(false);

        debug!(
            "Download analysis: size={} MB, concurrent={}, ranges={}",
            size.map(|s| s / 1024 / 1024).unwrap_or(0),
            self.config.concurrent_fragments,
            supports_ranges
        );

        // Try Range request if HEAD didn't return size
        let size = if (size.is_none() || size == Some(0)) && supports_ranges {
            debug!("HEAD didn't return valid size, trying Range request...");
            let client = self.client.clone();
            let url_string = url.to_string();
            let backoff = self.config.retry_config.to_backoff();

            match (|| {
                let client = client.clone();
                let url = url_string.clone();
                async move {
                    client
                        .get(&url)
                        .header("Range", "bytes=0-0")
                        .send()
                        .await
                        .map_err(|e| RdlpError::Network(format!("Size check failed: {e}")))
                }
            })
            .retry(backoff)
            .when(is_retryable_error)
            .notify(|err, dur| {
                warn!("HTTP GET (size check) failed, retrying in {dur:?}: {err}");
            })
            .await
            {
                Ok(response) => {
                    if let Some(content_range) = response.headers().get("content-range") {
                        if let Ok(range_str) = content_range.to_str() {
                            if let Some(total_str) = range_str.split('/').nth(1) {
                                let detected_size = total_str.parse::<u64>().ok();
                                debug!(
                                    "Detected size from Range: {} MB",
                                    detected_size.map(|s| s / 1024 / 1024).unwrap_or(0)
                                );
                                detected_size
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            size
        };

        let use_parallel = match size {
            Some(s) if s > 10 * 1024 * 1024 => {
                self.config.concurrent_fragments > 1 && supports_ranges
            }
            _ => false,
        };

        if use_parallel {
            info!(
                "Using parallel download mode ({} connections)",
                self.config.concurrent_fragments
            );
            return self
                .download_parallel(url, path, size.unwrap(), progress)
                .await;
        } else {
            let reason = match size {
                None | Some(0) => "could not detect file size",
                Some(s) if s <= 10 * 1024 * 1024 => "file too small for parallel",
                Some(_) if self.config.concurrent_fragments <= 1 => "concurrent_fragments <= 1",
                Some(_) if !supports_ranges => "server doesn't support ranges",
                Some(_) => "unknown reason",
            };
            warn!(
                "Using sequential download - reason: {reason} (size: {:?} MB, fragments: {}, ranges: {supports_ranges})",
                size.map(|s| s / 1024 / 1024),
                self.config.concurrent_fragments
            );
        }

        self.download_sequential(url, path, progress).await
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn get_size(&self, url: &str) -> Result<Option<u64>> {
        let client = self.client.clone();
        let url = url.to_string();
        let backoff = self.config.retry_config.to_backoff();

        let response = (|| {
            let client = client.clone();
            let url = url.clone();
            async move {
                client
                    .head(&url)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("HEAD request failed: {e}")))
            }
        })
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!("HTTP HEAD failed, retrying in {dur:?}: {err}");
        })
        .await?;

        Ok(response.content_length())
    }

    async fn download_with_resume(
        &self,
        url: &str,
        path: &Path,
        resume_from: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();
        let client = self.client.clone();
        let url_string = url.to_string();
        let backoff = self.config.retry_config.to_backoff();

        let response = (|| {
            let client = client.clone();
            let url = url_string.clone();
            async move {
                let response = client
                    .get(&url)
                    .header("Range", format!("bytes={resume_from}-"))
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Resume request failed: {e}")))?;

                if response.status().as_u16() != 206 {
                    return Err(RdlpError::Download(format!(
                        "Server does not support resume (expected HTTP 206, got {}). \
                         Cannot continue download without overwriting existing data. \
                         Please delete the partial file and restart the download.",
                        response.status()
                    )));
                }

                Ok(response)
            }
        })
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!("HTTP GET (resume) failed, retrying in {dur:?}: {err}");
        })
        .await?;

        let total_size = if let Some(content_range) = response.headers().get("content-range") {
            if let Ok(range_str) = content_range.to_str() {
                if let Some(total_str) = range_str.split('/').nth(1) {
                    total_str.parse::<u64>().ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            response.content_length().map(|size| size + resume_from)
        };

        // Check for parallel resume
        if let Some(total) = total_size {
            let progress_pct = (resume_from as f64 / total as f64) * 100.0;
            let remaining_size = total - resume_from;
            let supports_ranges = response
                .headers()
                .get("accept-ranges")
                .and_then(|v| v.to_str().ok())
                .map(|v| v != "none")
                .unwrap_or(true);

            debug!(
                "Resume analysis: {:.1}% ({} MB / {} MB), remaining={} MB, concurrent={}, ranges={}",
                progress_pct,
                resume_from / 1024 / 1024,
                total / 1024 / 1024,
                remaining_size / 1024 / 1024,
                self.config.concurrent_fragments,
                supports_ranges
            );

            let can_parallel = remaining_size > 10 * 1024 * 1024
                && self.config.concurrent_fragments > 1
                && supports_ranges;

            if can_parallel {
                info!(
                    "Using parallel resume mode ({} connections), keeping {} MB, parallelizing {} MB",
                    self.config.concurrent_fragments,
                    resume_from / 1024 / 1024,
                    remaining_size / 1024 / 1024
                );

                drop(response);
                return self
                    .download_parallel_resume(url, path, resume_from, total, progress)
                    .await;
            } else if !can_parallel {
                warn!(
                    "Parallel resume not available (remaining: {} MB, concurrent: {}, ranges: {}), using sequential",
                    remaining_size / 1024 / 1024,
                    self.config.concurrent_fragments,
                    supports_ranges
                );
            }
        }

        let content_length = response.content_length();
        let total_size = total_size.or_else(|| content_length.map(|size| size + resume_from));

        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded = resume_from;
        let mut last_update = Instant::now();
        let update_interval = Duration::from_millis(100);

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk).await.map_err(RdlpError::Io)?;
            downloaded += chunk.len() as u64;

            if let Some(ref callback) = progress {
                let now = Instant::now();
                if now.duration_since(last_update) >= update_interval {
                    let elapsed = now.duration_since(start_time).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        (downloaded - resume_from) as f64 / elapsed
                    } else {
                        0.0
                    };

                    let progress_info = DownloadProgress::new(downloaded, total_size, speed);
                    callback.on_progress(&progress_info);
                    last_update = now;
                }
            }
        }

        writer.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(downloaded, duration, 0);

        if let Some(callback) = progress {
            callback.on_complete(&stats);
        }

        Ok(stats)
    }
}
