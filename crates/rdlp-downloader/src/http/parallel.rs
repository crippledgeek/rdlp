//! Parallel download functionality
//!
//! Provides parallel download using multiple range requests with fine-grained chunking.

use super::HttpDownloader;
use super::chunk_ledger::ChunkLedger;
use super::chunk_manifest::{ChunkManifest, ChunkManifestTracker};
use super::chunk_name::{ChunkKind, ChunkSet};
use super::config::DownloaderConfig;
use crate::adaptive::{AdaptiveConfig, AdaptiveController, ChunkRequest, ControllerMode};
use crate::chunking::{ChunkSizeStrategy, calculate_chunks};
use crate::progress::{ProgressMetrics, ProgressReporterConfig, spawn_progress_reporter};
use crate::retry::{LazyLabel, RetryPolicy, Tallies, with_retry_cancellable};
use futures::Stream;
use futures::stream::{self, StreamExt, TryStreamExt};
use log::{debug, error, info, warn};
use rdlp_core::{DownloadStats, ProgressCallback, RdlpError, Result, RetryConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Retry policy for one chunk's body stream, derived from `base`.
///
/// This layer covers failures *during* body transfer — decode errors, a read
/// timeout mid-stream, a wrong-span response — after the connection already
/// succeeded. It nests over `download_range_with_progress`, which runs its own
/// `with_retry` on the `send()`, so total attempts for one chunk are
/// `(min(retries, CHUNK_RETRIES) + 1) x (retries + 1)` — the product of both
/// layers, not a constant. Capping this layer bounds one factor; the other is
/// bounded by `Config::validate`'s ceiling on `retries`. Without the cap, the
/// default ten would compose to over a hundred requests for a single chunk.
///
/// The cap is a ceiling, not an override — a caller that asks for fewer
/// retries (a test asking for none) gets what it asked for. Delays come from
/// `base`, with the ceiling held at [`CHUNK_MAX_DELAY`] so the pre-jitter
/// sequence is the 1s, 2s, 3s this path used before #570. `base`'s jitter
/// setting is preserved and the default enables it, which backon applies
/// *after* the clamp (`backon-1.6.0/src/backoff/exponential.rs` `next`), so
/// realised delays run up to double those figures. That is the intended
/// trade: parallel chunks that fail together would otherwise retry in
/// lockstep.
pub(super) fn chunk_retry_policy(base: &RetryConfig) -> RetryConfig {
    RetryConfig::new(
        base.max_retries.min(CHUNK_RETRIES),
        base.initial_delay,
        base.max_delay.min(CHUNK_MAX_DELAY),
        base.multiplier,
    )
    .with_jitter(base.jitter)
}

/// Ceiling on the delay between chunk body-stream attempts.
const CHUNK_MAX_DELAY: Duration = Duration::from_secs(3);

/// Retries (not attempts) allowed for one chunk's body stream.
const CHUNK_RETRIES: usize = 3;

/// Download a single chunk, retrying transient body-stream failures.
///
/// Shares the one retry mechanism with the HTTP, fragment, and DASH paths
/// (`crate::retry`); what is specific to a chunk is the policy
/// ([`chunk_retry_policy`] — small and fixed, because this nests over the
/// range request's own retry) and the partial-file handling below.
///
/// `cancel` — when `Some`, the whole retry loop races the token, so a cancel
/// arriving mid-backoff returns immediately rather than sitting out the sleep.
/// The partial chunk file is removed on cancel, matching the pre-#570
/// behaviour. A chunk that fails for good needs no cleanup here: the caller's
/// `try_collect` short-circuits and `ChunkLedger::cleanup` removes every
/// registered chunk path, so the merge step never runs.
pub async fn download_chunk_with_retry(
    downloader: &HttpDownloader,
    request: ChunkRequestSpec<'_>,
    cancel: Option<&CancellationToken>,
) -> Result<u64> {
    let ChunkRequestSpec {
        url,
        start,
        end,
        chunk_path,
        chunk_id,
        tallies,
    } = request;
    let config = chunk_retry_policy(&downloader.config.retry_config);
    let label = LazyLabel(|f| write!(f, "Chunk {chunk_id}"));

    // This outer loop's `counting_into` and the inner one inside
    // `download_range_with_progress` share the SAME `tallies.retries` counter
    // (issue #672 follow-up). Neither layer double-counts: each increment
    // corresponds to one real retried HTTP request, regardless of which
    // layer issued it. A chunk recovered by a single inner retry (a 500
    // absorbed before the outer loop ever sees an `Err`) reports 1. An inner
    // loop that exhausts its own `max_retries` on consecutive 5xx responses
    // returns `Err` to the outer loop, which then retries the SAME failure
    // kind — that is a distinct extra request too, and correctly adds
    // another increment, not a duplicate of the inner ones already counted.
    let outcome = with_retry_cancellable(
        RetryPolicy::new(&config, &label).counting_into(tallies.retries.as_ref()),
        cancel,
        || async {
            let attempt = downloader
                .download_range_with_progress(
                    ChunkRequestSpec {
                        url,
                        start,
                        end,
                        chunk_path,
                        chunk_id,
                        tallies: tallies.clone(),
                    },
                    cancel,
                )
                .await;
            if attempt.is_err() {
                // Drop this attempt's partial before returning. backon sleeps
                // *after* the error and only builds the next attempt afterwards,
                // so cleaning up at the start of that attempt instead would leave
                // the bytes on disk for the whole backoff. Not needed for the next
                // attempt's correctness — `download_range_with_progress` opens
                // with `File::create`, which truncates.
                let _ = tokio::fs::remove_file(chunk_path).await;
            }
            attempt
        },
    )
    .await;

    if matches!(outcome, Err(RdlpError::Cancelled)) {
        let _ = tokio::fs::remove_file(chunk_path).await;
    }
    outcome
}

/// Which bytes of which URL a chunk download covers, where they land, and
/// the attempt-wide counters the transfer reports into.
///
/// These six values travel together through the AIMD unfold closure and the
/// retry loop. Grouping them takes [`download_chunk_with_retry`] from eight
/// positional arguments to three, and — the point — stops two bare `u64`
/// offsets and a `u64` id from sitting in a row where only their order
/// distinguishes them.
pub struct ChunkRequestSpec<'a> {
    pub url: &'a str,
    /// First byte of the range, inclusive.
    pub start: u64,
    /// Last byte of the range, inclusive.
    pub end: u64,
    /// Where this chunk's bytes are written before the merge step.
    pub chunk_path: &'a Path,
    /// Identifies the chunk in logs.
    pub chunk_id: u64,
    /// Bytes written and retries taken, shared across every chunk task in
    /// this attempt: `bytes` feeds progress reporting as the body streams
    /// in, `retries` makes the finished `DownloadStats` report every retry
    /// actually taken (issue #672). Carried here, as the one pair it is,
    /// rather than as two more positional parameters on
    /// [`download_chunk_with_retry`].
    pub tallies: Tallies,
}

/// Mode for chunk merging operations
#[derive(Clone, Copy)]
enum MergeMode {
    /// Create new file and merge chunks (.part suffix)
    Create,
    /// Append chunks to existing file (.resume suffix)
    Append,
}

impl MergeMode {
    const fn log_action(self) -> &'static str {
        match self {
            Self::Create => "Merging",
            Self::Append => "Appending",
        }
    }
}

/// The kind of download attempt in progress: a fresh transfer starting at
/// byte 0, or a resume of a previously interrupted attempt starting at a
/// known byte offset.
///
/// Chunk kind, byte offset, and merge mode must always agree — a fresh
/// attempt merged in `Append` mode (or a resume merged in `Create` mode)
/// silently corrupts the output file, and before this type existed nothing
/// stopped that: `ChunkKind::Fresh` could be threaded alongside
/// `MergeMode::Append` and the compiler would not object. That is #568's bug
/// shape one level up — the chunk-name-grammar mismatch #568 fixed was one
/// instance of these three settings drifting apart; this type makes the
/// drift unrepresentable by deriving [`ChunkKind`] and [`MergeMode`] from a
/// single value instead of threading them as three independent parameters.
#[derive(Debug, Clone, Copy)]
pub(super) enum Attempt {
    /// A brand-new download, starting at byte 0.
    Fresh,
    /// Resuming a previously interrupted download.
    Resume {
        /// Byte offset already downloaded and present in the output file.
        from: u64,
    },
}

impl Attempt {
    /// Byte offset every chunk's range is relative to.
    pub(super) const fn byte_offset(self) -> u64 {
        match self {
            Self::Fresh => 0,
            Self::Resume { from } => from,
        }
    }

    /// The [`ChunkKind`] this attempt's chunk files must be named with.
    pub(super) const fn chunk_kind(self) -> ChunkKind {
        match self {
            Self::Fresh => ChunkKind::Fresh,
            Self::Resume { .. } => ChunkKind::Resume,
        }
    }

    /// The [`MergeMode`] this attempt's chunks must be merged with.
    const fn merge_mode(self) -> MergeMode {
        match self {
            Self::Fresh => MergeMode::Create,
            Self::Resume { .. } => MergeMode::Append,
        }
    }
}

/// Global atomic counter for generating unique download IDs
static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh (non-resume) HTTP download's fixed target: the resource, its
/// destination file, and the server-advertised total size the assembled
/// output must match (`verify_merged_size`'s backstop). The three are
/// meaningless apart — a URL with no destination, a destination with no
/// expected size to verify against, or a size with nothing to check it
/// against are each an incomplete description of the same download.
pub(super) struct DownloadTarget<'a> {
    pub(super) url: &'a str,
    pub(super) path: &'a Path,
    pub(super) total_size: u64,
}

/// Where a resumed download picks up: the resource, the destination file,
/// the byte offset already on disk, and the server-advertised total size
/// the finished file must match. Same rationale as [`DownloadTarget`], plus
/// `resume_from` — none of the four means "resume this transfer" without
/// the other three.
pub(super) struct ResumeTarget<'a> {
    pub(super) url: &'a str,
    pub(super) path: &'a Path,
    pub(super) resume_from: u64,
    pub(super) total_size: u64,
}

impl HttpDownloader {
    /// Parallel download using multiple range requests with fine-grained chunking.
    ///
    /// When `config.adaptive` is `true`, chunk sizes and connection counts are
    /// dynamically tuned by an AIMD controller. Otherwise the static
    /// `chunk_strategy` is used with a fixed connection count.
    pub(super) async fn download_parallel(
        &self,
        target: DownloadTarget<'_>,
        progress: Option<Box<dyn rdlp_core::ProgressCallback>>,
        retries: Arc<AtomicU64>,
    ) -> Result<DownloadStats> {
        let DownloadTarget {
            url,
            path,
            total_size,
        } = target;
        let start_time = Instant::now();
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let attempt = Attempt::Fresh;

        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let chunk_set =
            ChunkSet::for_attempt(path, download_id, attempt.chunk_kind()).map_err(|e| {
                RdlpError::Download {
                    message: format!("{e:#}"),
                    url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
                }
            })?;

        let progress: Option<Arc<dyn rdlp_core::ProgressCallback>> = progress.map(Arc::from);
        let tallies = Tallies {
            bytes: Arc::new(AtomicU64::new(0)),
            retries: Arc::clone(&retries),
        };
        let _progress_guard = spawn_progress_reporter(
            progress.clone(),
            ProgressMetrics::bytes_only(Arc::clone(&tallies.bytes)),
            ProgressReporterConfig::http(start_time, total_size, 0),
        );

        let span = TransferSpan {
            url,
            offset: attempt.byte_offset(),
            len: total_size,
        };
        let layout = ChunkLayout {
            chunk_set: &chunk_set,
            temp_dir,
        };
        let tracking =
            ChunkTracking::new(build_manifest_tracker(&layout, attempt, span.len), tallies);
        let (chunk_paths, total_downloaded) = if self.config.adaptive {
            self.download_parallel_adaptive(
                ParallelRun {
                    span,
                    layout,
                    tracking: &tracking,
                },
                progress.clone(),
                None,
            )
            .await?
        } else {
            self.download_parallel_static(ParallelRun {
                span,
                layout,
                tracking: &tracking,
            })
            .await?
        };

        let chunk_count = chunk_paths.len();

        merge_chunks_ordered(path, &chunk_paths, &self.config, attempt.merge_mode()).await?;

        // Backstop (#526): the assembly must match the advertised length before
        // this file is handed back as a completed download.
        verify_merged_size(path, total_size, url).await?;

        // The merge above succeeded, so the manifest's job for this attempt
        // is done — leaving it behind would let a future scan trust stale
        // per-chunk lengths against chunk files that no longer exist.
        tracking.manifest.delete().await;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(
            total_downloaded,
            duration,
            crate::retry::retries_taken(&retries),
        );

        info!(
            "Download complete: {} MB in {:.1}s ({:.1} MB/s)",
            total_downloaded / 1024 / 1024,
            duration.as_secs_f64(),
            (total_downloaded as f64 / duration.as_secs_f64()) / 1024.0 / 1024.0
        );
        debug!(
            "All {} chunks completed ({} MB total)",
            chunk_count,
            total_downloaded / 1024 / 1024
        );

        Ok(stats)
    }

    /// Parallel resume: downloads remaining chunks in parallel.
    pub(super) async fn download_parallel_resume(
        &self,
        target: ResumeTarget<'_>,
        progress: Option<Box<dyn rdlp_core::ProgressCallback>>,
        retries: Arc<AtomicU64>,
    ) -> Result<DownloadStats> {
        let ResumeTarget {
            url,
            path,
            resume_from,
            total_size,
        } = target;
        let start_time = Instant::now();
        let remaining_size = total_size - resume_from;
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let attempt = Attempt::Resume { from: resume_from };

        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let chunk_set =
            ChunkSet::for_attempt(path, download_id, attempt.chunk_kind()).map_err(|e| {
                RdlpError::Download {
                    message: format!("{e:#}"),
                    url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
                }
            })?;

        let progress: Option<Arc<dyn rdlp_core::ProgressCallback>> = progress.map(Arc::from);
        let tallies = Tallies {
            bytes: Arc::new(AtomicU64::new(resume_from)),
            retries: Arc::clone(&retries),
        };
        let mut progress_guard = spawn_progress_reporter(
            progress.clone(),
            ProgressMetrics::bytes_only(Arc::clone(&tallies.bytes)),
            ProgressReporterConfig::http(start_time, total_size, resume_from),
        );

        debug!(
            "Parallel resume: download_id={download_id}, already={} MB ({:.1}%), remaining={} MB, concurrent={}",
            resume_from / 1024 / 1024,
            (resume_from as f64 / total_size as f64) * 100.0,
            remaining_size / 1024 / 1024,
            self.config.concurrent_fragments
        );

        let span = TransferSpan {
            url,
            offset: attempt.byte_offset(),
            len: remaining_size,
        };
        let layout = ChunkLayout {
            chunk_set: &chunk_set,
            temp_dir,
        };
        let tracking =
            ChunkTracking::new(build_manifest_tracker(&layout, attempt, span.len), tallies);
        let result = if self.config.adaptive {
            self.download_parallel_adaptive(
                ParallelRun {
                    span,
                    layout,
                    tracking: &tracking,
                },
                progress.clone(),
                None,
            )
            .await
        } else {
            self.download_parallel_static(ParallelRun {
                span,
                layout,
                tracking: &tracking,
            })
            .await
        };

        let (chunk_paths, newly_downloaded) = match result {
            Ok(v) => v,
            Err(e) => {
                error!("Resume failed: {e}");
                progress_guard.abort();
                return Err(e);
            }
        };

        merge_chunks_ordered(path, &chunk_paths, &self.config, attempt.merge_mode()).await?;

        // Backstop (#526). On the resume path this also catches a `resume_from`
        // that disagreed with the file's real length: the appended bytes land
        // at EOF regardless of the offset the ranges were requested from, so a
        // mismatch shows up here as a wrong final size.
        verify_merged_size(path, total_size, url).await?;

        tracking.manifest.delete().await;

        let duration = start_time.elapsed();
        let total_downloaded = resume_from + newly_downloaded;
        let stats = DownloadStats::new(
            total_downloaded,
            duration,
            crate::retry::retries_taken(&retries),
        );

        info!(
            "Resume complete: {} MB total, {} MB new in {:.1}s ({:.1} MB/s)",
            total_downloaded / 1024 / 1024,
            newly_downloaded / 1024 / 1024,
            duration.as_secs_f64(),
            (newly_downloaded as f64 / duration.as_secs_f64()) / 1024.0 / 1024.0
        );

        Ok(stats)
    }

    /// Build the AIMD controller for one adaptive download attempt.
    fn build_controller(
        &self,
        size_to_download: u64,
        log_callback: Option<Arc<dyn ProgressCallback>>,
    ) -> Arc<AdaptiveController> {
        Arc::new(AdaptiveController::new(
            size_to_download,
            AdaptiveConfig {
                max_connections: self.config.concurrent_fragments,
                ..AdaptiveConfig::default()
            },
            ControllerMode::HttpChunked,
            log_callback,
        ))
    }

    /// Adaptive download: uses `AdaptiveController` for dynamic chunk sizing and concurrency.
    ///
    /// Returns `(chunk_paths_in_order, total_bytes_downloaded)`.
    /// `span.offset` is added to every chunk's start position (for resume).
    ///
    /// `cancel` is `Option<CancellationToken>` (owned, not a reference) so it
    /// can be cloned into the `try_unfold` closure. `CancellationToken` is
    /// cheaply cloneable. F6 (#308) infrastructure: no caller wires a live
    /// token today (both call sites pass `None`) — the outer orchestrator's
    /// own `select!` covers cancellation for the parallel path in the
    /// meantime (see `trait_impl.rs::download_format`) — but the plumbing
    /// here is deliberate, not dead (issue #673 tracks wiring a real token
    /// through), and `parallel_adaptive_cancel_uses_biased_select` guards it.
    async fn download_parallel_adaptive(
        &self,
        run: ParallelRun<'_>,
        log_callback: Option<Arc<dyn ProgressCallback>>,
        cancel: Option<CancellationToken>,
    ) -> Result<(Vec<PathBuf>, u64)> {
        let ParallelRun {
            span,
            layout,
            tracking,
        } = run;
        let TransferSpan {
            url,
            offset: byte_offset,
            len: size_to_download,
        } = span;
        let ChunkLayout {
            chunk_set,
            temp_dir,
        } = layout;
        let controller = self.build_controller(size_to_download, log_callback);
        let sem = controller.semaphore().clone();

        let url_shared: Arc<str> = Arc::from(url);

        // Generate tasks lazily using stream::unfold driven by controller.next_chunk().
        // buffer_unordered with a generous buffer lets the semaphore control actual concurrency.
        let buffer_factor = self.config.concurrent_fragments * 2;

        // Set once a sibling chunk fails, so the unfold generator stops
        // scheduling NEW chunks. It deliberately does NOT cancel chunks
        // already in flight (see `drain_collecting_first_error`) — only
        // gates further generation. This is purely a latency optimization:
        // sweep correctness never depended on it, since a stale read of this
        // flag only lets the generator produce one extra chunk before it
        // observes the flip, and the drain below waits that chunk out like
        // any other in-flight chunk. Relaxed ordering is correct for the
        // same reason — there is no data this flag needs to publish besides
        // itself.
        let stop_scheduling = Arc::new(AtomicBool::new(false));

        let buffered = {
            let downloader = self.clone();
            let url_arc = url_shared.clone();
            let temp_dir_owned = temp_dir.to_path_buf();
            let chunk_set_owned = chunk_set.clone();
            // Clone once outside the closure; each iteration re-clones from this.
            let cancel_outer = cancel.clone();
            let stop_flag = stop_scheduling.clone();
            let controller_for_jobs = controller.clone();
            let ledger_for_unfold = Arc::clone(&tracking.ledger);
            let tallies_for_unfold = tracking.tallies.clone();
            let manifest_for_unfold = tracking.manifest.clone();

            stream::try_unfold((controller.clone(), 0u64), move |(ctrl, chunk_id)| {
                let sem = sem.clone();
                let downloader = downloader.clone();
                let url = url_arc.clone();
                let chunk_path = chunk_set_owned.path_in(&temp_dir_owned, chunk_id);
                // Registered BEFORE this chunk's download starts (the future
                // below hasn't been polled yet), so a chunk that fails
                // mid-write is still tracked for cleanup.
                ledger_for_unfold.register(chunk_path.clone());
                let ctrl_next = ctrl.clone();
                let cancel_for_unfold = cancel_outer.clone();
                let stop_flag = stop_flag.clone();
                let controller_for_job = controller_for_jobs.clone();
                let tallies = tallies_for_unfold.clone();
                let manifest_for_job = manifest_for_unfold.clone();

                async move {
                    // A sibling chunk has already failed terminally: stop
                    // generating further work rather than downloading the
                    // rest of a doomed transfer (see the drain loop below).
                    if stop_flag.load(Ordering::Relaxed) {
                        return Ok(None);
                    }

                    let Some(chunk) = ctrl.next_chunk() else {
                        return Ok(None);
                    };

                    // F6 (#308): pre-dispatch cancel guard before semaphore acquisition.
                    if let Some(ref token) = cancel_for_unfold
                        && token.is_cancelled()
                    {
                        return Err(RdlpError::Cancelled);
                    }

                    let job = AdaptiveChunkJob {
                        downloader,
                        url,
                        chunk_id,
                        chunk_path,
                        byte_offset,
                        chunk,
                        semaphore: sem,
                        controller: controller_for_job,
                        cancel: cancel_for_unfold,
                        tallies,
                        manifest: manifest_for_job,
                    };

                    Ok(Some((run_adaptive_chunk(job), (ctrl_next, chunk_id + 1))))
                }
            })
            .try_buffer_unordered(buffer_factor)
        };
        futures::pin_mut!(buffered);

        let (results, first_err) = drain_collecting_first_error(buffered, &stop_scheduling).await;

        if let Some(e) = first_err {
            error!("Adaptive download failed: {e}");
            tracking.ledger.cleanup().await;
            tracking.manifest.delete().await;
            return Err(e);
        }

        Ok(into_ordered_paths(results))
    }

    /// Static download: uses a fixed chunk size and connection count.
    ///
    /// Returns `(chunk_paths_in_order, total_bytes_downloaded)`.
    async fn download_parallel_static(&self, run: ParallelRun<'_>) -> Result<(Vec<PathBuf>, u64)> {
        let ParallelRun {
            span,
            layout,
            tracking,
        } = run;
        let TransferSpan {
            url,
            offset: byte_offset,
            len: size_to_download,
        } = span;
        let ChunkLayout {
            chunk_set,
            temp_dir,
        } = layout;
        let plan = StaticChunkPlan::new(size_to_download, byte_offset, self.config.chunk_strategy);

        debug!(
            chunk_set:? = chunk_set,
            size_mb = size_to_download / 1024 / 1024,
            chunk_kb = plan.chunk_size / 1024,
            chunks = plan.total_chunks,
            concurrent = self.config.concurrent_fragments;
            "Static chunk analysis"
        );

        let url_shared: Arc<str> = Arc::from(url);
        let temp_dir_owned = temp_dir.to_path_buf();

        let results: Vec<(usize, PathBuf, u64)> = match stream::iter(0..plan.total_chunks)
            .map(|chunk_id| {
                let (start, end) = plan.range_of(chunk_id);
                let chunk_path = chunk_set.path_in(&temp_dir_owned, chunk_id as u64);
                // Registered BEFORE this chunk's download starts, so a
                // chunk that fails mid-write is still tracked for cleanup
                // — same mechanism as the adaptive path, just registered up
                // front here since the chunk ids are all known in advance.
                tracking.ledger.register(chunk_path.clone());
                let downloader = self.clone();
                let url = Arc::clone(&url_shared);
                let tallies = tracking.tallies.clone();
                let manifest = tracking.manifest.clone();

                async move {
                    let result = download_chunk_with_retry(
                        &downloader,
                        ChunkRequestSpec {
                            url: &url,
                            start,
                            end,
                            chunk_path: &chunk_path,
                            chunk_id: chunk_id as u64,
                            tallies,
                        },
                        None,
                    )
                    .await;
                    match &result {
                        Ok(bytes) => manifest.record_and_save(chunk_id as u64, *bytes).await,
                        Err(e) => error!("Chunk {chunk_id} failed after retries: {e}"),
                    }
                    result.map(|bytes| (chunk_id, chunk_path, bytes))
                }
            })
            .buffer_unordered(self.config.concurrent_fragments)
            .try_collect::<Vec<_>>()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("Download failed: {e}");
                tracking.ledger.cleanup().await;
                tracking.manifest.delete().await;
                return Err(e);
            }
        };

        Ok(into_ordered_paths(results))
    }
}

/// The span of bytes being transferred in one download attempt: the
/// resource URL, the byte offset every chunk's range is shifted by (0 for a
/// fresh download, `resume_from` for a resume), and the length of THIS
/// attempt's transfer (the full size for fresh, the remaining size for
/// resume). The three travel together — every chunk-planning call in both
/// `download_parallel_adaptive` and `download_parallel_static` needs all
/// three to compute a single chunk's absolute range.
struct TransferSpan<'a> {
    url: &'a str,
    offset: u64,
    len: u64,
}

/// Where this attempt's chunk files live: the naming grammar plus the
/// directory they're written into. The two travel together — every chunk
/// path in both `download_parallel_adaptive` and `download_parallel_static`
/// is `chunk_set.path_in(temp_dir, id)`, never one without the other.
struct ChunkLayout<'a> {
    chunk_set: &'a ChunkSet,
    temp_dir: &'a Path,
}

/// One parallel-download attempt, fully described: the transfer to perform,
/// where its chunks land on disk, and the per-chunk bookkeeping the run
/// updates as chunks complete. Grouped because each is meaningless alone
/// for a parallel run — a `span` with nowhere to put its chunks, a `layout`
/// with no span to fill it, or `tracking` with no run to track are all
/// incomplete on their own (Fowler's deletion test), unlike the
/// `RunControls{log_callback, cancel}` pairing a prior review rejected as
/// unrelated concerns bagged together.
///
/// `tracking` is borrowed, not owned: the entry point that built it
/// (`download_parallel` / `download_parallel_resume`) still needs its
/// manifest handle after the run returns, to delete the manifest once the
/// merge has consumed the chunk files it describes.
struct ParallelRun<'a> {
    span: TransferSpan<'a>,
    layout: ChunkLayout<'a>,
    tracking: &'a ChunkTracking,
}

/// The per-chunk bookkeeping one download attempt updates as each chunk
/// completes: the ledger owning deletion authority over the attempt's chunk
/// files (failure cleanup), the on-disk chunk-completion manifest recording
/// which chunks finished and at what length (#675, crash recovery), and the
/// byte/retry tallies the progress reporter and the finished
/// `DownloadStats` read (#672).
///
/// Grouped as one type because every per-chunk future in both the adaptive
/// `try_unfold` closure and the static `stream::iter` closure touches all
/// three, side by side: the ledger takes the chunk path before the future
/// is polled, and a successful download records its length into the
/// manifest; the byte and retry tallies are updated while it runs. All
/// three are mutated as chunks complete — unlike [`TransferSpan`] and
/// [`ChunkLayout`], which are decided once before the first chunk is
/// dispatched and only read thereafter. The byte counter lives in `tallies`
/// (as `Tallies::bytes`) and nowhere else.
struct ChunkTracking {
    ledger: Arc<ChunkLedger>,
    manifest: ChunkManifestTracker,
    tallies: Tallies,
}

impl ChunkTracking {
    /// A fresh ledger for a new attempt, alongside the attempt's manifest
    /// handle (from [`build_manifest_tracker`]) and the run's tallies (which
    /// the caller seeds — a resume starts `bytes` at `resume_from`).
    fn new(manifest: ChunkManifestTracker, tallies: Tallies) -> Self {
        Self {
            ledger: Arc::new(ChunkLedger::new()),
            manifest,
            tallies,
        }
    }
}

/// Build the write-side chunk-manifest handle for one download attempt
/// (#675). Centralized so `download_parallel` and `download_parallel_resume`
/// build it identically instead of each re-deriving the manifest path from
/// `layout.chunk_set` and re-stating `ChunkManifest::new`'s arguments.
/// `size_to_download` is the attempt's `TransferSpan::len` — the manifest
/// records THIS attempt's transfer length, which for a resume is the
/// remaining size, not the file's total.
fn build_manifest_tracker(
    layout: &ChunkLayout<'_>,
    attempt: Attempt,
    size_to_download: u64,
) -> ChunkManifestTracker {
    // A legacy `chunk_set` (no `download_id`) has nowhere to write a
    // manifest — `ChunkSet::manifest` already encodes that as `None`, so
    // this falls straight out to a tracker whose recording is a no-op,
    // rather than treating "no manifest" as an impossible case to panic on.
    let Some((download_id, path)) = layout.chunk_set.manifest(layout.temp_dir) else {
        return ChunkManifestTracker::disabled();
    };
    let manifest = ChunkManifest::new(download_id, attempt.chunk_kind(), size_to_download);
    ChunkManifestTracker::new(manifest, Some(path))
}

/// Everything one adaptive-chunk worker future needs to run to completion,
/// gathered into one named-field value so the `try_unfold` closure hands it
/// to [`run_adaptive_chunk`] as a single argument instead of a long
/// positional list.
struct AdaptiveChunkJob {
    downloader: HttpDownloader,
    url: Arc<str>,
    chunk_id: u64,
    chunk_path: PathBuf,
    byte_offset: u64,
    chunk: ChunkRequest,
    semaphore: Arc<Semaphore>,
    controller: Arc<AdaptiveController>,
    cancel: Option<CancellationToken>,
    /// Bytes and retries, shared across every chunk task in this attempt
    /// (issue #672).
    tallies: Tallies,
    /// Chunk-completion manifest this chunk's length is recorded into on
    /// success (#675).
    manifest: ChunkManifestTracker,
}

/// Run one adaptive chunk download: acquire a concurrency permit (racing
/// cancellation), download with retry, and report the outcome to the
/// controller.
async fn run_adaptive_chunk(job: AdaptiveChunkJob) -> Result<(u64, PathBuf, u64)> {
    let AdaptiveChunkJob {
        downloader,
        url,
        chunk_id,
        chunk_path,
        byte_offset,
        chunk,
        semaphore,
        controller,
        cancel,
        tallies,
        manifest,
    } = job;

    // F6 (#308): race semaphore acquisition against cancel so a pending
    // permit-wait unblocks immediately on cancellation.
    let _permit = if let Some(ref token) = cancel {
        tokio::select! {
            biased;
            () = token.cancelled() => return Err(RdlpError::Cancelled),
            p = semaphore.acquire_owned() => p.map_err(|_| RdlpError::Download {
                message: "Semaphore closed".to_string(),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
            })?,
        }
    } else {
        semaphore
            .acquire_owned()
            .await
            .map_err(|_| RdlpError::Download {
                message: "Semaphore closed".to_string(),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
            })?
    };

    let start_time = Instant::now();
    let abs_start = byte_offset + chunk.start;
    // end is exclusive in ChunkRequest, but HTTP Range is inclusive
    let abs_end = byte_offset + chunk.end - 1;

    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            url: &url,
            start: abs_start,
            end: abs_end,
            chunk_path: &chunk_path,
            chunk_id,
            tallies,
        },
        cancel.as_ref(),
    )
    .await;

    match &result {
        Ok(bytes) => {
            controller.report_chunk_complete(*bytes, start_time.elapsed());
            manifest.record_and_save(chunk_id, *bytes).await;
        }
        Err(e) => {
            error!("Adaptive chunk {chunk_id} failed after retries: {e}");
        }
    }

    result.map(|bytes| (chunk_id, chunk_path, bytes))
}

/// Drain `stream` to completion, collecting every `Ok` item while retaining
/// only the FIRST `Err` — later errors are logged, not silently dropped.
///
/// This is deliberately not `try_collect`: dropping a `TryBufferUnordered`
/// stream mid-poll on the first error would abandon any chunk still in
/// flight, and `tokio::fs` operations dispatched to the blocking pool keep
/// running in the background after their driving future is dropped
/// (`tokio::fs` docs) — sweeping immediately would race those writes and could
/// delete a file the background write hasn't produced yet, or leave a file
/// created just after the sweep ran.
///
/// `stop_scheduling` is set as soon as the first error is observed so the
/// caller's generator (a `stream::try_unfold`) stops producing NEW work.
/// This bounds the drain — it does not eliminate it: `BufferUnordered`
/// (and `TryBufferUnordered`) refill their in-flight queue back up to its
/// concurrency limit on every poll *before* checking that queue for a
/// completed item, including the poll that surfaces this very error. So by
/// the time the first error is observed, up to one full buffer's worth of
/// chunks are already real, in-flight HTTP requests; the drain waits out
/// exactly those (never zero, never the rest of the transfer — new
/// generation is cut off from here on).
async fn drain_collecting_first_error<S, T>(
    mut stream: S,
    stop_scheduling: &AtomicBool,
) -> (Vec<T>, Option<RdlpError>)
where
    S: Stream<Item = Result<T>> + Unpin,
{
    let mut items = Vec::new();
    let mut first_err: Option<RdlpError> = None;

    while let Some(item) = stream.next().await {
        match item {
            Ok(v) => items.push(v),
            Err(e) => {
                if first_err.is_none() {
                    stop_scheduling.store(true, Ordering::Relaxed);
                    first_err = Some(e);
                } else {
                    warn!(
                        "Additional chunk error after the first (which is what surfaces as \
                         the download failure): {e}"
                    );
                }
            }
        }
    }

    (items, first_err)
}

/// Sort chunk results by id and split into ordered paths + total bytes
/// downloaded. Shared by the adaptive and static paths, which otherwise
/// duplicated this sort-then-split.
fn into_ordered_paths<I: Ord + Copy>(mut results: Vec<(I, PathBuf, u64)>) -> (Vec<PathBuf>, u64) {
    results.sort_by_key(|(id, _, _)| *id);
    let total_bytes: u64 = results.iter().map(|(_, _, b)| b).sum();
    let chunk_paths: Vec<PathBuf> = results.into_iter().map(|(_, path, _)| path).collect();
    (chunk_paths, total_bytes)
}

/// Byte-range plan for a static (non-adaptive) chunked download: chunk size
/// and count are computed once from `size_to_download` and the configured
/// [`ChunkSizeStrategy`].
struct StaticChunkPlan {
    chunk_size: usize,
    total_chunks: usize,
    size_to_download: u64,
    byte_offset: u64,
}

impl StaticChunkPlan {
    fn new(size_to_download: u64, byte_offset: u64, strategy: ChunkSizeStrategy) -> Self {
        let (chunk_size, total_chunks) = calculate_chunks(size_to_download, strategy);
        Self {
            chunk_size,
            total_chunks,
            size_to_download,
            byte_offset,
        }
    }

    /// Inclusive byte range `(start, end)` for `chunk_id` (0-based), already
    /// shifted by `byte_offset`. The last chunk absorbs any remainder, so
    /// `size_to_download` need not be a multiple of `chunk_size`.
    const fn range_of(&self, chunk_id: usize) -> (u64, u64) {
        let start = self.byte_offset + (chunk_id as u64 * self.chunk_size as u64);
        let end = if chunk_id == self.total_chunks - 1 {
            self.byte_offset + self.size_to_download - 1
        } else {
            start + self.chunk_size as u64 - 1
        };
        (start, end)
    }
}

/// Confirm the assembled output is exactly the size the server advertised.
///
/// Last-resort integrity gate for #526, deliberately independent of the
/// per-chunk validation in `download_range_with_progress`: that layer checks
/// each response against what was requested, while this one checks the
/// finished artifact against the resource's advertised length. A defect in
/// chunk bookkeeping — a dropped, duplicated, or misordered chunk — leaves the
/// per-chunk checks satisfied but the assembly wrong, and only shows up here.
///
/// A size match is not a proof of correctness (#526 produced a full-length file
/// with displaced interior bytes), so this complements the per-chunk checks
/// rather than replacing them.
pub(crate) async fn verify_merged_size(path: &Path, expected_total: u64, url: &str) -> Result<()> {
    let actual = tokio::fs::metadata(path)
        .await
        .map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to stat merged output '{}' for size verification: {e}",
                    path.display()
                ),
            ))
        })?
        .len();

    if actual != expected_total {
        return Err(RdlpError::Download {
            url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
            message: format!(
                "assembled output '{}' is {actual} bytes but the server advertised \
                 {expected_total}; the download is incomplete or misassembled.",
                path.display()
            ),
        });
    }

    Ok(())
}

/// Merge or append chunks to file using an explicit ordered list of paths.
async fn merge_chunks_ordered(
    path: &Path,
    chunk_paths: &[PathBuf],
    config: &DownloaderConfig,
    mode: MergeMode,
) -> Result<()> {
    let merge_timeout = config.merge_timeout;
    let action = mode.log_action();

    tokio::time::timeout(merge_timeout, async {
        let mut writer = open_merge_sink(path, mode, config.buffer_size).await?;

        debug!(chunks = chunk_paths.len(); "{action} chunks into file");
        let mut deleted_chunks = 0;
        let total = chunk_paths.len();
        for (idx, chunk_path) in chunk_paths.iter().enumerate() {
            if drain_chunk_into(&mut writer, chunk_path).await? {
                deleted_chunks += 1;
            }

            if (idx + 1) % 100 == 0 || idx == total - 1 {
                debug!(processed = idx + 1, total; "{action} progress");
            }
        }
        debug!(deleted = deleted_chunks; "Chunk cleanup complete");

        writer.flush().await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to flush merged output file '{}': {e}",
                    path.display()
                ),
            ))
        })?;
        Ok(())
    })
    .await
    .map_err(|_| RdlpError::Download {
        message: format!("Merge timed out after {}s", merge_timeout.as_secs()),
        url: None,
    })?
}

/// Open the merge output file per `mode`, wrapped in a buffered writer sized
/// to `buffer_size`.
async fn open_merge_sink(
    path: &Path,
    mode: MergeMode,
    buffer_size: usize,
) -> Result<BufWriter<File>> {
    let file = match mode {
        MergeMode::Create => File::create(path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to create output file '{}': {e}", path.display()),
            ))
        })?,
        MergeMode::Append => tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to open output file for append '{}': {e}",
                        path.display()
                    ),
                ))
            })?,
    };
    Ok(BufWriter::with_capacity(buffer_size, file))
}

/// Copy one chunk file's contents into `writer`, then delete the chunk file.
///
/// Returns whether the chunk file was actually deleted by this call: a
/// `NotFound` on removal (the file is already gone, the desired end state)
/// or any other deletion failure (logged at `warn!`, matching the severity
/// this merge loop always used) both count as "not deleted" without failing
/// the merge — only a failure to open or copy the chunk aborts it.
async fn drain_chunk_into(writer: &mut BufWriter<File>, chunk_path: &Path) -> Result<bool> {
    let mut chunk_file = File::open(chunk_path).await.map_err(|e| {
        RdlpError::Io(std::io::Error::new(
            e.kind(),
            format!("failed to open chunk file '{}': {e}", chunk_path.display()),
        ))
    })?;
    tokio::io::copy(&mut chunk_file, writer)
        .await
        .map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to copy chunk '{}' into output: {e}",
                    chunk_path.display()
                ),
            ))
        })?;

    match tokio::fs::remove_file(chunk_path).await {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => {
            warn!(path:? = chunk_path; "Failed to delete chunk file: {e}");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download_err(msg: &str) -> RdlpError {
        RdlpError::Download {
            message: msg.to_string(),
            url: None,
        }
    }

    // ── drain_collecting_first_error ────────────────────────────────────────

    #[tokio::test]
    async fn drain_collecting_first_error_returns_all_items_when_all_ok() {
        let items: Vec<Result<u32>> = vec![Ok(1), Ok(2), Ok(3)];
        let flag = AtomicBool::new(false);

        let (collected, first_err) = drain_collecting_first_error(stream::iter(items), &flag).await;

        assert_eq!(collected, vec![1, 2, 3]);
        assert!(first_err.is_none());
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn drain_collecting_first_error_returns_first_of_two_errors() {
        let items: Vec<Result<u32>> = vec![
            Ok(1),
            Err(download_err("first")),
            Err(download_err("second")),
        ];
        let flag = AtomicBool::new(false);

        let (_collected, first_err) =
            drain_collecting_first_error(stream::iter(items), &flag).await;

        let e = first_err.expect("an error must surface");
        assert_eq!(format!("{e}"), format!("{}", download_err("first")));
    }

    #[tokio::test]
    async fn drain_collecting_first_error_sets_stop_scheduling_flag() {
        let items: Vec<Result<u32>> = vec![Err(download_err("boom"))];
        let flag = AtomicBool::new(false);

        let _ = drain_collecting_first_error(stream::iter(items), &flag).await;

        assert!(flag.load(Ordering::Relaxed));
    }

    /// #568 regression guard: the drain used to be a plain `try_collect`,
    /// which drops the stream (and abandons any in-flight work) the instant
    /// the first error surfaces. Items the underlying stream yields AFTER
    /// that first error represent real, already-dispatched work and MUST
    /// still be collected — simplifying this function back to `try_collect`
    /// silently resurrects that leak.
    #[tokio::test]
    async fn drain_collecting_first_error_still_collects_items_yielded_after_first_error() {
        let items: Vec<Result<u32>> = vec![Ok(1), Err(download_err("boom")), Ok(2), Ok(3)];
        let flag = AtomicBool::new(false);

        let (collected, first_err) = drain_collecting_first_error(stream::iter(items), &flag).await;

        assert_eq!(collected, vec![1, 2, 3]);
        assert!(first_err.is_some());
    }

    // ── StaticChunkPlan ──────────────────────────────────────────────────────

    #[test]
    fn static_chunk_plan_range_of_first_chunk() {
        let plan = StaticChunkPlan::new(10 * 1024 * 1024, 0, ChunkSizeStrategy::Fixed(1024 * 1024));
        assert_eq!(plan.range_of(0), (0, 1024 * 1024 - 1));
    }

    #[test]
    fn static_chunk_plan_range_of_interior_chunk() {
        let plan = StaticChunkPlan::new(10 * 1024 * 1024, 0, ChunkSizeStrategy::Fixed(1024 * 1024));
        assert_eq!(plan.range_of(5), (5 * 1024 * 1024, 6 * 1024 * 1024 - 1));
    }

    #[test]
    fn static_chunk_plan_range_of_last_chunk() {
        let plan = StaticChunkPlan::new(10 * 1024 * 1024, 0, ChunkSizeStrategy::Fixed(1024 * 1024));
        assert_eq!(plan.total_chunks, 10);
        assert_eq!(plan.range_of(9), (9 * 1024 * 1024, 10 * 1024 * 1024 - 1));
    }

    #[test]
    fn static_chunk_plan_single_chunk_download() {
        let plan = StaticChunkPlan::new(500 * 1024, 0, ChunkSizeStrategy::Fixed(1024 * 1024));
        assert_eq!(plan.total_chunks, 1);
        assert_eq!(plan.range_of(0), (0, 500 * 1024 - 1));
    }

    #[test]
    fn static_chunk_plan_non_multiple_of_chunk_size_absorbs_remainder_in_last_chunk() {
        let size = 10 * 1024 * 1024 + 500;
        let plan = StaticChunkPlan::new(size, 0, ChunkSizeStrategy::Fixed(1024 * 1024));
        assert_eq!(plan.total_chunks, 11);
        let last = plan.total_chunks - 1;
        assert_eq!(plan.range_of(last), (last as u64 * 1024 * 1024, size - 1));
    }

    #[test]
    fn static_chunk_plan_range_of_respects_byte_offset_for_resume() {
        let plan = StaticChunkPlan::new(
            10 * 1024 * 1024,
            5 * 1024 * 1024,
            ChunkSizeStrategy::Fixed(1024 * 1024),
        );
        assert_eq!(plan.range_of(0), (5 * 1024 * 1024, 6 * 1024 * 1024 - 1));
    }

    // ── build_manifest_tracker / legacy chunk sets ──────────────────────────

    /// A legacy `ChunkSet` (no `download_id`) has no manifest to write to —
    /// `build_manifest_tracker` must fall out to a disabled tracker
    /// structurally (via `ChunkSet::manifest` returning `None`), not panic.
    /// The disabled tracker's `record_and_save` must be a genuine no-op:
    /// no file written, for any path this attempt might otherwise have used.
    #[tokio::test]
    async fn legacy_chunk_set_yields_a_disabled_manifest_tracker() {
        let temp_dir = tempfile::tempdir().unwrap();
        let chunk_set = ChunkSet::legacy("video.mp4").unwrap();
        assert!(
            chunk_set.manifest(temp_dir.path()).is_none(),
            "a legacy set has no download_id to key a manifest on"
        );

        let layout = ChunkLayout {
            chunk_set: &chunk_set,
            temp_dir: temp_dir.path(),
        };
        let tracking = ChunkTracking::new(
            build_manifest_tracker(&layout, Attempt::Fresh, 100),
            Tallies::new(),
        );
        tracking.manifest.record_and_save(0, 100).await;

        let mut entries = tokio::fs::read_dir(temp_dir.path()).await.unwrap();
        assert!(
            entries.next_entry().await.unwrap().is_none(),
            "a disabled tracker must never write a manifest file"
        );
    }

    // ── Attempt ──────────────────────────────────────────────────────────────

    #[test]
    fn attempt_fresh_derives_create_and_zero_offset() {
        let attempt = Attempt::Fresh;
        assert_eq!(attempt.byte_offset(), 0);
        assert_eq!(attempt.chunk_kind(), ChunkKind::Fresh);
        assert!(matches!(attempt.merge_mode(), MergeMode::Create));
    }

    #[test]
    fn attempt_resume_derives_append_and_from_offset() {
        let attempt = Attempt::Resume { from: 4096 };
        assert_eq!(attempt.byte_offset(), 4096);
        assert_eq!(attempt.chunk_kind(), ChunkKind::Resume);
        assert!(matches!(attempt.merge_mode(), MergeMode::Append));
    }

    // ── into_ordered_paths ───────────────────────────────────────────────────

    #[test]
    fn into_ordered_paths_sorts_by_id_and_sums_bytes() {
        let results = vec![
            (2u64, PathBuf::from("c"), 30u64),
            (0u64, PathBuf::from("a"), 10u64),
            (1u64, PathBuf::from("b"), 20u64),
        ];

        let (paths, total) = into_ordered_paths(results);

        assert_eq!(
            paths,
            vec![PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c")]
        );
        assert_eq!(total, 60);
    }

    // ── write-side manifest wiring (#675, spec review finding 4) ────────────
    //
    // Both tests call `download_parallel_static`/`_adaptive` DIRECTLY rather
    // than through the public `download_to_file` — the public path deletes
    // the manifest on success (via the caller, after `verify_merged_size`),
    // which would make the manifest unreadable by the time a black-box test
    // could inspect it. Calling the private methods directly leaves the
    // manifest on disk for these tests to load back and assert on, exactly
    // as recorded, without duplicating a second read of `tracking.manifest`'s
    // interior lock.

    /// One 256 KiB chunk per id — shared by both tests below so their
    /// expected `completed` map is identical regardless of path.
    const WIRING_TEST_CHUNK_BYTES: u64 = 256 * 1024;
    const WIRING_TEST_CHUNK_COUNT: u64 = 3;

    async fn mock_three_chunks(server: &mut mockito::ServerGuard, total_size: u64) {
        for id in 0..WIRING_TEST_CHUNK_COUNT {
            let start = id * WIRING_TEST_CHUNK_BYTES;
            let end = start + WIRING_TEST_CHUNK_BYTES - 1;
            server
                .mock("GET", "/video.mp4")
                .match_header("range", format!("bytes={start}-{end}").as_str())
                .with_status(206)
                .with_header(
                    "content-range",
                    &format!("bytes {start}-{end}/{total_size}"),
                )
                .with_body(vec![id as u8; WIRING_TEST_CHUNK_BYTES as usize])
                .create_async()
                .await;
        }
    }

    #[tokio::test]
    async fn static_download_records_every_chunk_length_in_the_manifest() {
        use mockito::Server;
        use tempfile::TempDir;

        let mut server = Server::new_async().await;
        let temp_dir = TempDir::new().unwrap();
        let total_size = WIRING_TEST_CHUNK_BYTES * WIRING_TEST_CHUNK_COUNT;
        mock_three_chunks(&mut server, total_size).await;

        let url = format!("{}/video.mp4", server.url());
        let downloader = HttpDownloader::new()
            .with_concurrent_fragments(3)
            .with_chunk_strategy(ChunkSizeStrategy::Legacy {
                chunk_count: WIRING_TEST_CHUNK_COUNT as usize,
            });
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let chunk_set = ChunkSet::for_attempt("video.mp4", download_id, ChunkKind::Fresh).unwrap();
        let span = TransferSpan {
            url: &url,
            offset: Attempt::Fresh.byte_offset(),
            len: total_size,
        };
        let layout = ChunkLayout {
            chunk_set: &chunk_set,
            temp_dir: temp_dir.path(),
        };
        let tracking = ChunkTracking::new(
            build_manifest_tracker(&layout, Attempt::Fresh, span.len),
            Tallies::new(),
        );

        let (chunk_paths, total_downloaded) = downloader
            .download_parallel_static(ParallelRun {
                span,
                layout,
                tracking: &tracking,
            })
            .await
            .expect("static download must succeed");
        assert_eq!(chunk_paths.len(), WIRING_TEST_CHUNK_COUNT as usize);
        assert_eq!(total_downloaded, total_size);

        let manifest_path = chunk_set.manifest_path_in(temp_dir.path()).unwrap();
        let manifest = ChunkManifest::load_matching(&manifest_path, download_id, ChunkKind::Fresh)
            .await
            .expect(
                "manifest must be readable after a successful static download — RED with \
                 `record_and_save` removed from the static per-chunk closure",
            );
        for id in 0..WIRING_TEST_CHUNK_COUNT {
            assert_eq!(
                manifest.recorded_len(id),
                Some(WIRING_TEST_CHUNK_BYTES),
                "chunk {id} must be recorded at its actual length"
            );
        }
    }

    #[tokio::test]
    async fn adaptive_download_records_every_chunk_length_in_the_manifest() {
        use mockito::Server;
        use tempfile::TempDir;

        let mut server = Server::new_async().await;
        let temp_dir = TempDir::new().unwrap();
        let total_size = WIRING_TEST_CHUNK_BYTES * WIRING_TEST_CHUNK_COUNT;
        mock_three_chunks(&mut server, total_size).await;

        let url = format!("{}/video.mp4", server.url());
        let downloader = HttpDownloader::new()
            .with_concurrent_fragments(3)
            .with_adaptive(true);
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let chunk_set = ChunkSet::for_attempt("video.mp4", download_id, ChunkKind::Fresh).unwrap();
        let span = TransferSpan {
            url: &url,
            offset: Attempt::Fresh.byte_offset(),
            len: total_size,
        };
        let layout = ChunkLayout {
            chunk_set: &chunk_set,
            temp_dir: temp_dir.path(),
        };
        let tracking = ChunkTracking::new(
            build_manifest_tracker(&layout, Attempt::Fresh, span.len),
            Tallies::new(),
        );

        let (chunk_paths, total_downloaded) = downloader
            .download_parallel_adaptive(
                ParallelRun {
                    span,
                    layout,
                    tracking: &tracking,
                },
                None,
                None,
            )
            .await
            .expect("adaptive download must succeed");
        assert_eq!(chunk_paths.len(), WIRING_TEST_CHUNK_COUNT as usize);
        assert_eq!(total_downloaded, total_size);

        let manifest_path = chunk_set.manifest_path_in(temp_dir.path()).unwrap();
        let manifest = ChunkManifest::load_matching(&manifest_path, download_id, ChunkKind::Fresh)
            .await
            .expect(
                "manifest must be readable after a successful adaptive download — RED with \
                 `record_and_save` removed from `run_adaptive_chunk`",
            );
        for id in 0..WIRING_TEST_CHUNK_COUNT {
            assert_eq!(
                manifest.recorded_len(id),
                Some(WIRING_TEST_CHUNK_BYTES),
                "chunk {id} must be recorded at its actual length"
            );
        }
    }
}
