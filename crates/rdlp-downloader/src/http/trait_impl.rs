//! `Downloader` trait implementation for [`HttpDownloader`].
//!
//! Implements the core download operations: `download_to_file`,
//! `download_to_writer`, `supports`, `get_size`, and `download_with_resume`.

use async_trait::async_trait;
use futures::StreamExt;
use log::debug;
use rdlp_core::{
    DownloadProgress, DownloadStats, Downloader, ProgressCallback, RdlpError, Result,
    check_http_response,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncWriteExt, BufWriter};

use super::config::PROGRESS_UPDATE_INTERVAL;
use super::{HttpDownloader, with_retry};

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
        let timeout = self.config.download_timeout;
        tokio::time::timeout(timeout, async {
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
                let hdrs = self.headers();

                match with_retry(&self.config.retry_config, "HTTP GET (size check)", || {
                    let client = client.clone();
                    let url = url_string.clone();
                    let hdrs = hdrs.clone();
                    async move {
                        client
                            .get(&url)
                            .headers(hdrs)
                            .header("Range", "bytes=0-0")
                            .send()
                            .await
                            .map_err(|e| RdlpError::Network { message: format!("Size check failed: {e}"), url: Some(url.clone()) })
                    }
                })
                .await
                {
                    Ok(response) => response
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.split('/').nth(1))
                        .and_then(|s| s.parse::<u64>().ok())
                        .inspect(|size| {
                            debug!(
                                "Detected size from Range: {} MB",
                                size / 1024 / 1024
                            );
                        }),
                    Err(_) => None,
                }
            } else {
                size
            };

            let use_parallel = match size {
                Some(s) if s > super::config::PARALLEL_THRESHOLD => {
                    self.config.concurrent_fragments > 1 && supports_ranges
                }
                _ => false,
            };

            if use_parallel {
                debug!(
                    "Using parallel download mode ({} connections)",
                    self.config.concurrent_fragments
                );
                return self
                    .download_parallel(url, path, size.expect("size is Some when use_parallel is true"), progress)
                    .await;
            }

            let reason = match size {
                None | Some(0) => "could not detect file size",
                Some(s) if s <= super::config::PARALLEL_THRESHOLD => "file too small for parallel",
                Some(_) if self.config.concurrent_fragments <= 1 => "concurrent_fragments <= 1",
                Some(_) if !supports_ranges => "server doesn't support ranges",
                Some(_) => "unknown reason",
            };
            debug!(
                "Using sequential download - reason: {reason} (size: {:?} MB, fragments: {}, ranges: {supports_ranges})",
                size.map(|s| s / 1024 / 1024),
                self.config.concurrent_fragments
            );

            self.download_sequential(url, path, progress).await
        })
        .await
        .map_err(|_| RdlpError::Download {
            message: format!("Download timed out after {}s", timeout.as_secs()),
            url: Some(url.to_string()),
        })?
    }

    /// Stream an HTTP download into an arbitrary async writer (e.g. stdout).
    ///
    /// Unlike `download_to_file`, this always uses sequential I/O (no parallel
    /// chunks) because the destination is not seekable. On `BrokenPipe` the
    /// download stops gracefully and returns the bytes written so far.
    async fn download_to_writer(
        &self,
        url: &str,
        writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let timeout = self.config.download_timeout;
        tokio::time::timeout(timeout, async {
            let start_time = Instant::now();
            let client = self.client.clone();
            let url_string = url.to_string();
            let hdrs = self.headers();

            let response = with_retry(&self.config.retry_config, "HTTP GET (stdout)", || {
                let client = client.clone();
                let url = url_string.clone();
                let hdrs = hdrs.clone();
                async move {
                    let response = client.get(&url).headers(hdrs).send().await.map_err(|e| {
                        RdlpError::Network {
                            message: format!("GET request failed: {e}"),
                            url: Some(url.clone()),
                        }
                    })?;

                    check_http_response(&response)?;
                    Ok(response)
                }
            })
            .await?;

            let total_size = response.content_length();
            let mut buf_writer = BufWriter::with_capacity(self.config.buffer_size, writer);

            let mut stream = response.bytes_stream();
            let mut downloaded: u64 = 0;
            let mut last_update = Instant::now();
            let update_interval = PROGRESS_UPDATE_INTERVAL;
            let read_timeout = self.config.read_timeout;

            while let Some(chunk_result) = tokio::time::timeout(read_timeout, stream.next())
                .await
                .map_err(|_| RdlpError::Network {
                message: format!("Read timed out (no data for {}s)", read_timeout.as_secs()),
                url: Some(url_string.clone()),
            })? {
                let chunk = chunk_result.map_err(|e| RdlpError::Network {
                    message: format!("Failed to read chunk: {e}"),
                    url: Some(url_string.clone()),
                })?;

                match buf_writer.write_all(&chunk).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                        debug!("Broken pipe on stdout, stopping gracefully");
                        break;
                    }
                    Err(e) => return Err(RdlpError::Io(e)),
                }
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

            // BrokenPipe accounting: when `write_all` hits BrokenPipe, we
            // break before `downloaded +=`, so the failing chunk is excluded.
            // However, earlier chunks that were written to the BufWriter's
            // internal buffer may not have reached the pipe yet (up to
            // `buffer_size` bytes). This means `downloaded` can *overstate*
            // the bytes actually delivered to the consumer by up to one
            // buffer's worth. This is inherent to buffered I/O and
            // acceptable for stats/logging purposes.
            match buf_writer.flush().await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                    debug!("Broken pipe on flush, ignoring");
                }
                Err(e) => return Err(RdlpError::Io(e)),
            }

            let duration = start_time.elapsed();
            let stats = DownloadStats::new(downloaded, duration, 0);

            if let Some(callback) = progress {
                callback.on_complete(&stats);
            }

            Ok(stats)
        })
        .await
        .map_err(|_| RdlpError::Download {
            message: format!("Download timed out after {}s", timeout.as_secs()),
            url: Some(url.to_string()),
        })?
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn get_size(&self, url: &str) -> Result<Option<u64>> {
        let client = self.client.clone();
        let url = url.to_string();
        let hdrs = self.headers();

        let response = with_retry(&self.config.retry_config, "HTTP HEAD", || {
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

        Ok(response.content_length())
    }

    async fn download_with_resume(
        &self,
        url: &str,
        path: &Path,
        resume_from: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let timeout = self.config.download_timeout;
        tokio::time::timeout(timeout, async {
            let start_time = Instant::now();
            let client = self.client.clone();
            let url_string: Arc<str> = Arc::from(url);
            let hdrs = self.headers();

            let response = with_retry(&self.config.retry_config, "HTTP GET (resume)", || {
                let client = client.clone();
                let url = Arc::clone(&url_string);
                let hdrs = hdrs.clone();
                async move {
                    let response = client
                        .get(url.as_ref())
                        .headers(hdrs)
                        .header("Range", format!("bytes={resume_from}-"))
                        .send()
                        .await
                        .map_err(|e| RdlpError::Network { message: format!("Resume request failed: {e}"), url: Some(url.to_string()) })?;

                    if response.status().as_u16() != 206 {
                        return Err(RdlpError::Download {
                            url: Some(url.to_string()),
                            message: format!(
                                "Server does not support resume (expected HTTP 206, got {}). \
                                 Cannot continue download without overwriting existing data. \
                                 Please delete the partial file and restart the download.",
                                response.status()
                            ),
                        });
                    }

                    Ok(response)
                }
            })
            .await?;

            let total_size = response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split('/').nth(1))
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| response.content_length().map(|size| size + resume_from));

            // Check for parallel resume
            if let Some(total) = total_size {
                let progress_pct = (resume_from as f64 / total as f64) * 100.0;
                let remaining_size = total - resume_from;
                let supports_ranges = response
                    .headers()
                    .get("accept-ranges")
                    .and_then(|v| v.to_str().ok())
                    != Some("none");

                debug!(
                    "Resume analysis: {:.1}% ({} MB / {} MB), remaining={} MB, concurrent={}, ranges={}",
                    progress_pct,
                    resume_from / 1024 / 1024,
                    total / 1024 / 1024,
                    remaining_size / 1024 / 1024,
                    self.config.concurrent_fragments,
                    supports_ranges
                );

                let can_parallel = remaining_size > super::config::PARALLEL_THRESHOLD
                    && self.config.concurrent_fragments > 1
                    && supports_ranges;

                if can_parallel {
                    debug!(
                        "Using parallel resume mode ({} connections), keeping {} MB, parallelizing {} MB",
                        self.config.concurrent_fragments,
                        resume_from / 1024 / 1024,
                        remaining_size / 1024 / 1024
                    );

                    drop(response);
                    return self
                        .download_parallel_resume(url, path, resume_from, total, progress)
                        .await;
                }

                debug!(
                    "Parallel resume not available (remaining: {} MB, concurrent: {}, ranges: {}), using sequential",
                    remaining_size / 1024 / 1024,
                    self.config.concurrent_fragments,
                    supports_ranges
                );
            }

            let file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .await
                .map_err(|e| RdlpError::Io(
                    std::io::Error::new(e.kind(), format!("failed to open partial file for resume '{}': {e}", path.display()))
                ))?;
            let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

            let mut stream = response.bytes_stream();
            let mut downloaded = resume_from;
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
                let chunk = chunk_result
                    .map_err(|e| RdlpError::Network { message: format!("Failed to read resume response body from {url_string}: {e}"), url: Some(url_string.to_string()) })?;

                writer.write_all(&chunk).await.map_err(|e| RdlpError::Io(
                    std::io::Error::new(e.kind(), format!("failed to write to resumed file '{}': {e}", path.display()))
                ))?;
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

                if let Some(ref limiter) = self.rate_limiter {
                    limiter.acquire(chunk.len()).await;
                }
            }

            writer.flush().await.map_err(|e| RdlpError::Io(
                std::io::Error::new(e.kind(), format!("failed to flush resumed file '{}': {e}", path.display()))
            ))?;

            let duration = start_time.elapsed();
            let stats = DownloadStats::new(downloaded, duration, 0);

            if let Some(callback) = progress {
                callback.on_complete(&stats);
            }

            Ok(stats)
        })
        .await
        .map_err(|_| RdlpError::Download {
            message: format!("Download timed out after {}s", timeout.as_secs()),
            url: Some(url.to_string()),
        })?
    }
}
