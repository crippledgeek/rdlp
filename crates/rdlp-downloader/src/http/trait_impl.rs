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
use super::{ExpectedSpan, ExpectedTransfer, HttpDownloader};
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

            let (response, range) = with_retry(
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

                    // A 206 alone does not prove the body starts where the
                    // partial file ends, nor that it reaches the resource's
                    // actual end. The resumed bytes are appended at EOF, so a
                    // response enclosing a different or short span splices
                    // foreign data into the file or silently truncates it at
                    // the resume point — the #526 corruption shape (wrong
                    // offset) and the #674 shape (short tail), both on this
                    // path. RFC 9110 §15.3.7 requires the client to inspect
                    // Content-Range; do so before any byte is appended. Shared
                    // with the chunk/fragment closed-span check via
                    // `ExpectedSpan::OpenEnded` — same validator, same error
                    // family, because both defend the identical invariant.
                    // The validated range is carried out alongside the
                    // response so the caller doesn't re-parse the same header
                    // a second time to learn the total.
                    //
                    // The validator's error propagates UNREWRAPPED: a wrong
                    // span is `RdlpError::Network` (per-response anomaly,
                    // retried by this closure's `with_retry` exactly like the
                    // chunk path retries it), while a bad status or missing
                    // header is `RdlpError::Download` (a capability the
                    // server lacks, not retried). Rewrapping both into
                    // `Download` here — as this path used to — silently
                    // disabled the retry the chunk path already had. The
                    // "delete the partial file and restart" operator
                    // guidance lives in `validate_range_response`'s
                    // open-ended error messages instead of being reattached
                    // here.
                    let range = super::validate_range_response(
                        &response,
                        ExpectedSpan::OpenEnded { start: resume_from },
                        url.as_ref(),
                    )?;

                    Ok((response, range))
                }
            })
            .await?;

            // `range.complete_length` is the validated total from this same
            // response's `Content-Range`; falling back to `Content-Length` +
            // `resume_from` only covers the case the header declared `*`
            // (total genuinely unknown — see `ExpectedSpan::OpenEnded`'s doc).
            let total_size = range
                .complete_length
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

            // #674: when the response discloses the resource's total length,
            // hold the appended tail to it exactly — `downloaded` already
            // starts at `resume_from`, so the whole-file total is the
            // expected length for both the mid-stream and end-of-stream
            // checks, mirroring the chunk path's pair of guards.
            let resume_context = "resumed download";
            let transfer = total_size.map(|expected_len| ExpectedTransfer {
                expected_len,
                context: resume_context,
            });

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

                if let Some(ref transfer) = transfer {
                    transfer.reject_overlong(downloaded, chunk.len() as u64, url_string.as_ref())?;
                }

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

            if let Some(ref transfer) = transfer {
                transfer.confirm_exact(downloaded, url_string.as_ref())?;
            }
            if let Some(total) = total_size {
                super::parallel::verify_output_size(path, total, url_string.as_ref()).await?;
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
}
