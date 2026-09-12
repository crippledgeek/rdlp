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
use super::{ContentRange, HTTP_PARTIAL_CONTENT, HttpDownloader, HttpResumeState, Source};
use crate::progress::SpeedMeter;
use crate::retry::{RetryPolicy, with_retry};

#[allow(clippy::too_many_lines, clippy::option_if_let_else)]
#[async_trait]
impl Downloader for HttpDownloader {
    fn protocol(&self) -> &'static str {
        "http"
    }

    /// F6: Override `download_format` to thread `cancel` into the download path.
    ///
    /// The trait's default impl discards `cancel`. This override runs the
    /// shared [`HttpDownloader::fresh_download`] and passes `cancel` through
    /// to it. The whole download — probe included — is wrapped in a
    /// `tokio::select!` so cancellation fires even before the first byte
    /// arrives.
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

        let timed = tokio::time::timeout(timeout, self.fresh_download(url, path, progress, cancel));

        match cancel {
            Some(token) => {
                tokio::select! {
                    biased;
                    () = token.cancelled() => Err(RdlpError::Cancelled),
                    result = timed => {
                        result.map_err(|_| RdlpError::Download {
                            message: format!("Download timed out after {}s", timeout.as_secs()),
                            url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                        })?
                    }
                }
            }
            None => timed.await.map_err(|_| RdlpError::Download {
                message: format!("Download timed out after {}s", timeout.as_secs()),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
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
        tokio::time::timeout(timeout, self.fresh_download(url, path, progress, None))
            .await
            .map_err(|_| RdlpError::Download {
                message: format!("Download timed out after {}s", timeout.as_secs()),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
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
        cancel: Option<&CancellationToken>,
    ) -> Result<DownloadStats> {
        // #347: the trait method now threads `cancel` into the cooperative-cancel
        // variant so the resume path stops mid-stream rather than relying solely
        // on the orchestrator's outer select!.
        self.download_with_resume_with_cancel(url, path, resume_from, progress, cancel)
            .await
    }
}

#[allow(clippy::too_many_lines)]
impl HttpDownloader {
    /// A download from byte 0: probe, then parallel or sequential.
    ///
    /// The one body behind `download_format` and `download_to_file`, which
    /// differ only in how they wrap it (a cancel-racing `select!` versus a
    /// plain timeout with `cancel: None`).
    ///
    /// The probe's strong validator, when it offered one, is persisted to the
    /// resume sidecar BEFORE any body byte is requested and handed to every
    /// chunk as the [`Source`] it must verify against (#565, RFC 9110
    /// §15.3.7.3). The sidecar is removed on success; on failure it stays, so
    /// a later resume can send it as `If-Range`. A sidecar write failure is
    /// surfaced as `RdlpError::Io` rather than downgraded, because a download
    /// that cannot record what it fetched cannot later be resumed safely.
    ///
    /// Writing the sidecar this early is safe because the orchestrator takes
    /// this path only at `resume_from == 0` (`rdlp-api` `execution.rs`), so a
    /// sidecar left behind by a probe-then-GET failure meets a `load` with no
    /// bytes on disk and the resume path simply restarts.
    pub(crate) async fn fresh_download(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&CancellationToken>,
    ) -> Result<DownloadStats> {
        // F3: single GET probe replaces HEAD x2 + Range:bytes=0-0 sequence.
        let probe = self.probe(url).await?;
        let size = probe.size;
        let supports_ranges = probe.supports_ranges;

        debug!(
            "Probe result: size={} MB, concurrent={}, ranges={}",
            size.map_or(0, |s| s / 1024 / 1024),
            self.config.concurrent_fragments,
            supports_ranges
        );

        // "No sidecar" must mean "no validator": one left by an earlier
        // attempt would describe bytes this attempt is about to replace.
        match probe.validator.clone() {
            Some(v) => HttpResumeState::new(v, probe.complete_length)
                .save(path)
                .await
                .map_err(RdlpError::Io)?,
            None => HttpResumeState::remove(path).await,
        }
        let source = Source::new(url, probe.validator);

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

        let stats = if let Some(ps) = parallel_size {
            debug!(
                "Using parallel download mode ({} connections)",
                self.config.concurrent_fragments
            );
            // Parallel-path cooperative cancel is pre-existing AIMD work,
            // out of scope for F6; outer select! at the orchestrator covers it.
            self.download_parallel(&source, path, ps, progress).await?
        } else {
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
            self.download_sequential(url, path, progress, cancel)
                .await?
        };

        HttpResumeState::remove(path).await;
        Ok(stats)
    }

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

            let response = with_retry(
                RetryPolicy::new(&self.config.retry_config, &"HTTP GET (stdout)"),
                || {
                    let client = client.clone();
                    let url = url_string.clone();
                    let hdrs = hdrs.clone();
                    async move {
                        let response =
                            client.get(&url).headers(hdrs).send().await.map_err(|e| {
                                RdlpError::Network {
                                    message: format!("GET request failed: {e}"),
                                    url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                                }
                            })?;

                        check_http_response(&response)?;
                        Ok(response)
                    }
                },
            )
            .await?;

            let total_size = response.content_length();
            let mut buf_writer = BufWriter::with_capacity(self.config.buffer_size, writer);

            let stream = response.bytes_stream();
            tokio::pin!(stream);
            let mut downloaded: u64 = 0;
            let mut last_update = Instant::now();
            let update_interval = PROGRESS_UPDATE_INTERVAL;
            let read_timeout = self.config.read_timeout;
            let mut speed_meter = SpeedMeter::new();
            speed_meter.update(downloaded, start_time);

            loop {
                let next = super::next_with_cancel_and_timeout(
                    stream.as_mut(),
                    cancel,
                    read_timeout,
                    &url_string,
                )
                .await;
                // F6 / #307 follow-up: on cancel, flush whatever bytes are
                // already in the BufWriter so any accumulator-mode caller
                // gets all bytes that crossed the loop's write_all boundary
                // before the cancel arm fired. Pattern matches
                // download_with_resume_with_cancel below. Flush errors are
                // swallowed in favour of surfacing the original Cancelled.
                let next = match next {
                    Err(RdlpError::Cancelled) => {
                        let _ = buf_writer.flush().await;
                        return Err(RdlpError::Cancelled);
                    }
                    other => other?,
                };
                match next {
                    None => break,
                    Some(Err(e)) => {
                        return Err(RdlpError::Network {
                            message: format!("Failed to read chunk: {e}"),
                            url: Some(rdlp_redact::RedactedUrlBuf::from(url_string.as_str())),
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
                                speed_meter.update(downloaded, now);
                                let speed = speed_meter.bytes_per_sec().unwrap_or(0.0);

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
            url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
        })?
    }

    /// F6: cooperative-cancel-aware variant of `download_with_resume`.
    /// The trait method `download_with_resume` delegates here, threading its
    /// `cancel` argument through (#347). Direct callers can also pass a
    /// `CancellationToken` for mid-stream cancellation.
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

            let response = with_retry(
                RetryPolicy::new(&self.config.retry_config, &"HTTP GET (resume)"),
                || {
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
                        .map_err(|e| RdlpError::Network { message: format!("Resume request failed: {e}"), url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())) })?;

                    if response.status().as_u16() != HTTP_PARTIAL_CONTENT {
                        return Err(RdlpError::Download {
                            url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
                            message: format!(
                                "Server does not support resume (expected HTTP \
                                 {HTTP_PARTIAL_CONTENT}, got {}). Cannot continue download \
                                 without overwriting existing data. Please delete the partial \
                                 file and restart the download.",
                                response.status()
                            ),
                        });
                    }

                    // A 206 alone does not prove the body starts where the
                    // partial file ends. The resumed bytes are appended at EOF,
                    // so a response enclosing a different span splices foreign
                    // data into the file at the resume point — the #526
                    // corruption shape on this path. RFC 9110 §15.3.7 requires
                    // the client to inspect Content-Range; do so before any
                    // byte is appended.
                    match ContentRange::from_headers(response.headers()) {
                        Some(range) if range.first_pos == resume_from => {}
                        Some(range) => {
                            return Err(RdlpError::Download {
                                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
                                message: format!(
                                    "Resume response starts at byte {} but the partial file ends \
                                     at {resume_from}; appending it would corrupt the file. \
                                     Please delete the partial file and restart the download.",
                                    range.first_pos
                                ),
                            });
                        }
                        None => {
                            return Err(RdlpError::Download {
                                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
                                message: format!(
                                    "Resume response has a missing, malformed, or invalid \
                                     Content-Range header, so the span it encloses cannot be \
                                     verified against the partial file's {resume_from} bytes. \
                                     Please delete the partial file and restart the download."
                                ),
                            });
                        }
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
                    // No sidecar is consulted on this path yet, so the resume
                    // chunks run unverified exactly as before #565's Task 4.
                    return self
                        .download_parallel_resume(
                            &Source::unverified(url),
                            path,
                            resume_from,
                            total,
                            progress,
                        )
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
            let mut speed_meter = SpeedMeter::new();
            speed_meter.update(downloaded, start_time);

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
                    .map_err(|e| RdlpError::Network { message: format!("Failed to read resume response body from {}: {e}", rdlp_redact::RedactedUrl::new(url_string.as_ref())), url: Some(rdlp_redact::RedactedUrlBuf::from(url_string.as_ref())) })?;

                writer.write_all(&chunk).await.map_err(|e| RdlpError::Io(
                    std::io::Error::new(e.kind(), format!("failed to write to resumed file '{}': {e}", path.display()))
                ))?;
                downloaded += chunk.len() as u64;

                if let Some(ref callback) = progress {
                    let now = Instant::now();
                    if now.duration_since(last_update) >= update_interval {
                        speed_meter.update(downloaded, now);
                        let speed = speed_meter.bytes_per_sec().unwrap_or(0.0);

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
            url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
        })?
    }
}
