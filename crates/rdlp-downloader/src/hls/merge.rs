//! HLS segment merging and cleanup.
//!
//! std::sync::Mutex is intentional for `Arc<Mutex<HlsDownloadState>>`: guards
//! never cross an .await point. Snapshots are cloned out of the lock before
//! any `.save().await` call, and all critical sections are pure sync
//! (HashSet edits, counter updates, clone-out-for-save).
//! See docs/implementation/tls-impersonation/phase-1-report.md Finding 2.2.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use log::{debug, warn};
use rdlp_core::{ProgressCallback, RdlpError, Result, RetryConfig};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::instrument;

use super::segment::download_segment_with_retry;
use super::types::SegmentInfo;
use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};
use crate::hls_state::HlsDownloadState;
use crate::http::HttpDownloader;

/// Download segments with resume support
///
/// Skips segments that are already completed (tracked in state).
/// Updates state after each successful segment download.
/// Saves state periodically for crash recovery.
///
/// # Arguments
/// * `http_downloader` - HTTP downloader to use
/// * `retry_config` - Retry configuration
/// * `buffer_size` - Buffer size for writing
/// * `concurrent_segments` - Number of concurrent segment downloads
/// * `max_segment_failures` - Maximum failures before aborting
/// * `segments` - List of segments with URLs and durations
/// * `temp_dir` - Directory to save temporary segment files
/// * `base_filename` - Base filename for temporary files
/// * `progress_counter` - Shared atomic counter for bytes downloaded
/// * `segments_counter` - Shared atomic counter for segments completed
/// * `duration_counter` - Shared atomic counter for duration completed (in centiseconds for precision)
/// * `state` - Shared download state for resume tracking
/// * `output_path` - Final output path (for state file location)
///
/// # Returns
/// * `Ok(Vec<PathBuf>)` - Paths to ALL segment files (in order, including pre-existing)
/// * `Err(_)` - Download error (network, I/O, etc.)
#[allow(clippy::too_many_arguments)]
#[instrument(skip(http_downloader, retry_config, segments, progress_counter, segments_counter, duration_counter, state, log_callback), fields(segments = segments.len()))]
pub(crate) async fn download_segments_with_resume(
    http_downloader: &HttpDownloader,
    retry_config: Arc<RetryConfig>,
    buffer_size: usize,
    concurrent_segments: usize,
    max_segment_failures: usize,
    segments: &[SegmentInfo],
    temp_dir: &Path,
    base_filename: &str,
    progress_counter: Arc<AtomicU64>,
    segments_counter: Arc<AtomicU64>,
    duration_counter: Arc<AtomicU64>,
    state: Arc<Mutex<HlsDownloadState>>,
    output_path: &Path,
    log_callback: Option<Arc<dyn ProgressCallback>>,
) -> Result<Vec<PathBuf>> {
    let total_segments = segments.len();

    // Get already completed segments and validate they exist on disk
    let original_completed: HashSet<usize> = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .completed_segments
        .clone();

    // Verify completed segments actually exist on disk (handles corrupted/deleted files)
    let is_valid_on_disk = |idx: &usize| -> bool {
        let segment_path = temp_dir.join(format!("{base_filename}.part{idx}"));
        // Safe: HLS segment-merge path runs inside spawn_blocking; no async runtime active on this thread.
        #[allow(clippy::disallowed_methods)]
        std::fs::metadata(&segment_path)
            .map(|meta| meta.len() > 0)
            .unwrap_or(false)
    };
    let completed: HashSet<usize> = original_completed
        .iter()
        .copied()
        .filter(is_valid_on_disk)
        .collect();
    let mut missing_segments: Vec<usize> = original_completed
        .iter()
        .copied()
        .filter(|idx| !completed.contains(idx))
        .collect();

    if !missing_segments.is_empty() {
        missing_segments.sort_unstable();
        warn!(
            count = missing_segments.len(),
            segments:? = missing_segments;
            "State claimed segments completed but files missing/empty, re-downloading"
        );
        // Update state to remove invalid entries
        {
            let mut state_guard = state.lock().unwrap_or_else(|e| e.into_inner());
            for idx in &missing_segments {
                state_guard.completed_segments.remove(idx);
            }
        }
    }

    let to_download: Vec<(usize, SegmentInfo)> = segments
        .iter()
        .enumerate()
        .filter(|(idx, _)| !completed.contains(idx))
        .map(|(idx, seg)| (idx, seg.clone()))
        .collect();

    // Calculate duration already downloaded from completed segments
    let completed_duration: f64 = segments
        .iter()
        .enumerate()
        .filter(|(idx, _)| completed.contains(idx))
        .map(|(_, seg)| seg.duration)
        .sum();
    duration_counter.store((completed_duration * 100.0) as u64, Ordering::Relaxed);

    let already_downloaded = completed.len();
    let remaining = to_download.len();

    if remaining == 0 {
        debug!(total = total_segments; "All segments already downloaded, skipping to merge");
    } else {
        debug!(
            remaining,
            completed = already_downloaded,
            concurrent = concurrent_segments;
            "Downloading HLS segments"
        );
    }

    // Download remaining segments using adaptive controller with semaphore-based concurrency.
    let controller = Arc::new(AdaptiveController::new(
        0, // total_size not meaningful for HLS (segment count drives iteration)
        AdaptiveConfig {
            max_connections: concurrent_segments,
            ..AdaptiveConfig::default()
        },
        ControllerMode::HlsSegments,
        log_callback,
    ));
    let sem = controller.semaphore().clone();

    let http_downloader = http_downloader.clone();
    let temp_dir_owned = temp_dir.to_path_buf();
    let base_filename_owned = base_filename.to_string();
    let output_path_owned = output_path.to_path_buf();

    let mut stream = stream::iter(to_download.into_iter())
        .map(|(idx, seg)| {
            let segment_path = temp_dir_owned.join(format!("{base_filename_owned}.part{idx}"));
            let http_downloader = http_downloader.clone();
            let retry_config = retry_config.clone();
            let progress = progress_counter.clone();
            let segments = segments_counter.clone();
            let duration = duration_counter.clone();
            let state = state.clone();
            let output_path = output_path_owned.clone();
            let seg_duration = seg.duration;
            let seg_url = seg.url;
            let sem = sem.clone();
            let controller = controller.clone();

            async move {
                // Check if segment file already exists and is non-empty (single async stat)
                if let Ok(meta) = tokio::fs::metadata(&segment_path).await
                    && meta.len() > 0
                {
                    debug!(
                        segment = idx,
                        bytes = meta.len();
                        "Segment already exists, skipping"
                    );
                    let bytes = meta.len();
                    // Mark as completed in state
                    {
                        let mut state_guard = state.lock().unwrap_or_else(|e| e.into_inner());
                        state_guard.mark_completed(idx, bytes);
                    }
                    segments.fetch_add(1, Ordering::Relaxed);
                    progress.fetch_add(bytes, Ordering::Relaxed);
                    duration.fetch_add((seg_duration * 100.0) as u64, Ordering::Relaxed);
                    return Ok((idx, segment_path, bytes));
                }

                // Acquire semaphore permit before starting the download so that
                // the adaptive controller governs the actual concurrency level.
                let _permit = sem.acquire_owned().await.map_err(|_| RdlpError::Download {
                    message: "Semaphore closed".to_string(),
                    url: Some(seg_url.clone()),
                })?;

                let download_start = Instant::now();

                // Download segment with retry logic
                let result = download_segment_with_retry(
                    &http_downloader,
                    &retry_config,
                    buffer_size,
                    idx,
                    seg_url,
                    segment_path.clone(),
                    progress.clone(),
                )
                .await;

                match &result {
                    Ok((_, _, bytes)) => {
                        // Report to adaptive controller (permit still held — released on drop)
                        controller.report_segment_complete(
                            *bytes,
                            download_start.elapsed(),
                            Some(seg_duration),
                        );

                        // Update state on success; clone if periodic save needed
                        let snapshot = {
                            let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
                            guard.mark_completed(idx, *bytes);
                            if guard.completed_segments.len().is_multiple_of(50) {
                                Some(guard.clone())
                            } else {
                                None
                            }
                        };
                        // Save outside the lock to avoid holding it across I/O
                        if let Some(snapshot) = snapshot
                            && let Err(e) = snapshot.save(&output_path).await
                        {
                            warn!("Failed to save HLS state: {e}");
                        }

                        segments.fetch_add(1, Ordering::Relaxed);
                        duration.fetch_add((seg_duration * 100.0) as u64, Ordering::Relaxed);
                    }
                    Err(e) => {
                        // Save state on error before propagating
                        let snapshot = state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        if let Err(save_err) = snapshot.save(&output_path).await {
                            warn!("Failed to save HLS state on error: {save_err}");
                        }
                        warn!(segment = idx; "Segment download failed: {e}");
                    }
                }

                result
            }
        })
        .buffer_unordered(concurrent_segments * 2);

    // Collect results, tolerating up to max_segment_failures errors
    let mut results: Vec<(usize, PathBuf, u64)> = Vec::new();
    let mut segment_failures = 0usize;

    while let Some(result) = stream.next().await {
        match result {
            Ok(item) => results.push(item),
            Err(e) => {
                segment_failures += 1;
                warn!(
                    failures = segment_failures,
                    max = max_segment_failures;
                    "Segment failed: {e}"
                );
                if segment_failures >= max_segment_failures {
                    let snapshot = state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let _ = snapshot.save(output_path).await;
                    return Err(RdlpError::Download {
                        message: format!(
                            "Too many segment failures ({segment_failures}), aborting"
                        ),
                        url: None,
                    });
                }
            }
        }
    }
    if segment_failures > 0 {
        warn!(failures = segment_failures; "Completed with segment failures");
    }

    // Save final state (clone under lock, then save outside lock)
    let snapshot = state.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Err(e) = snapshot.save(output_path).await {
        warn!("Failed to save final HLS state: {e}");
    }

    // Build complete list of segment paths (including pre-existing)
    let mut all_segment_paths: Vec<(usize, PathBuf)> = Vec::with_capacity(total_segments);

    // Add downloaded segments from this run
    all_segment_paths.extend(results.into_iter().map(|(idx, path, _)| (idx, path)));

    // Add pre-existing segments
    all_segment_paths.extend(completed.into_iter().filter_map(|idx| {
        let segment_path = temp_dir.join(format!("{base_filename}.part{idx}"));
        segment_path.exists().then_some((idx, segment_path))
    }));

    // Sort by index for correct merge order
    all_segment_paths.sort_by_key(|(idx, _)| *idx);

    // Verify we have the expected number of segments (accounting for tolerated failures)
    let expected_count = total_segments - segment_failures;
    let actual_count = all_segment_paths.len();
    if actual_count != expected_count {
        return Err(RdlpError::Download {
            message: format!("Missing segments: expected {expected_count}, got {actual_count}"),
            url: None,
        });
    }

    let segment_paths: Vec<PathBuf> = all_segment_paths
        .into_iter()
        .map(|(_, path)| path)
        .collect();

    debug!(total = total_segments; "All segments ready for merge");
    Ok(segment_paths)
}

/// Merge segment files into final output file.
///
/// For fMP4 streams, each segment may reference an init segment (EXT-X-MAP).
/// The init segment is written before the first media segment that uses it,
/// and re-written whenever the init segment changes (supporting playlists
/// with multiple EXT-X-MAP tags).
///
/// `segment_init_paths[i]` is the init segment file for `segment_paths[i]`,
/// or `None` for plain TS segments.
pub(crate) async fn merge_segments(
    buffer_size: usize,
    merge_timeout: Duration,
    segment_paths: &[PathBuf],
    output_path: &Path,
    segment_init_paths: &[Option<PathBuf>],
) -> Result<u64> {
    tokio::time::timeout(merge_timeout, async {
        let has_init = segment_init_paths.iter().any(|p| p.is_some());
        debug!(
            segments = segment_paths.len(),
            fmp4 = has_init;
            "Merging segments into final file"
        );

        let final_file = File::create(output_path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to create HLS output file '{}': {e}",
                    output_path.display()
                ),
            ))
        })?;
        let mut writer = BufWriter::with_capacity(buffer_size, final_file);
        let mut total_bytes = 0u64;

        // Track the current init segment so we only re-write it on change
        let mut current_init: Option<&Path> = None;

        for (idx, segment_path) in segment_paths.iter().enumerate() {
            // Check if this segment needs an init segment (different from current)
            let seg_init = segment_init_paths.get(idx).and_then(|p| p.as_deref());

            if seg_init != current_init {
                if let Some(init_path) = seg_init {
                    let mut init_file = File::open(init_path).await.map_err(|e| {
                        RdlpError::Io(std::io::Error::new(
                            e.kind(),
                            format!(
                                "failed to open init segment file '{}': {e}",
                                init_path.display()
                            ),
                        ))
                    })?;
                    let bytes =
                        tokio::io::copy(&mut init_file, &mut writer)
                            .await
                            .map_err(|e| {
                                RdlpError::Io(std::io::Error::new(
                                    e.kind(),
                                    format!(
                                        "failed to copy init segment '{}' to output: {e}",
                                        init_path.display()
                                    ),
                                ))
                            })?;
                    total_bytes += bytes;
                    debug!(bytes, segment = idx; "Wrote fMP4 init segment");
                }
                current_init = seg_init;
            }

            let mut segment_file = File::open(segment_path).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to open segment file '{}': {e}",
                        segment_path.display()
                    ),
                ))
            })?;
            let bytes = tokio::io::copy(&mut segment_file, &mut writer)
                .await
                .map_err(|e| {
                    RdlpError::Io(std::io::Error::new(
                        e.kind(),
                        format!(
                            "failed to copy segment '{}' to output: {e}",
                            segment_path.display()
                        ),
                    ))
                })?;
            total_bytes += bytes;

            if (idx + 1) % 100 == 0 || idx == segment_paths.len() - 1 {
                debug!(
                    merged = idx + 1,
                    total = segment_paths.len(),
                    mb = total_bytes / (1024 * 1024);
                    "Merge progress"
                );
            }
        }

        writer.flush().await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to flush HLS output file '{}': {e}",
                    output_path.display()
                ),
            ))
        })?;
        debug!(mb = total_bytes / (1024 * 1024); "Merge complete");

        Ok(total_bytes)
    })
    .await
    .map_err(|_| RdlpError::Download {
        message: format!("Merge timed out after {}s", merge_timeout.as_secs()),
        url: None,
    })?
}

/// Clean up temporary segment files
///
/// Deletes all temporary segment files after successful merge.
/// Logs deletion progress for transparency.
///
/// # Arguments
/// * `segment_paths` - Paths to segment files to delete
pub(crate) async fn cleanup_segments(segment_paths: &[PathBuf]) {
    debug!(count = segment_paths.len(); "Cleaning up segment files");

    let mut deleted = 0;
    for path in segment_paths {
        match tokio::fs::remove_file(path).await {
            Ok(()) => deleted += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(path:? = path; "Failed to delete segment file: {e}"),
        }
    }

    debug!(deleted; "Segment cleanup complete");
}
