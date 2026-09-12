//! `Downloader` trait implementation for [`HttpDownloader`].
//!
//! Implements the core download operations: `download_to_file`,
//! `download_to_writer`, `supports`, `file_size`, and `download_with_resume`.

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{
    DownloadStats, Downloader, ProgressCallback, RdlpError, Result, check_http_response,
};
use rdlp_http::{RangeSpec, RangedRequest, StrongValidator};
use rdlp_types::Format;
use std::path::Path;
use std::time::Instant;
use tokio::io::BufWriter;
use tokio_util::sync::CancellationToken;

use super::{
    BodySink, HttpDownloader, HttpResumeState, RangeVerdict, Sink, Source, StreamPolicy, WriteMode,
    admit_for_verdict, range_verdict,
};
use crate::retry::{RetryPolicy, with_retry};

#[async_trait]
impl Downloader for HttpDownloader {
    fn protocol(&self) -> &'static str {
        "http"
    }

    /// F6: Override `download_format` to thread `cancel` into the download path.
    ///
    /// The trait's default impl discards `cancel`. This override runs the
    /// shared `HttpDownloader::fresh_download` and passes `cancel` through
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

            let mut buf_writer = BufWriter::with_capacity(self.config.buffer_size, writer);
            let downloaded = self
                .stream_body(
                    response,
                    BodySink {
                        writer: &mut buf_writer,
                        policy: StreamPolicy::StopOnBrokenPipe,
                        offset: 0,
                    },
                    progress.as_deref(),
                    cancel,
                )
                .await?;

            Ok(finish(progress.as_deref(), start_time, downloaded))
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
    ///
    /// The resume request carries the sidecar's strong validator as
    /// `If-Range` (RFC 9110 §13.1.5) and dispatches on what came back — see
    /// [`range_verdict`] for the three answers and [`Self::resume_from_response`]
    /// for what each one means for the partial on disk. No sidecar means no
    /// validator, and §15.3.7.3 grants combining parts only under a shared
    /// strong one: the partial is discarded and the download restarts through
    /// the fresh path rather than growing a second one.
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
            let Some(state) = HttpResumeState::load(path).await else {
                warn!(
                    "No resume validator recorded for '{}'; RFC 9110 §15.3.7.3 permits \
                     combining parts only under a shared strong validator, so the \
                     {resume_from}-byte partial is discarded and the download restarts",
                    path.display()
                );
                self.discard_partial(path).await?;
                return self.fresh_download(url, path, progress, cancel).await;
            };

            let start_time = Instant::now();
            let client = self.client.clone();
            let hdrs = self.headers();
            let source = Source::new(url, Some(state.validator.clone()));
            let range = RangeSpec::From(resume_from);

            let response = with_retry(
                RetryPolicy::new(&self.config.retry_config, &"HTTP GET (resume)"),
                || {
                    let client = client.clone();
                    let source = source.clone();
                    let hdrs = hdrs.clone();
                    async move {
                        let response =
                            rdlp_http::download_request(&client, &source.url, Some(&hdrs))
                                .ranged(range, source.validator.as_ref())
                                .send()
                                .await
                                .map_err(|e| RdlpError::Network {
                                    message: format!("Resume request failed: {e}"),
                                    url: Some(rdlp_redact::RedactedUrlBuf::from(
                                        source.url.as_str(),
                                    )),
                                })?;
                        admit_for_verdict(response)
                    }
                },
            )
            .await?;

            let attempt = ResumeAttempt {
                source: &source,
                state: &state,
                path,
                resume_from,
                start_time,
            };
            self.resume_from_response(attempt, response, progress, cancel)
                .await
        })
        .await
        .map_err(|_| RdlpError::Download {
            message: format!("Download timed out after {}s", timeout.as_secs()),
            url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
        })?
    }

    /// What each answer to `Range` + `If-Range` means for the partial.
    ///
    /// - `Partial` (206): append — or fan the remainder out as verified
    ///   chunks — unless the sidecar and the response disagree about the
    ///   complete length under the same validator, which §8.8.1's uniqueness
    ///   says cannot both be right: restart.
    /// - `Replaced` (200, §13.2.2 step 5): the body is the whole current
    ///   representation. The same strong validator with `Content-Length == N`
    ///   means the N-byte partial already IS it (§8.8.1; §15.5.17 notes
    ///   servers answer 200 where 416 was apt), so finish without writing.
    ///   Otherwise the resource changed: write THIS body from zero and record
    ///   its validator, so an interruption of the rewrite resumes correctly.
    /// - `Unsatisfiable` (416): `bytes */L` "indicates the current length"
    ///   (§14.4). `L == N` is complete; `L < N` cannot contain the partial as
    ///   a prefix, so restart; `L > N` contradicts §14.1.2 (satisfiable iff
    ///   `first-pos < current length`) and is an error. Without `Content-Range`
    ///   (only a SHOULD, §15.5.17) a probe under the same `If-Range` decides.
    async fn resume_from_response(
        &self,
        attempt: ResumeAttempt<'_>,
        response: wreq::Response,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&CancellationToken>,
    ) -> Result<DownloadStats> {
        let ResumeAttempt {
            source,
            state,
            path,
            resume_from,
            start_time,
        } = attempt;
        let url = source.url.as_str();
        let range = RangeSpec::From(resume_from);

        match range_verdict(&response, &source.meta(range), url)? {
            RangeVerdict::Partial { range: got } => {
                if let (Some(stored), Some(now)) = (state.complete_length, got.complete_length())
                    && stored != now
                {
                    warn!(
                        "Validator matched but the complete length changed ({stored} → {now}); \
                         the validator cannot be trusted, so the {resume_from}-byte partial is \
                         discarded and the download restarts"
                    );
                    self.discard_partial(path).await?;
                    return self.fresh_download(url, path, progress, cancel).await;
                }

                let total_size = got
                    .complete_length()
                    .or_else(|| response.content_length().map(|rest| rest + resume_from));
                if let Some(total) = total_size
                    && self.can_parallel_resume(&response, total, resume_from)
                {
                    drop(response);
                    let stats = self
                        .download_parallel_resume(source, path, resume_from, total, progress)
                        .await?;
                    HttpResumeState::remove(path).await;
                    return Ok(stats);
                }

                let total = self
                    .stream_to_file(
                        response,
                        Sink {
                            path,
                            mode: WriteMode::Append { from: resume_from },
                        },
                        progress.as_deref(),
                        cancel,
                    )
                    .await?;
                HttpResumeState::remove(path).await;
                Ok(finish(progress.as_deref(), start_time, total))
            }
            RangeVerdict::Replaced => {
                let current = StrongValidator::from_headers(response.headers());
                if current.as_ref() == Some(&state.validator)
                    && response.content_length() == Some(resume_from)
                {
                    HttpResumeState::remove(path).await;
                    return Ok(finish(progress.as_deref(), start_time, resume_from));
                }
                warn!(
                    "Resource changed since the earlier attempt (200 to If-Range); discarding \
                     the {resume_from}-byte partial and writing the current representation"
                );
                // Discard BEFORE recording the new validator: every failure
                // state in between is then "no sidecar" (or the old one),
                // which restarts. The other order leaves v1's bytes under a
                // sidecar naming v2, and the next resume would append v2's
                // tail to v1's prefix.
                self.discard_partial(path).await?;
                match current {
                    Some(v) => HttpResumeState::new(v, response.content_length())
                        .save(path)
                        .await
                        .map_err(RdlpError::Io)?,
                    None => HttpResumeState::remove(path).await,
                }
                let total = self
                    .stream_to_file(
                        response,
                        Sink {
                            path,
                            mode: WriteMode::Create,
                        },
                        progress.as_deref(),
                        cancel,
                    )
                    .await?;
                HttpResumeState::remove(path).await;
                Ok(finish(progress.as_deref(), start_time, total))
            }
            RangeVerdict::Unsatisfiable {
                complete_length: Some(len),
            } if len == resume_from => {
                HttpResumeState::remove(path).await;
                Ok(finish(progress.as_deref(), start_time, resume_from))
            }
            RangeVerdict::Unsatisfiable {
                complete_length: Some(len),
            } if len < resume_from => {
                warn!(
                    "Server reports the resource is now {len} bytes, shorter than the \
                     {resume_from}-byte partial; discarding it and restarting"
                );
                self.discard_partial(path).await?;
                self.fresh_download(url, path, progress, cancel).await
            }
            RangeVerdict::Unsatisfiable {
                complete_length: Some(len),
            } => Err(RdlpError::Download {
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
                message: format!(
                    "416 with complete-length {len} > partial size {resume_from}: a range \
                     starting inside the representation is satisfiable (RFC 9110 §14.1.2), so \
                     this is not a satisfiable-range failure; leaving the partial for diagnosis"
                ),
            }),
            RangeVerdict::Unsatisfiable {
                complete_length: None,
            } => {
                let probe = self.probe_with(url, Some(&state.validator)).await?;
                if probe.validator.as_ref() == Some(&state.validator)
                    && probe.complete_length == Some(resume_from)
                {
                    HttpResumeState::remove(path).await;
                    Ok(finish(progress.as_deref(), start_time, resume_from))
                } else {
                    warn!(
                        "416 without Content-Range and the probe does not confirm the \
                         {resume_from}-byte partial as complete; discarding it and restarting"
                    );
                    self.discard_partial(path).await?;
                    self.fresh_download(url, path, progress, cancel).await
                }
            }
        }
    }

    /// Today's parallel-resume decision, unchanged: enough remains to be
    /// worth splitting, more than one connection is allowed, and the server
    /// did not declare `Accept-Ranges: none`.
    fn can_parallel_resume(&self, response: &wreq::Response, total: u64, resume_from: u64) -> bool {
        let remaining_size = total.saturating_sub(resume_from);
        let supports_ranges = response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            != Some("none");

        debug!(
            "Resume analysis: {:.1}% ({} MB / {} MB), remaining={} MB, concurrent={}, ranges={}",
            (resume_from as f64 / total as f64) * 100.0,
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
        } else {
            debug!(
                "Parallel resume not available (remaining: {} MB, concurrent: {}, ranges: {}), using sequential",
                remaining_size / 1024 / 1024,
                self.config.concurrent_fragments,
                supports_ranges
            );
        }
        can_parallel
    }

    /// Truncate the partial in place and drop its sidecar, so the download
    /// can restart from zero exactly as a fresh one does (`File::create`).
    ///
    /// `path` is rdlp's own `.rdlp-part` temp name (`rdlp-api`
    /// `orchestrator/naming.rs` `part_path`), never the user's clean target,
    /// so truncating it stays inside the #743/#744 doctrine: nothing a user
    /// supplied is touched.
    async fn discard_partial(&self, path: &Path) -> Result<()> {
        tokio::fs::File::create(path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to discard partial file '{}': {e}", path.display()),
            ))
        })?;
        HttpResumeState::remove(path).await;
        Ok(())
    }
}

/// Everything [`HttpDownloader::resume_from_response`] needs to know about the
/// attempt it is judging, grouped so the verdict arms read as the spec does.
struct ResumeAttempt<'a> {
    /// The URL and the validator every ranged request carried as `If-Range`.
    source: &'a Source,
    /// The sidecar the partial was fetched under.
    state: &'a HttpResumeState,
    /// The partial file.
    path: &'a Path,
    /// Bytes already on disk.
    resume_from: u64,
    /// When the resume request went out; the stats' duration runs from here.
    start_time: Instant,
}

/// Stats for a resume that ends with `total` bytes on disk, reported to the
/// progress callback exactly as the fresh path does.
fn finish(
    progress: Option<&dyn ProgressCallback>,
    start_time: Instant,
    total: u64,
) -> DownloadStats {
    let stats = DownloadStats::new(total, start_time.elapsed(), 0);
    if let Some(callback) = progress {
        callback.on_complete(&stats);
    }
    stats
}
