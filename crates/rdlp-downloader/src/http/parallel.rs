//! Parallel download functionality
//!
//! Provides parallel download using multiple range requests with fine-grained chunking.

use super::HttpDownloader;
use super::config::DownloaderConfig;
use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};
use crate::chunking::calculate_chunks;
use crate::progress::{ProgressMetrics, ProgressReporterConfig, spawn_progress_reporter};
use futures::stream::{self, StreamExt, TryStreamExt};
use log::{debug, error, info};
use rdlp_core::{DownloadStats, ProgressCallback, RdlpError, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Mode for chunk merging operations
#[derive(Clone, Copy)]
enum MergeMode {
    /// Create new file and merge chunks (.part suffix)
    Create,
    /// Append chunks to existing file (.resume suffix)
    Append,
}

impl MergeMode {
    fn log_action(self) -> &'static str {
        match self {
            MergeMode::Create => "Merging",
            MergeMode::Append => "Appending",
        }
    }
}

/// Global atomic counter for generating unique download IDs
static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Cleanup chunk files by index range (used when chunk list is not available)
async fn cleanup_chunk_files(
    temp_dir: &Path,
    filename: &str,
    download_id: u64,
    total_chunks: usize,
) {
    debug!(chunks = total_chunks; "Cleaning up partial chunk files");
    let mut deleted = 0;
    for chunk_id in 0..total_chunks {
        let chunk_path = temp_dir.join(format!("{filename}.{download_id}.part{chunk_id}"));
        if tokio::fs::remove_file(&chunk_path).await.is_ok() {
            deleted += 1;
        }
    }
    debug!(deleted; "Chunk cleanup complete");
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
            .ok_or_else(|| RdlpError::Download("Invalid output path: no filename".to_string()))?
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
                "part",
                progress.clone(),
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
                "part",
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
            .ok_or_else(|| RdlpError::Download("Invalid output path: no filename".to_string()))?
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
                "resume",
                progress.clone(),
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
                "resume",
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
        suffix: &str,
        log_callback: Option<Arc<dyn ProgressCallback>>,
    ) -> Result<(Vec<PathBuf>, u64)> {
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

        let results: Vec<(u64, PathBuf, u64)> = {
            let downloader = self.clone();
            let url_arc = url_shared.clone();
            let progress = progress_counter.clone();
            let temp_dir_owned = temp_dir.to_path_buf();
            let filename_owned = filename.to_string();
            let suffix_owned = suffix.to_string();

            stream::try_unfold((controller.clone(), 0u64), move |(ctrl, chunk_id)| {
                let sem = sem.clone();
                let downloader = downloader.clone();
                let url = url_arc.clone();
                let progress = progress.clone();
                let ctrl_report = ctrl.clone();
                let chunk_path = temp_dir_owned.join(format!(
                    "{}.{}.{}{}",
                    filename_owned, download_id, suffix_owned, chunk_id
                ));
                let ctrl_next = ctrl.clone();

                async move {
                    let chunk = match ctrl.next_chunk() {
                        Some(c) => c,
                        None => return Ok(None),
                    };

                    let download_fut = async move {
                        let _permit = sem
                            .acquire_owned()
                            .await
                            .map_err(|_| RdlpError::Download("Semaphore closed".to_string()))?;
                        let start_time = Instant::now();
                        let abs_start = byte_offset + chunk.start;
                        // end is exclusive in ChunkRequest, but HTTP Range is inclusive
                        let abs_end = byte_offset + chunk.end - 1;

                        let result = downloader
                            .download_range_with_progress(
                                &url,
                                abs_start,
                                abs_end,
                                &chunk_path,
                                Some(progress),
                            )
                            .await;

                        match &result {
                            Ok(bytes) => {
                                ctrl_report.report_chunk_complete(*bytes, start_time.elapsed());
                            }
                            Err(e) => {
                                error!("Adaptive chunk {chunk_id} failed: {e}");
                            }
                        }

                        result.map(|bytes| (chunk_id, chunk_path, bytes))
                    };

                    Ok(Some((download_fut, (ctrl_next, chunk_id + 1))))
                }
            })
            .try_buffer_unordered(buffer_factor)
            .try_collect()
            .await?
        };

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
        suffix: &str,
    ) -> Result<(Vec<PathBuf>, u64)> {
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
        let filename_owned = filename.to_string();
        let suffix_owned = suffix.to_string();

        let results: Vec<(usize, PathBuf, u64)> = match stream::iter(0..total_chunks)
            .map(|chunk_id| {
                let start = byte_offset + (chunk_id as u64 * chunk_size as u64);
                let end = if chunk_id == total_chunks - 1 {
                    byte_offset + size_to_download - 1
                } else {
                    start + chunk_size as u64 - 1
                };

                let chunk_path = temp_dir_owned.join(format!(
                    "{}.{}.{}{}",
                    filename_owned, download_id, suffix_owned, chunk_id
                ));
                let downloader = self.clone();
                let url = Arc::clone(&url_shared);
                let progress = Some(progress_counter.clone());

                async move {
                    let result = downloader
                        .download_range_with_progress(&url, start, end, &chunk_path, progress)
                        .await;
                    if let Err(ref e) = result {
                        error!("Chunk {chunk_id} failed: {e}");
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
                cleanup_chunk_files(temp_dir, filename, download_id, total_chunks).await;
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
            MergeMode::Create => File::create(path).await.map_err(RdlpError::Io)?,
            MergeMode::Append => tokio::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .await
                .map_err(RdlpError::Io)?,
        };
        let mut writer = BufWriter::with_capacity(config.buffer_size, file);

        debug!(chunks = chunk_paths.len(); "{action} chunks into file");
        let mut deleted_chunks = 0;
        let total = chunk_paths.len();
        for (idx, chunk_path) in chunk_paths.iter().enumerate() {
            let mut chunk_file = File::open(chunk_path).await.map_err(RdlpError::Io)?;
            tokio::io::copy(&mut chunk_file, &mut writer)
                .await
                .map_err(RdlpError::Io)?;

            if tokio::fs::remove_file(chunk_path).await.is_ok() {
                deleted_chunks += 1;
            }

            if (idx + 1) % 100 == 0 || idx == total - 1 {
                debug!(processed = idx + 1, total; "{action} progress");
            }
        }
        debug!(deleted = deleted_chunks; "Chunk cleanup complete");

        writer.flush().await.map_err(RdlpError::Io)?;
        Ok(())
    })
    .await
    .map_err(|_| {
        RdlpError::Download(format!(
            "Merge timed out after {}s",
            merge_timeout.as_secs()
        ))
    })?
}
