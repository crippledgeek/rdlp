//! Parallel download functionality
//!
//! Provides parallel download using multiple range requests with fine-grained chunking.

use futures::stream::{self, StreamExt, TryStreamExt};
use log::{debug, error, info};
use rdlp_core::{DownloadProgress, DownloadStats, RdlpError, Result};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use super::HttpDownloader;
use super::config::DownloaderConfig;
use crate::chunking::calculate_chunks;

/// Global atomic counter for generating unique download IDs
pub(super) static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// RAII guard for progress reporter tasks
///
/// Ensures the progress reporter task is aborted when the guard goes out of scope,
/// preventing task leaks on early returns or errors.
pub(super) struct ProgressGuard(pub Option<tokio::task::JoinHandle<()>>);

impl ProgressGuard {
    pub fn new(task: Option<tokio::task::JoinHandle<()>>) -> Self {
        Self(task)
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// Cleanup chunk files on error
pub(super) async fn cleanup_chunk_files(
    temp_dir: &Path,
    filename: &str,
    download_id: u64,
    total_chunks: usize,
) {
    info!(chunks = total_chunks; "Cleaning up partial chunk files");
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
    /// Parallel download using multiple range requests with fine-grained chunking
    pub(super) async fn download_parallel(
        &self,
        url: &str,
        path: &Path,
        total_size: u64,
        progress: Option<Box<dyn rdlp_core::ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let (chunk_size, total_chunks) = calculate_chunks(total_size, self.config.chunk_strategy);

        debug!(
            download_id,
            size_mb = total_size / 1024 / 1024,
            chunk_kb = chunk_size / 1024,
            chunks = total_chunks,
            concurrent = self.config.concurrent_fragments,
            batches = total_chunks.div_ceil(self.config.concurrent_fragments);
            "Chunk analysis"
        );

        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let filename = path
            .file_name()
            .ok_or_else(|| RdlpError::Download("Invalid output path: no filename".to_string()))?
            .to_string_lossy()
            .to_string();

        let downloaded = Arc::new(AtomicU64::new(0));

        let _progress_guard = ProgressGuard::new(create_progress_reporter(
            progress,
            downloaded.clone(),
            start_time,
            total_size,
            0,
        ));

        info!(
            "Starting parallel download with {} concurrent connections",
            self.config.concurrent_fragments
        );

        let url_shared: Arc<str> = Arc::from(url);

        let chunk_results: Vec<u64> = stream::iter(0..total_chunks)
            .map(|chunk_id| {
                let start = chunk_id as u64 * chunk_size as u64;
                let end = if chunk_id == total_chunks - 1 {
                    total_size - 1
                } else {
                    start + chunk_size as u64 - 1
                };

                let chunk_path =
                    temp_dir.join(format!("{}.{}.part{}", &filename, download_id, chunk_id));
                let downloader = self.clone();
                let url = Arc::clone(&url_shared);
                let progress_counter = Some(downloaded.clone());

                async move {
                    let result = downloader
                        .download_range_with_progress(
                            &url,
                            start,
                            end,
                            &chunk_path,
                            progress_counter,
                        )
                        .await;
                    if let Err(ref e) = result {
                        error!("Chunk {chunk_id} failed: {e}");
                    }
                    result.map(|bytes| (chunk_id, bytes, chunk_path))
                }
            })
            .buffer_unordered(self.config.concurrent_fragments)
            .try_collect::<Vec<_>>()
            .await
            .inspect_err(|e| {
                error!("Download failed: {e}");
                let temp_dir = temp_dir.to_path_buf();
                let filename = filename.clone();
                tokio::spawn(async move {
                    cleanup_chunk_files(&temp_dir, &filename, download_id, total_chunks).await;
                });
            })?
            .into_iter()
            .map(|(_, bytes, _)| bytes)
            .collect();

        let total_downloaded: u64 = chunk_results.iter().sum();
        info!(
            "All {} chunks completed ({} MB total)",
            total_chunks,
            total_downloaded / 1024 / 1024
        );

        merge_chunks(
            path,
            temp_dir,
            &filename,
            download_id,
            total_chunks,
            &self.config,
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

        Ok(stats)
    }

    /// Parallel resume: downloads remaining chunks in parallel
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
        let (chunk_size, total_chunks) =
            calculate_chunks(remaining_size, self.config.chunk_strategy);

        debug!(
            "Parallel resume: download_id={download_id}, already={} MB ({:.1}%), remaining={} MB, chunk_size={} KB, chunks={total_chunks}, concurrent={}",
            resume_from / 1024 / 1024,
            (resume_from as f64 / total_size as f64) * 100.0,
            remaining_size / 1024 / 1024,
            chunk_size / 1024,
            self.config.concurrent_fragments
        );

        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let filename = path
            .file_name()
            .ok_or_else(|| RdlpError::Download("Invalid output path: no filename".to_string()))?
            .to_string_lossy()
            .to_string();

        let downloaded = Arc::new(AtomicU64::new(resume_from));

        let progress_task = create_progress_reporter(
            progress,
            downloaded.clone(),
            start_time,
            total_size,
            resume_from,
        );

        info!(
            "Starting parallel download with {} concurrent connections",
            self.config.concurrent_fragments
        );

        let url_shared: Arc<str> = Arc::from(url);

        let chunk_results: Vec<u64> = stream::iter(0..total_chunks)
            .map(|chunk_id| {
                let start = resume_from + (chunk_id as u64 * chunk_size as u64);
                let end = if chunk_id == total_chunks - 1 {
                    total_size - 1
                } else {
                    start + chunk_size as u64 - 1
                };

                let chunk_path =
                    temp_dir.join(format!("{}.{}.resume{}", &filename, download_id, chunk_id));
                let downloader = self.clone();
                let url = Arc::clone(&url_shared);
                let progress_counter = Some(downloaded.clone());

                async move {
                    let result = downloader
                        .download_range_with_progress(
                            &url,
                            start,
                            end,
                            &chunk_path,
                            progress_counter,
                        )
                        .await;
                    if let Err(ref e) = result {
                        error!("Chunk {chunk_id} failed: {e}");
                    }
                    result.map(|bytes| (chunk_id, bytes, chunk_path))
                }
            })
            .buffer_unordered(self.config.concurrent_fragments)
            .try_collect::<Vec<_>>()
            .await
            .inspect_err(|e| {
                error!("Resume failed: {e}");
                if let Some(task) = &progress_task {
                    task.abort();
                }
                let temp_dir = temp_dir.to_path_buf();
                let filename = filename.clone();
                tokio::spawn(async move {
                    cleanup_chunk_files(&temp_dir, &filename, download_id, total_chunks).await;
                });
            })?
            .into_iter()
            .map(|(_, bytes, _)| bytes)
            .collect();

        let newly_downloaded: u64 = chunk_results.iter().sum();
        info!(
            "All {} chunks completed ({} MB new data)",
            total_chunks,
            newly_downloaded / 1024 / 1024
        );

        if let Some(task) = progress_task {
            task.abort();
        }

        // Append chunks to existing file
        append_chunks(
            path,
            temp_dir,
            &filename,
            download_id,
            total_chunks,
            &self.config,
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
}

/// Create a progress reporter task
fn create_progress_reporter(
    callback: Option<Box<dyn rdlp_core::ProgressCallback>>,
    downloaded: Arc<AtomicU64>,
    start_time: Instant,
    total_size: u64,
    resume_from: u64,
) -> Option<tokio::task::JoinHandle<()>> {
    callback.map(|cb| {
        tokio::spawn(async move {
            let mut last_update = Instant::now();
            let update_interval = Duration::from_millis(100);

            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let now = Instant::now();
                if now.duration_since(last_update) >= update_interval {
                    let bytes_downloaded = downloaded.load(Ordering::Relaxed);
                    let elapsed = now.duration_since(start_time).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        (bytes_downloaded - resume_from) as f64 / elapsed
                    } else {
                        0.0
                    };

                    let progress_info =
                        DownloadProgress::new(bytes_downloaded, Some(total_size), speed);
                    cb.on_progress(&progress_info);
                    last_update = now;

                    if bytes_downloaded >= total_size {
                        break;
                    }
                }
            }
        })
    })
}

/// Merge chunks into final file
async fn merge_chunks(
    path: &Path,
    temp_dir: &Path,
    filename: &str,
    download_id: u64,
    total_chunks: usize,
    config: &DownloaderConfig,
) -> Result<()> {
    info!(chunks = total_chunks; "Merging chunks into final file");
    let final_file = File::create(path).await.map_err(RdlpError::Io)?;
    let mut writer = BufWriter::with_capacity(config.buffer_size, final_file);

    let mut deleted_chunks = 0;
    for chunk_id in 0..total_chunks {
        let chunk_path = temp_dir.join(format!("{filename}.{download_id}.part{chunk_id}"));
        let mut chunk_file = File::open(&chunk_path).await.map_err(RdlpError::Io)?;
        tokio::io::copy(&mut chunk_file, &mut writer)
            .await
            .map_err(RdlpError::Io)?;

        if tokio::fs::remove_file(&chunk_path).await.is_ok() {
            deleted_chunks += 1;
        }

        if (chunk_id + 1) % 100 == 0 || chunk_id == total_chunks - 1 {
            debug!(merged = chunk_id + 1, total = total_chunks; "Merge progress");
        }
    }
    debug!(deleted = deleted_chunks; "Chunk cleanup complete");

    writer.flush().await.map_err(RdlpError::Io)?;
    Ok(())
}

/// Append chunks to existing file (for resume)
async fn append_chunks(
    path: &Path,
    temp_dir: &Path,
    filename: &str,
    download_id: u64,
    total_chunks: usize,
    config: &DownloaderConfig,
) -> Result<()> {
    let file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await
        .map_err(RdlpError::Io)?;
    let mut writer = BufWriter::with_capacity(config.buffer_size, file);

    info!(chunks = total_chunks; "Appending chunks to existing file");
    let mut deleted_chunks = 0;
    for chunk_id in 0..total_chunks {
        let chunk_path = temp_dir.join(format!("{filename}.{download_id}.resume{chunk_id}"));
        let mut chunk_file = File::open(&chunk_path).await.map_err(RdlpError::Io)?;
        tokio::io::copy(&mut chunk_file, &mut writer)
            .await
            .map_err(RdlpError::Io)?;

        if tokio::fs::remove_file(&chunk_path).await.is_ok() {
            deleted_chunks += 1;
        }

        if (chunk_id + 1) % 100 == 0 || chunk_id == total_chunks - 1 {
            debug!(appended = chunk_id + 1, total = total_chunks; "Append progress");
        }
    }
    debug!(deleted = deleted_chunks; "Chunk cleanup complete");

    writer.flush().await.map_err(RdlpError::Io)?;
    Ok(())
}
