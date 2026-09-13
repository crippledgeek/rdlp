//! `Downloader` trait implementation for [`HttpDownloader`].
//!
//! Implements the core download operations: `download_to_file`,
//! `download_to_writer`, `supports`, `file_size`, and `download_with_resume`.

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{DownloadStats, Downloader, ProgressCallback, RdlpError, Result};
use rdlp_http::{RangeSpec, RangedRequest, StrongValidator};
use rdlp_types::Format;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;
use tokio::io::BufWriter;
use tokio_util::sync::CancellationToken;

use super::parallel::{DownloadTarget, ResumeTarget};
use super::{
    BodySink, HttpDownloader, HttpResumeState, RangeVerdict, SendSpec, Sink, Source, StreamPolicy,
    WriteMode, admit_partial_span, require_success, total_from_remaining,
};
use crate::retry::retries_taken;

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

        let timed = tokio::time::timeout(
            timeout,
            self.fresh_download(FreshIo {
                url,
                path,
                progress,
                cancel,
                // Shared across the probe and whichever branch follows it
                // (issue #672), so a retry spent probing is not silently
                // dropped from the reported `DownloadStats.retries` when the
                // download itself succeeds first try — and vice versa.
                retries: Arc::new(AtomicU64::new(0)),
            }),
        );

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
        tokio::time::timeout(
            timeout,
            self.fresh_download(FreshIo {
                url,
                path,
                progress,
                cancel: None,
                // Shared across the probe and whichever branch follows it
                // (issue #672) — see the matching comment in `download_format`.
                retries: Arc::new(AtomicU64::new(0)),
            }),
        )
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
    pub(crate) async fn fresh_download(&self, io: FreshIo<'_>) -> Result<DownloadStats> {
        let FreshIo {
            url,
            path,
            progress,
            cancel,
            retries,
        } = io;
        // F3: single GET probe replaces HEAD x2 + Range:bytes=0-0 sequence.
        let probe = self.probe(url, &retries).await?;
        let size = probe.size;
        let supports_ranges = probe.supports_ranges;

        debug!(
            "Probe result: size={} MB, concurrent={}, ranges={}",
            size.map_or(0, |s| s / 1024 / 1024),
            self.config.concurrent_fragments,
            supports_ranges
        );

        HttpResumeState::record(path, probe.validator.clone(), probe.complete_length)
            .await
            .map_err(RdlpError::Io)?;
        let source = Source::new(url, probe.validator).with_complete_length(probe.complete_length);

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
            self.download_parallel(
                DownloadTarget {
                    source: &source,
                    path,
                    total_size: ps,
                },
                progress,
                Arc::clone(&retries),
            )
            .await?
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
            self.download_sequential(url, path, progress, cancel, &retries)
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
            let hdrs = self.headers();
            let retries = AtomicU64::new(0);

            // Deliberately NOT `download_request`: stdout never resumes and has
            // no offsets to protect, so wreq's decoding stays on and any
            // coding the server picks is fine here.
            let response = self
                .send_with_retry(
                    SendSpec {
                        label: "HTTP GET (stdout)",
                        url,
                        gate: &require_success,
                        retries: &retries,
                    },
                    || self.client().get(url).headers(hdrs.clone()),
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
                        transfer: None,
                    },
                    progress.as_deref(),
                    cancel,
                )
                .await?;

            Ok(finish(
                progress.as_deref(),
                Elapsed {
                    start_time,
                    retries: retries_taken(&retries),
                },
                downloaded,
            ))
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
    /// [`super::range_verdict`] for the three answers and [`Self::resume_from_response`]
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
        // Shared with the fresh path a restart delegates to and with a
        // possible `download_parallel_resume` hand-off (issue #672), so a
        // retry spent on this function's own resume request is not dropped
        // whichever way the attempt ends.
        let retries = Arc::new(AtomicU64::new(0));
        tokio::time::timeout(timeout, async {
            let io = ResumeIo {
                url,
                path,
                resume_from,
                progress,
                cancel,
                retries,
            };
            let Some(state) = HttpResumeState::load(path).await else {
                return self
                    .restart(
                        io,
                        "no resume validator recorded — the partial predates this rdlp \
                         version or the server offered no strong ETag/Last-Modified; RFC \
                         9110 §15.3.7.3 permits combining parts only under a shared strong \
                         validator, so the partial is discarded and the download restarts",
                    )
                    .await;
            };

            let start_time = Instant::now();
            let hdrs = self.headers();
            let source = Source::new(url, Some(state.validator.clone()))
                .with_complete_length(state.complete_length);
            let range = RangeSpec::From(resume_from);
            let meta = source.meta(range);

            // A wrong-span 206 is one bad response, not a verdict on the
            // partial: the gate turns it into the retryable error so the
            // request is re-issued (#526, #674), and every other answer
            // reaches `resume_from_response` with the verdict it was read
            // once for.
            let (response, verdict) = self
                .send_with_retry(
                    SendSpec {
                        label: "HTTP GET (resume)",
                        url,
                        gate: &|response| admit_partial_span(response, &meta, url),
                        retries: &io.retries,
                    },
                    || {
                        rdlp_http::download_request(self.client(), url, Some(&hdrs))
                            .ranged(range, source.validator.as_ref())
                    },
                )
                .await?;

            let attempt = ResumeAttempt {
                io,
                source: &source,
                state: &state,
                start_time,
            };
            self.resume_from_response(attempt, response, verdict?).await
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
    /// - `Replaced` (200): the body is the whole current representation,
    ///   whether because `If-Range` was false (§13.2.2 step 5) or because the
    ///   server ignored `Range` altogether (§14.2 "MAY ignore") — the two are
    ///   indistinguishable. The same strong validator with
    ///   `Content-Length == N` means the N-byte partial already IS it
    ///   (§8.8.1; §15.5.17 notes servers answer 200 where 416 was apt), so
    ///   finish without writing. Otherwise write THIS body from zero and
    ///   record its validator, so an interruption of the rewrite resumes
    ///   correctly.
    /// - `Mismatched` (206 naming another representation, or none): §15.3.7.3
    ///   forbids combining, and the fresh download is the only exit — an
    ///   error would send the orchestrator back into this same resume. Restart.
    /// - `Unsatisfiable` (416): `bytes */L` "indicates the current length"
    ///   (§14.4). `L < N` cannot contain the partial as a prefix, so restart;
    ///   `L > N` contradicts §14.1.2 (satisfiable iff `first-pos < current
    ///   length`) and is an error. `L == N`, and a 416 without `Content-Range`
    ///   (only a SHOULD, §15.5.17), are both settled by
    ///   [`Self::probe_confirms_partial`]: one `If-Range` probe, because 416
    ///   implies the validator matched only on a conforming server (§14.2
    ///   evaluates `Range` after the preconditions), and a server ignoring
    ///   `If-Range` could otherwise pass off a different N-byte representation.
    async fn resume_from_response(
        &self,
        attempt: ResumeAttempt<'_>,
        response: wreq::Response,
        verdict: RangeVerdict,
    ) -> Result<DownloadStats> {
        let source = attempt.source;
        let sidecar = attempt.state;
        let path = attempt.io.path;
        let url = attempt.io.url;
        let resume_from = attempt.io.resume_from;

        match verdict {
            RangeVerdict::Partial { range: got } => {
                if let (Some(stored), Some(now)) = (sidecar.complete_length, got.complete_length())
                    && stored != now
                {
                    return self
                        .restart(
                            attempt.io,
                            &format!(
                                "the validator matched but the complete length changed \
                                 ({stored} → {now}), so the validator cannot be trusted"
                            ),
                        )
                        .await;
                }

                // The whole-file total this tail is held to, in order of
                // authority: the 206's own complete-length; else the length
                // recorded under the same strong validator (§8.8.1 — the
                // sidecar's, which a `bytes N-M/*` answer does not void);
                // else `Content-Length` plus the offset.
                let total_size = got
                    .complete_length()
                    .or(sidecar.complete_length)
                    .or_else(|| total_from_remaining(response.content_length(), resume_from));
                if let Some(total) = total_size
                    && self.can_parallel_resume(&response, total, resume_from)
                {
                    drop(response);
                    let mut attempt = attempt;
                    let stats = self
                        .download_parallel_resume(
                            ResumeTarget {
                                source,
                                path,
                                resume_from,
                                total_size: total,
                            },
                            attempt.io.progress.take(),
                            Arc::clone(&attempt.io.retries),
                        )
                        .await?;
                    return Ok(attempt.finished_with(stats).await);
                }

                // #674: when the response discloses the resource's total
                // length, the appended tail is held to it exactly —
                // `stream_to_file` counts from `resume_from`, so the
                // whole-file total is the expected length for its
                // mid-stream, end-of-stream and on-disk checks.
                let total = self
                    .stream_to_file(
                        response,
                        Sink {
                            path,
                            mode: WriteMode::Append { from: resume_from },
                            expected_total: total_size,
                        },
                        attempt.io.progress.as_deref(),
                        attempt.io.cancel,
                    )
                    .await?;
                Ok(attempt.completed(total).await)
            }
            RangeVerdict::Replaced => {
                let current = StrongValidator::from_headers(response.headers());
                if current.as_ref() == Some(&sidecar.validator)
                    && response.content_length() == Some(resume_from)
                {
                    return Ok(attempt.completed(resume_from).await);
                }
                warn!(
                    "200 to a Range+If-Range request: the representation cannot be confirmed \
                     identical to the {resume_from}-byte partial at '{}'; writing the current \
                     representation from zero",
                    path.display()
                );
                // Discard BEFORE recording the new validator: every failure
                // state in between is then "no sidecar" (or the old one),
                // which restarts. The other order leaves v1's bytes under a
                // sidecar naming v2, and the next resume would append v2's
                // tail to v1's prefix.
                self.discard_partial(path).await?;
                let expected_total = response.content_length();
                HttpResumeState::record(path, current, expected_total)
                    .await
                    .map_err(RdlpError::Io)?;
                let total = self
                    .stream_to_file(
                        response,
                        Sink {
                            path,
                            mode: WriteMode::Create,
                            expected_total,
                        },
                        attempt.io.progress.as_deref(),
                        attempt.io.cancel,
                    )
                    .await?;
                Ok(attempt.completed(total).await)
            }
            RangeVerdict::Mismatched { mismatch } => {
                self.restart(
                    attempt.io,
                    &format!(
                        "the resume response is not the representation the partial came from \
                         ({mismatch})"
                    ),
                )
                .await
            }
            RangeVerdict::Unsatisfiable {
                complete_length: Some(len),
            } if len < resume_from => {
                self.restart(
                    attempt.io,
                    &format!(
                        "the server reports the resource is now {len} bytes, shorter than the \
                         partial"
                    ),
                )
                .await
            }
            RangeVerdict::Unsatisfiable {
                complete_length: Some(len),
            } if len > resume_from => Err(RdlpError::Download {
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
                message: format!(
                    "416 with complete-length {len} > partial size {resume_from}: a range \
                     starting inside the representation is satisfiable (RFC 9110 §14.1.2), so \
                     this is not a satisfiable-range failure. The partial and its \
                     `.http_state.json` sidecar are left for diagnosis; delete both to \
                     download from scratch"
                ),
            }),
            // `Some(len)` with `len == resume_from`, or no `Content-Range` at
            // all: one `If-Range` probe decides.
            RangeVerdict::Unsatisfiable { complete_length } => {
                if self
                    .probe_confirms_partial(&attempt.io, &sidecar.validator)
                    .await?
                {
                    Ok(attempt.completed(resume_from).await)
                } else {
                    let reported = complete_length.map_or_else(
                        || "416 without Content-Range".to_owned(),
                        |len| format!("416 reporting {len} bytes"),
                    );
                    self.restart(
                        attempt.io,
                        &format!(
                            "{reported}, and the If-Range probe does not confirm the partial \
                             as the complete current representation"
                        ),
                    )
                    .await
                }
            }
        }
    }

    /// Whether one `If-Range` probe confirms that the `resume_from`-byte
    /// partial IS the complete current representation: a 206 whose headers
    /// still carry `validator` and whose complete-length is exactly
    /// `resume_from`.
    ///
    /// Three outcomes, not two: `Ok(true)` confirms, `Ok(false)` is an
    /// answer that does not (the caller restarts), and `Err` is no answer
    /// at all — the transport or `Http` error that outlived the probe's
    /// retries, propagated so the partial and its sidecar stay for the next
    /// attempt. Folding that into `false` would let a passing 5xx outage
    /// discard a partial that is, on a conforming server, already complete.
    ///
    /// `complete_length` is `Some` only on a 206, so the headers being judged
    /// are a partial's — the one shape [`StrongValidator::verify_partial`] is
    /// defined over, with its §15.3.7 asymmetry (a 206 must repeat `ETag`
    /// but SHOULD NOT repeat `Last-Modified`).
    async fn probe_confirms_partial(
        &self,
        io: &ResumeIo<'_>,
        validator: &StrongValidator,
    ) -> Result<bool> {
        let resume_from = io.resume_from;
        let probe = self
            .probe_answered(io.url, Some(validator), &io.retries)
            .await?;
        Ok(probe.complete_length == Some(resume_from)
            && validator.verify_partial(&probe.headers).is_ok())
    }

    /// Discard the partial and go through the fresh path, saying why.
    ///
    /// Every "this partial cannot be continued" outcome of a resume — no
    /// sidecar; a matching validator whose complete-length changed; a 206
    /// naming another representation (`Mismatched`); a 416 reporting a
    /// shorter representation; a 416 the `If-Range` probe does not confirm —
    /// ends here, so the warn/discard/restart sequence exists once and the
    /// five reasons read as one log shape.
    async fn restart(&self, io: ResumeIo<'_>, reason: &str) -> Result<DownloadStats> {
        warn!(
            "{reason}; discarding the {}-byte partial at '{}' and restarting the download",
            io.resume_from,
            io.path.display()
        );
        self.discard_partial(io.path).await?;
        self.fresh_download(FreshIo {
            url: io.url,
            path: io.path,
            progress: io.progress,
            cancel: io.cancel,
            retries: io.retries,
        })
        .await
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

/// What a download from byte 0 needs: the resource, the destination, the
/// caller's hooks, and the retry tally shared across the probe and whichever
/// transfer follows it (issue #672) — a restart hands its own tally on so
/// the retries spent on the abandoned resume request are still reported.
pub(crate) struct FreshIo<'a> {
    pub(crate) url: &'a str,
    pub(crate) path: &'a Path,
    pub(crate) progress: Option<Box<dyn ProgressCallback>>,
    pub(crate) cancel: Option<&'a CancellationToken>,
    pub(crate) retries: Arc<AtomicU64>,
}

/// The partial on disk and the caller's hooks — what a restart, an append,
/// a rewrite and an "already complete" each need.
struct ResumeIo<'a> {
    url: &'a str,
    /// The partial file.
    path: &'a Path,
    /// Bytes already on disk.
    resume_from: u64,
    progress: Option<Box<dyn ProgressCallback>>,
    cancel: Option<&'a CancellationToken>,
    /// Every retry this attempt takes — the resume request, the confirming
    /// probe, a parallel fan-out or the fresh path a restart delegates to —
    /// counts here (issue #672).
    retries: Arc<AtomicU64>,
}

/// Everything [`HttpDownloader::resume_from_response`] needs to know about the
/// attempt it is judging, grouped so the verdict arms read as the spec does.
struct ResumeAttempt<'a> {
    io: ResumeIo<'a>,
    /// The URL and the validator every ranged request carried as `If-Range`.
    source: &'a Source,
    /// The sidecar the partial was fetched under.
    state: &'a HttpResumeState,
    /// When the resume request went out; the stats' duration runs from here.
    start_time: Instant,
}

impl ResumeAttempt<'_> {
    /// The download ends with `total` bytes on disk: report, then finish.
    /// `total == resume_from` is the "already complete" ending shared by a
    /// same-validator 200 and a probe-confirmed 416 — the partial already IS
    /// the representation and nothing was written; the append and the
    /// 200-rewrite pass what they streamed.
    async fn completed(self, total: u64) -> DownloadStats {
        let stats = finish(
            self.io.progress.as_deref(),
            Elapsed {
                start_time: self.start_time,
                retries: retries_taken(&self.io.retries),
            },
            total,
        );
        self.finished_with(stats).await
    }

    /// The one ending: the sidecar goes, the stats are the result. Reached
    /// directly by the parallel fan-out, which built and reported its own
    /// stats (its progress reporter owns the callback for the duration), and
    /// through [`Self::completed`] by every sequential ending.
    async fn finished_with(self, stats: DownloadStats) -> DownloadStats {
        HttpResumeState::remove(self.io.path).await;
        stats
    }
}

/// What a finished transfer cost: when it started and how many retries it
/// took — the two `DownloadStats` inputs that are not the byte count.
#[derive(Clone, Copy)]
struct Elapsed {
    start_time: Instant,
    retries: usize,
}

/// Stats for a transfer that ends with `total` bytes delivered, reported to
/// the progress callback exactly as the fresh path does.
fn finish(progress: Option<&dyn ProgressCallback>, elapsed: Elapsed, total: u64) -> DownloadStats {
    let stats = DownloadStats::new(total, elapsed.start_time.elapsed(), elapsed.retries);
    if let Some(callback) = progress {
        callback.on_complete(&stats);
    }
    stats
}
