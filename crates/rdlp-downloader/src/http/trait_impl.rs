//! `Downloader` trait implementation for [`HttpDownloader`].
//!
//! Implements the core download operations: `download_to_file`,
//! `download_to_writer`, `supports`, `file_size`, and `download_with_resume`.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{
    DownloadProgress, DownloadStats, Downloader, ProgressCallback, RdlpError, Result,
    check_http_response,
};
use rdlp_types::Format;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

use super::config::PROGRESS_UPDATE_INTERVAL;
use super::{HttpDownloader, with_retry};

#[allow(clippy::too_many_lines, clippy::option_if_let_else)]
#[async_trait]
impl Downloader for HttpDownloader {
    fn protocol(&self) -> &'static str {
        "http"
    }

    /// F6: Override `download_format` to thread `cancel` into the download path.
    ///
    /// The trait's default impl discards `cancel`. This override re-implements
    /// the `download_to_file` body inline — probe, then parallel-or-sequential
    /// dispatch — and passes `cancel` to `download_sequential`. The probe itself
    /// is wrapped in a `tokio::select!` so cancellation fires even before the
    /// first byte arrives.
    ///
    /// Note: `download_parallel` does not yet take `cancel`; the outer
    /// orchestrator `select!` provides cancellation for the parallel path.
    async fn download_format(
        &self,
        format: &Format,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&CancellationToken>,
    ) -> Result<DownloadStats> {
        let url = &format.url;
        let timeout = self.config.download_timeout;

        let download_fut = async {
            let probe = self.probe(url).await?;
            let size = probe.size;
            let supports_ranges = probe.supports_ranges;

            debug!(
                "Probe result: size={} MB, concurrent={}, ranges={}",
                size.map_or(0, |s| s / 1024 / 1024),
                self.config.concurrent_fragments,
                supports_ranges
            );

            let parallel_size = match size {
                Some(s) if s > self.config.parallel_threshold => {
                    if self.config.concurrent_fragments > 1 && supports_ranges {
                        Some(s)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(ps) = parallel_size {
                // Parallel-path cooperative cancel is pre-existing AIMD work,
                // out of scope for F6; outer select! at the orchestrator covers it.
                return self.download_parallel(url, path, ps, progress).await;
            }

            self.download_sequential(url, path, progress, cancel).await
        };

        let timed = tokio::time::timeout(timeout, download_fut);

        match cancel {
            Some(token) => {
                tokio::select! {
                    biased;
                    () = token.cancelled() => Err(RdlpError::Cancelled),
                    result = timed => {
                        result.map_err(|_| RdlpError::Download {
                            message: format!("Download timed out after {}s", timeout.as_secs()),
                            url: Some(url.clone()),
                        })?
                    }
                }
            }
            None => timed.await.map_err(|_| RdlpError::Download {
                message: format!("Download timed out after {}s", timeout.as_secs()),
                url: Some(url.clone()),
            })?,
        }
    }

    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let timeout = self.config.download_timeout;
        tokio::time::timeout(timeout, async {
            // F3: single GET probe replaces HEAD x2 + Range:bytes=0-0 sequence.
            // See docs/superpowers/specs/2026-05-21-f3-f6-download-optimization-design.md
            let probe = self.probe(url).await?;
            let size = probe.size;
            let supports_ranges = probe.supports_ranges;

            debug!(
                "Probe result: size={} MB, concurrent={}, ranges={}",
                size.map_or(0, |s| s / 1024 / 1024),
                self.config.concurrent_fragments,
                supports_ranges
            );

            let parallel_size = match size {
                Some(s) if s > self.config.parallel_threshold => {
                    if self.config.concurrent_fragments > 1 && supports_ranges {
                        Some(s)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(ps) = parallel_size {
                debug!(
                    "Using parallel download mode ({} connections)",
                    self.config.concurrent_fragments
                );
                return self.download_parallel(url, path, ps, progress).await;
            }

            let reason = match size {
                None | Some(0) => "could not detect file size",
                Some(s) if s <= self.config.parallel_threshold => "file too small for parallel",
                Some(_) if self.config.concurrent_fragments <= 1 => "concurrent_fragments <= 1",
                Some(_) if !supports_ranges => "server doesn't support ranges",
                Some(_) => "unknown reason",
            };
            debug!(
                "Using sequential download - reason: {reason} (size: {:?} MB, fragments: {}, ranges: {supports_ranges})",
                size.map(|s| s / 1024 / 1024),
                self.config.concurrent_fragments
            );

            self.download_sequential(url, path, progress, None).await
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
    ///
    /// Delegates to `download_to_writer_with_cancel` with `cancel: None`.
    /// For cooperative cancellation callers use that inherent method directly.
    async fn download_to_writer(
        &self,
        url: &str,
        writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // Trait method discards cancel (outer select! covers it). For cooperative
        // cancel, callers use `download_to_writer_with_cancel` directly.
        self.download_to_writer_with_cancel(url, writer, progress, None)
            .await
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn download_with_resume(
        &self,
        url: &str,
        path: &Path,
        resume_from: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // Trait method discards cancel (outer select! covers it). For cooperative
        // cancel, callers use `download_with_resume_with_cancel` directly.
        // TODO(#287): trait method itself may take cancel in a future BC-break.
        self.download_with_resume_with_cancel(url, path, resume_from, progress, None)
            .await
    }
}

#[allow(clippy::too_many_lines)]
impl HttpDownloader {
    /// F6 (#307): cooperative-cancel-aware variant of `download_to_writer`.
    /// The trait method `download_to_writer` delegates here with `cancel: None`.
    /// Direct callers can pass a `CancellationToken` for mid-stream cancellation.
    pub(crate) async fn download_to_writer_with_cancel(
        &self,
        url: &str,
        writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&CancellationToken>,
    ) -> Result<DownloadStats> {
        // F6: pre-cancel guard. Short-circuits before any network round-trip.
        if let Some(token) = cancel
            && token.is_cancelled()
        {
            return Err(RdlpError::Cancelled);
        }

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

            let stream = response.bytes_stream();
            tokio::pin!(stream);
            let mut downloaded: u64 = 0;
            let mut last_update = Instant::now();
            let update_interval = PROGRESS_UPDATE_INTERVAL;
            let read_timeout = self.config.read_timeout;

            loop {
                match super::next_with_cancel_and_timeout(
                    stream.as_mut(),
                    cancel,
                    read_timeout,
                    &url_string,
                )
                .await?
                {
                    None => break,
                    Some(Err(e)) => {
                        return Err(RdlpError::Network {
                            message: format!("Failed to read chunk: {e}"),
                            url: Some(url_string.clone()),
                        });
                    }
                    Some(Ok(chunk)) => {
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

                                let progress_info =
                                    DownloadProgress::new(downloaded, total_size, speed);
                                callback.on_progress(&progress_info);
                                last_update = now;
                            }
                        }

                        if let Some(ref limiter) = self.rate_limiter {
                            limiter.acquire(chunk.len()).await;
                        }
                    }
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

    /// F6: cooperative-cancel-aware variant of `download_with_resume`.
    /// The trait method `download_with_resume` delegates here with `cancel: None`.
    /// Direct callers (e.g. tests) can pass a `CancellationToken` for mid-stream cancellation.
    pub(crate) async fn download_with_resume_with_cancel(
        &self,
        url: &str,
        path: &Path,
        resume_from: u64,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&CancellationToken>,
    ) -> Result<DownloadStats> {
        // F6: pre-cancel guard. Mirrors download_sequential — avoids issuing
        // a network round-trip when the caller has already cancelled.
        if let Some(token) = cancel
            && token.is_cancelled()
        {
            return Err(RdlpError::Cancelled);
        }

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

            let total_size = crate::http::parse_content_range_total(response.headers())
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

                let can_parallel = remaining_size > self.config.parallel_threshold
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

            let stream = response.bytes_stream();
            tokio::pin!(stream);
            let mut downloaded = resume_from;
            let mut last_update = Instant::now();
            let update_interval = PROGRESS_UPDATE_INTERVAL;
            let read_timeout = self.config.read_timeout;

            loop {
                let next = match crate::http::next_with_cancel_and_timeout(
                    stream.as_mut(),
                    cancel,
                    read_timeout,
                    &url_string,
                )
                .await
                {
                    Ok(item) => item,
                    Err(RdlpError::Cancelled) => {
                        // Flush partial bytes already in BufWriter to disk.
                        writer.flush().await.ok();
                        return Err(RdlpError::Cancelled);
                    }
                    Err(e) => return Err(e),
                };

                let Some(chunk_result) = next else { break };
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
