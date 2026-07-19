//! Parallel download functionality
//!
//! Provides parallel download using multiple range requests with fine-grained chunking.

use super::HttpDownloader;
use super::chunk_name::{ChunkKind, ChunkSet};
use super::config::DownloaderConfig;
use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};
use crate::chunking::calculate_chunks;
use crate::progress::{ProgressMetrics, ProgressReporterConfig, spawn_progress_reporter};
use futures::stream::{self, StreamExt, TryStreamExt};
use log::{debug, error, info, warn};
use rdlp_core::{DownloadStats, ProgressCallback, RdlpError, Result, is_retryable_error};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

/// Maximum number of retry attempts for a single chunk download.
///
/// Covers body-stream failures (decode errors, read timeouts mid-transfer)
/// that occur after the initial HTTP connection succeeds. The inner
/// `download_range_with_progress` handles connection-level retries separately.
const MAX_CHUNK_RETRIES: u32 = 3;

/// Download a single chunk with retry for transient body-stream failures.
///
/// Wraps `download_range_with_progress` in a retry loop with linear backoff
/// (1s, 2s, 3s). Partial files are cleaned up between attempts. Only
/// retryable errors (5xx, 429, network, I/O) trigger retries.
///
/// `cancel` — when `Some`, cancellation is checked at the top of every loop
/// iteration and forwarded into `download_range_with_progress`. The backoff
/// sleep is also raced against the token so cancellation fires immediately.
///
/// This is a different retry domain from the inner `with_retry` on
/// `send()` — this handles failures that occur *during* body transfer,
/// not connection-level failures.
#[allow(
    clippy::too_many_arguments,
    reason = "Each argument carries a distinct semantic role (downloader, url, byte range, sink path, progress counter, chunk identifier, cancel token); extracting a params struct would obscure the call sites inside the AIMD unfold closure without removing complexity."
)]
pub async fn download_chunk_with_retry(
    downloader: &HttpDownloader,
    url: &str,
    start: u64,
    end: u64,
    chunk_path: &Path,
    progress: Option<Arc<AtomicU64>>,
    chunk_id: u64,
    cancel: Option<&CancellationToken>,
) -> Result<u64> {
    let mut retries = 0u32;

    loop {
        // Pre-iteration cancel guard.
        if let Some(token) = cancel
            && token.is_cancelled()
        {
            let _ = tokio::fs::remove_file(chunk_path).await;
            return Err(RdlpError::Cancelled);
        }

        match downloader
            .download_range_with_progress(url, start, end, chunk_path, progress.clone(), cancel)
            .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(RdlpError::Cancelled) => {
                let _ = tokio::fs::remove_file(chunk_path).await;
                return Err(RdlpError::Cancelled);
            }
            Err(e) => {
                if retries < MAX_CHUNK_RETRIES && is_retryable_error(&e) {
                    retries += 1;
                    warn!(
                        "Chunk {chunk_id} failed (attempt {}/{}): {e}",
                        retries,
                        MAX_CHUNK_RETRIES + 1
                    );
                    // Clean up partial file before retry
                    let _ = tokio::fs::remove_file(chunk_path).await;
                    // Race the backoff sleep against cancel so we don't block.
                    if let Some(token) = cancel {
                        tokio::select! {
                            biased;
                            () = token.cancelled() => {
                                return Err(RdlpError::Cancelled);
                            }
                            () = tokio::time::sleep(Duration::from_secs(u64::from(retries))) => {}
                        }
                    } else {
                        tokio::time::sleep(Duration::from_secs(u64::from(retries))).await;
                    }
                } else {
                    return Err(e);
                }
            }
        }
    }
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

/// Global atomic counter for generating unique download IDs
static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Sweep every chunk file `chunk_set` claims in `temp_dir`, logging the
/// outcome without letting a sweep failure mask `original_err` — the download
/// error is what the caller must surface; a cleanup failure is secondary.
async fn sweep_after_failure(chunk_set: &ChunkSet, temp_dir: &Path, original_err: &RdlpError) {
    match chunk_set.sweep(temp_dir).await {
        Ok(report) => {
            debug!(deleted = report.deleted; "Swept chunk files after failed download: {original_err}");
        }
        Err(sweep_err) => {
            warn!(
                "Failed to sweep chunk files after failed download ({original_err}): {sweep_err:#}"
            );
        }
    }
}

impl HttpDownloader {
    /// Parallel download using multiple range requests with fine-grained chunking.
    ///
    /// When `config.adaptive` is `true`, chunk sizes and connection counts are
    /// dynamically tuned by an AIMD controller. Otherwise the static
    /// `chunk_strategy` is used with a fixed connection count.
    pub(super) async fn download_parallel(
        &self,
        url: &str,
        path: &Path,
        total_size: u64,
        progress: Option<Box<dyn rdlp_core::ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let filename = path
            .file_name()
            .ok_or_else(|| RdlpError::Download {
                message: "Invalid output path: no filename".to_string(),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
            })?
            .to_string_lossy()
            .into_owned();

        let downloaded = Arc::new(AtomicU64::new(0));
        let progress: Option<Arc<dyn rdlp_core::ProgressCallback>> = progress.map(Arc::from);
        let _progress_guard = spawn_progress_reporter(
            progress.clone(),
            ProgressMetrics::bytes_only(downloaded.clone()),
            ProgressReporterConfig::http(start_time, total_size, 0),
        );

        let (chunk_paths, total_downloaded) = if self.config.adaptive {
            self.download_parallel_adaptive(
                url,
                total_size,
                download_id,
                temp_dir,
                &filename,
                downloaded.clone(),
                0,
                ChunkKind::Fresh,
                progress.clone(),
                None,
            )
            .await?
        } else {
            self.download_parallel_static(
                url,
                total_size,
                download_id,
                temp_dir,
                &filename,
                downloaded.clone(),
                0,
                ChunkKind::Fresh,
            )
            .await?
        };

        let chunk_count = chunk_paths.len();

        merge_chunks_ordered(
            path,
            temp_dir,
            &filename,
            download_id,
            &chunk_paths,
            &self.config,
            MergeMode::Create,
        )
        .await?;

        // Backstop (#526): the assembly must match the advertised length before
        // this file is handed back as a completed download.
        verify_merged_size(path, total_size, url).await?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(total_downloaded, duration, 0);

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
        url: &str,
        path: &Path,
        resume_from: u64,
        total_size: u64,
        progress: Option<Box<dyn rdlp_core::ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();
        let remaining_size = total_size - resume_from;
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let filename = path
            .file_name()
            .ok_or_else(|| RdlpError::Download {
                message: "Invalid output path: no filename".to_string(),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
            })?
            .to_string_lossy()
            .into_owned();

        let downloaded = Arc::new(AtomicU64::new(resume_from));
        let progress: Option<Arc<dyn rdlp_core::ProgressCallback>> = progress.map(Arc::from);
        let mut progress_guard = spawn_progress_reporter(
            progress.clone(),
            ProgressMetrics::bytes_only(downloaded.clone()),
            ProgressReporterConfig::http(start_time, total_size, resume_from),
        );

        debug!(
            "Parallel resume: download_id={download_id}, already={} MB ({:.1}%), remaining={} MB, concurrent={}",
            resume_from / 1024 / 1024,
            (resume_from as f64 / total_size as f64) * 100.0,
            remaining_size / 1024 / 1024,
            self.config.concurrent_fragments
        );

        let result = if self.config.adaptive {
            self.download_parallel_adaptive(
                url,
                remaining_size,
                download_id,
                temp_dir,
                &filename,
                downloaded.clone(),
                resume_from,
                ChunkKind::Resume,
                progress.clone(),
                None,
            )
            .await
        } else {
            self.download_parallel_static(
                url,
                remaining_size,
                download_id,
                temp_dir,
                &filename,
                downloaded.clone(),
                resume_from,
                ChunkKind::Resume,
            )
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

        merge_chunks_ordered(
            path,
            temp_dir,
            &filename,
            download_id,
            &chunk_paths,
            &self.config,
            MergeMode::Append,
        )
        .await?;

        // Backstop (#526). On the resume path this also catches a `resume_from`
        // that disagreed with the file's real length: the appended bytes land
        // at EOF regardless of the offset the ranges were requested from, so a
        // mismatch shows up here as a wrong final size.
        verify_merged_size(path, total_size, url).await?;

        let duration = start_time.elapsed();
        let total_downloaded = resume_from + newly_downloaded;
        let stats = DownloadStats::new(total_downloaded, duration, 0);

        info!(
            "Resume complete: {} MB total, {} MB new in {:.1}s ({:.1} MB/s)",
            total_downloaded / 1024 / 1024,
            newly_downloaded / 1024 / 1024,
            duration.as_secs_f64(),
            (newly_downloaded as f64 / duration.as_secs_f64()) / 1024.0 / 1024.0
        );

        Ok(stats)
    }

    /// Adaptive download: uses `AdaptiveController` for dynamic chunk sizing and concurrency.
    ///
    /// Returns `(chunk_paths_in_order, total_bytes_downloaded)`.
    /// `byte_offset` is added to every chunk's start position (for resume).
    ///
    /// `cancel` is `Option<CancellationToken>` (owned, not a reference) so it can be
    /// cloned into the `try_unfold` closure. `CancellationToken` is cheaply cloneable.
    #[allow(clippy::too_many_arguments)]
    async fn download_parallel_adaptive(
        &self,
        url: &str,
        size_to_download: u64,
        download_id: u64,
        temp_dir: &Path,
        filename: &str,
        progress_counter: Arc<AtomicU64>,
        byte_offset: u64,
        kind: ChunkKind,
        log_callback: Option<Arc<dyn ProgressCallback>>,
        cancel: Option<CancellationToken>,
    ) -> Result<(Vec<PathBuf>, u64)> {
        let chunk_set = ChunkSet::for_attempt(filename, download_id, kind).map_err(|e| {
            RdlpError::Download {
                message: format!("{e:#}"),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
            }
        })?;

        let controller = Arc::new(AdaptiveController::new(
            size_to_download,
            AdaptiveConfig {
                max_connections: self.config.concurrent_fragments,
                ..AdaptiveConfig::default()
            },
            ControllerMode::HttpChunked,
            log_callback,
        ));

        let sem = controller.semaphore().clone();

        // Chunk paths collected in assignment order; we re-sort by chunk_id before merge.
        // Each entry is (chunk_id, PathBuf, bytes_downloaded).
        let url_shared: Arc<str> = Arc::from(url);

        // Generate tasks lazily using stream::unfold driven by controller.next_chunk().
        // buffer_unordered with a generous buffer lets the semaphore control actual concurrency.
        let buffer_factor = self.config.concurrent_fragments * 2;

        // Set once a sibling chunk fails, so the unfold generator stops
        // scheduling NEW chunks. It deliberately does NOT cancel chunks
        // already in flight (see below) — only gates further generation.
        let stop_scheduling = Arc::new(AtomicBool::new(false));

        let buffered = {
            let downloader = self.clone();
            let url_arc = url_shared.clone();
            let progress = progress_counter.clone();
            let temp_dir_owned = temp_dir.to_path_buf();
            let chunk_set_owned = chunk_set.clone();
            // Clone once outside the closure; each iteration re-clones from this.
            let cancel_outer = cancel.clone();
            let stop_flag = stop_scheduling.clone();

            stream::try_unfold((controller.clone(), 0u64), move |(ctrl, chunk_id)| {
                let sem = sem.clone();
                let downloader = downloader.clone();
                let url = url_arc.clone();
                let progress = progress.clone();
                let ctrl_report = ctrl.clone();
                let chunk_path = chunk_set_owned.path_in(&temp_dir_owned, chunk_id);
                let ctrl_next = ctrl.clone();
                let cancel_for_unfold = cancel_outer.clone();
                let stop_flag = stop_flag.clone();

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

                    let cancel_for_chunk = cancel_for_unfold.clone();

                    let download_fut = async move {
                        // F6 (#308): race semaphore acquisition against cancel so
                        // a pending permit-wait unblocks immediately on cancellation.
                        let _permit = if let Some(ref token) = cancel_for_chunk {
                            tokio::select! {
                                biased;
                                () = token.cancelled() => return Err(RdlpError::Cancelled),
                                p = sem.acquire_owned() => p.map_err(|_| RdlpError::Download {
                                    message: "Semaphore closed".to_string(),
                                    url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
                                })?,
                            }
                        } else {
                            sem.acquire_owned().await.map_err(|_| RdlpError::Download {
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
                            &url,
                            abs_start,
                            abs_end,
                            &chunk_path,
                            Some(progress),
                            chunk_id,
                            cancel_for_chunk.as_ref(),
                        )
                        .await;

                        match &result {
                            Ok(bytes) => {
                                ctrl_report.report_chunk_complete(*bytes, start_time.elapsed());
                            }
                            Err(e) => {
                                error!("Adaptive chunk {chunk_id} failed after retries: {e}");
                            }
                        }

                        result.map(|bytes| (chunk_id, chunk_path, bytes))
                    };

                    Ok(Some((download_fut, (ctrl_next, chunk_id + 1))))
                }
            })
            .try_buffer_unordered(buffer_factor)
        };
        futures::pin_mut!(buffered);

        // Drain every buffered future to completion rather than dropping the
        // stream on the first error (`try_collect`'s default): dropping the
        // `TryBufferUnordered` mid-poll abandons any chunk still in flight,
        // and `tokio::fs` operations dispatched to the blocking pool keep
        // running in the background after their driving future is dropped
        // (tokio::fs docs) — sweeping immediately would race those writes and
        // could delete a file the background write hasn't produced yet, or
        // leave a file created just after the sweep ran. Setting
        // `stop_scheduling` (checked above) halts new chunk generation
        // immediately so this only waits out the already-buffered futures,
        // not the whole remaining transfer.
        let mut results: Vec<(u64, PathBuf, u64)> = Vec::new();
        let mut first_err: Option<RdlpError> = None;
        while let Some(item) = buffered.next().await {
            match item {
                Ok(v) => results.push(v),
                Err(e) => {
                    if first_err.is_none() {
                        stop_scheduling.store(true, Ordering::Relaxed);
                        first_err = Some(e);
                    }
                }
            }
        }

        if let Some(e) = first_err {
            sweep_after_failure(&chunk_set, temp_dir, &e).await;
            return Err(e);
        }

        // Sort by chunk_id to ensure correct merge order.
        let mut sorted = results;
        sorted.sort_by_key(|(id, _, _)| *id);

        let total_bytes: u64 = sorted.iter().map(|(_, _, b)| b).sum();
        let chunk_paths: Vec<PathBuf> = sorted.into_iter().map(|(_, path, _)| path).collect();

        Ok((chunk_paths, total_bytes))
    }

    /// Static download: uses a fixed chunk size and connection count.
    ///
    /// Returns `(chunk_paths_in_order, total_bytes_downloaded)`.
    #[allow(clippy::too_many_arguments)]
    async fn download_parallel_static(
        &self,
        url: &str,
        size_to_download: u64,
        download_id: u64,
        temp_dir: &Path,
        filename: &str,
        progress_counter: Arc<AtomicU64>,
        byte_offset: u64,
        kind: ChunkKind,
    ) -> Result<(Vec<PathBuf>, u64)> {
        let chunk_set = ChunkSet::for_attempt(filename, download_id, kind).map_err(|e| {
            RdlpError::Download {
                message: format!("{e:#}"),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
            }
        })?;

        let (chunk_size, total_chunks) =
            calculate_chunks(size_to_download, self.config.chunk_strategy);

        debug!(
            download_id,
            size_mb = size_to_download / 1024 / 1024,
            chunk_kb = chunk_size / 1024,
            chunks = total_chunks,
            concurrent = self.config.concurrent_fragments;
            "Static chunk analysis"
        );

        let url_shared: Arc<str> = Arc::from(url);
        let temp_dir_owned = temp_dir.to_path_buf();

        let results: Vec<(usize, PathBuf, u64)> = match stream::iter(0..total_chunks)
            .map(|chunk_id| {
                let start = byte_offset + (chunk_id as u64 * chunk_size as u64);
                let end = if chunk_id == total_chunks - 1 {
                    byte_offset + size_to_download - 1
                } else {
                    start + chunk_size as u64 - 1
                };

                let chunk_path = chunk_set.path_in(&temp_dir_owned, chunk_id as u64);
                let downloader = self.clone();
                let url = Arc::clone(&url_shared);
                let progress = Some(progress_counter.clone());

                async move {
                    let result = download_chunk_with_retry(
                        &downloader,
                        &url,
                        start,
                        end,
                        &chunk_path,
                        progress,
                        chunk_id as u64,
                        None,
                    )
                    .await;
                    if let Err(ref e) = result {
                        error!("Chunk {chunk_id} failed after retries: {e}");
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
                sweep_after_failure(&chunk_set, temp_dir, &e).await;
                return Err(e);
            }
        };

        let total_bytes: u64 = results.iter().map(|(_, _, b)| b).sum();

        // Results arrive in completion order; sort by chunk_id for correct merge.
        let mut sorted = results;
        sorted.sort_by_key(|(id, _, _)| *id);
        let chunk_paths: Vec<PathBuf> = sorted.into_iter().map(|(_, path, _)| path).collect();

        Ok((chunk_paths, total_bytes))
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
    _temp_dir: &Path,
    _filename: &str,
    _download_id: u64,
    chunk_paths: &[PathBuf],
    config: &DownloaderConfig,
    mode: MergeMode,
) -> Result<()> {
    let merge_timeout = config.merge_timeout;
    let action = mode.log_action();

    tokio::time::timeout(merge_timeout, async {
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
        let mut writer = BufWriter::with_capacity(config.buffer_size, file);

        debug!(chunks = chunk_paths.len(); "{action} chunks into file");
        let mut deleted_chunks = 0;
        let total = chunk_paths.len();
        for (idx, chunk_path) in chunk_paths.iter().enumerate() {
            let mut chunk_file = File::open(chunk_path).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!("failed to open chunk file '{}': {e}", chunk_path.display()),
                ))
            })?;
            tokio::io::copy(&mut chunk_file, &mut writer)
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
                Ok(()) => deleted_chunks += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!(path:? = chunk_path; "Failed to delete chunk file: {e}"),
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
