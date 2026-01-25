use async_trait::async_trait;
use backon::Retryable;
use futures::stream::{self, StreamExt, TryStreamExt};
use log::{debug, info, warn};
use tracing::instrument;
use rdlp_core::{
    DownloadProgress, DownloadStats, Downloader, ProgressCallback, RdlpError, Result, RetryConfig,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

use crate::hls_state::HlsDownloadState;
use crate::http::HttpDownloader;

/// HLS (HTTP Live Streaming) downloader
///
/// Downloads HLS streams by parsing m3u8 playlists and downloading segments
/// in parallel using the HTTP downloader infrastructure.
///
/// # Architecture
///
/// 1. Parse m3u8 playlist to extract segment URLs
/// 2. Download segments in parallel using `buffer_unordered`
/// 3. Merge segments into final video file
/// 4. Clean up temporary segment files
///
/// # Performance
///
/// - Default: 8 concurrent segment downloads
/// - Typical: 500 MB video in 60-90 seconds
/// - Bottleneck: Server throttling (not client)
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_downloader::HlsDownloader;
/// use rdlp_core::Downloader;
/// use std::path::Path;
///
/// # async fn example() -> rdlp_core::Result<()> {
/// let downloader = HlsDownloader::new();
/// downloader.download_to_file(
///     "https://example.com/playlist.m3u8",
///     Path::new("video.mp4"),
///     None
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct HlsDownloader {
    http_downloader: HttpDownloader,
    concurrent_segments: usize,
    buffer_size: usize,
    retry_config: Arc<RetryConfig>,
    /// Expected total size for progress reporting (set externally)
    expected_size: Option<u64>,
}

impl HlsDownloader {
    /// Create a new HLS downloader with default settings
    pub fn new() -> Self {
        Self {
            http_downloader: HttpDownloader::new(),
            concurrent_segments: 8,       // Default: 8 parallel segments
            buffer_size: 2 * 1024 * 1024, // 2 MB buffer for merging
            retry_config: Arc::new(RetryConfig::default_config()),
            expected_size: None,
        }
    }

    /// Set the HTTP downloader to use
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_http_downloader(mut self, http: HttpDownloader) -> Self {
        self.http_downloader = http;
        self
    }

    /// Set number of concurrent segment downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_concurrent_segments(mut self, count: usize) -> Self {
        self.concurrent_segments = count.max(1);
        self
    }

    /// Set buffer size for segment merging
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Set retry configuration
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = Arc::new(config);
        self
    }

    /// Set expected total size for progress reporting
    ///
    /// This allows the progress bar to show accurate percentage and ETA
    /// even though HLS streams don't have a known total size upfront.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_expected_size(mut self, size: u64) -> Self {
        self.expected_size = Some(size);
        self
    }

    /// Download a single HLS segment with retry logic using backon
    ///
    /// Handles network errors, timeouts, and expired URLs by retrying with exponential backoff.
    ///
    /// # Arguments
    /// * `idx` - Segment index (for logging)
    /// * `url` - Segment URL
    /// * `segment_path` - Path to save segment
    /// * `progress` - Shared progress counter
    ///
    /// # Returns
    /// * `Ok((index, path, bytes))` - Successfully downloaded segment
    /// * `Err(_)` - Failed after all retries
    #[instrument(skip(self, progress), fields(segment = idx))]
    async fn download_segment_with_retry(
        &self,
        idx: usize,
        url: String,
        segment_path: PathBuf,
        progress: Arc<AtomicU64>,
    ) -> Result<(usize, PathBuf, u64)> {
        let http_client = self.http_downloader.client().clone();
        let buffer_size = self.buffer_size;
        let backoff = self.retry_config.to_backoff();

        // Use backon for retry with exponential backoff and jitter
        let result = (|| {
            let client = http_client.clone();
            let url = url.clone();
            let segment_path = segment_path.clone();
            let progress = progress.clone();

            async move {
                // Download segment to file
                let response = client
                    .get(&url)
                    .timeout(Duration::from_secs(30)) // 30 second timeout per segment
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            RdlpError::Network(format!("Segment {idx} timeout"))
                        } else if e.is_connect() {
                            RdlpError::Network(format!("Segment {idx} connection failed"))
                        } else {
                            RdlpError::Network(format!("Segment {idx} request failed: {e}"))
                        }
                    })?;

                if !response.status().is_success() {
                    return Err(RdlpError::Network(format!(
                        "Segment {} returned HTTP {}",
                        idx,
                        response.status()
                    )));
                }

                // Stream segment to file with progress tracking
                let file = File::create(&segment_path).await.map_err(RdlpError::Io)?;
                let mut writer = BufWriter::with_capacity(buffer_size, file);
                let mut stream = response.bytes_stream();
                let mut downloaded = 0u64;

                while let Some(chunk_result) = stream.next().await {
                    let chunk = chunk_result
                        .map_err(|e| RdlpError::Network(format!("Segment {idx} read error: {e}")))?;

                    writer.write_all(&chunk).await.map_err(RdlpError::Io)?;
                    downloaded += chunk.len() as u64;

                    // Update shared progress counter (lock-free atomic)
                    progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                }

                writer.flush().await.map_err(RdlpError::Io)?;

                Ok((idx, segment_path, downloaded))
            }
        })
        .retry(backoff)
        .when(|e: &RdlpError| {
            // Retry on network errors, not on permanent failures
            matches!(e, RdlpError::Network(_) | RdlpError::Io(_))
        })
        .notify(|err, dur| {
            warn!(segment = idx, delay:? = dur; "Segment download failed, retrying: {err}");
        })
        .await?;

        Ok(result)
    }

    /// Parse m3u8 playlist and extract segment URLs
    ///
    /// Handles both media playlists (direct segments) and master playlists
    /// (redirects to best variant). Uses recursive parsing for master playlists.
    ///
    /// # Arguments
    /// * `m3u8_url` - URL of the m3u8 playlist
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of segment URLs
    /// * `Err(_)` - Network error, parse error, or empty playlist
    async fn parse_playlist(&self, m3u8_url: &str) -> Result<Vec<String>> {
        // Fetch playlist text
        let playlist_text = self
            .http_downloader
            .client()
            .get(m3u8_url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch playlist: {e}")))?
            .text()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to read playlist: {e}")))?;

        // Parse with m3u8-rs
        let playlist = m3u8_rs::parse_playlist_res(playlist_text.as_bytes())
            .map_err(|e| RdlpError::Extraction(format!("M3U8 parse error: {e:?}")))?;

        match playlist {
            m3u8_rs::Playlist::MediaPlaylist(media) => {
                // Direct media playlist - extract segments
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

                let segments: Vec<String> = media
                    .segments
                    .iter()
                    .map(|seg| {
                        // Join relative URLs with base URL
                        base_url
                            .join(&seg.uri)
                            .map(|u| u.to_string())
                            .unwrap_or_else(|_| seg.uri.clone())
                    })
                    .collect();

                if segments.is_empty() {
                    return Err(RdlpError::Extraction("Playlist has no segments".into()));
                }

                // Security check: limit max segments
                const MAX_SEGMENTS: usize = 10_000;
                if segments.len() > MAX_SEGMENTS {
                    return Err(RdlpError::Extraction(format!(
                        "Playlist has too many segments: {} (max: {})",
                        segments.len(),
                        MAX_SEGMENTS
                    )));
                }

                Ok(segments)
            }
            m3u8_rs::Playlist::MasterPlaylist(master) => {
                // Master playlist - select first variant
                if master.variants.is_empty() {
                    return Err(RdlpError::Extraction(
                        "Master playlist has no variants".into(),
                    ));
                }

                let variant = &master.variants[0];
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

                let media_playlist_url = base_url
                    .join(&variant.uri)
                    .map_err(|e| RdlpError::Extraction(format!("Failed to join URL: {e}")))?
                    .to_string();

                debug!(
                    variant:? = variant.uri,
                    bandwidth = variant.bandwidth;
                    "Master playlist detected, selecting variant"
                );

                // Recursively parse media playlist
                Box::pin(self.parse_playlist(&media_playlist_url)).await
            }
        }
    }

    /// Download segments with resume support
    ///
    /// Skips segments that are already completed (tracked in state).
    /// Updates state after each successful segment download.
    /// Saves state periodically for crash recovery.
    ///
    /// # Arguments
    /// * `segment_urls` - List of segment URLs to download
    /// * `temp_dir` - Directory to save temporary segment files
    /// * `base_filename` - Base filename for temporary files
    /// * `progress_counter` - Shared atomic counter for bytes downloaded
    /// * `segments_counter` - Shared atomic counter for segments completed
    /// * `state` - Shared download state for resume tracking
    /// * `output_path` - Final output path (for state file location)
    ///
    /// # Returns
    /// * `Ok(Vec<PathBuf>)` - Paths to ALL segment files (in order, including pre-existing)
    /// * `Err(_)` - Download error (network, I/O, etc.)
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip(self, segment_urls, progress_counter, segments_counter, state), fields(segments = segment_urls.len()))]
    async fn download_segments_with_resume(
        &self,
        segment_urls: Vec<String>,
        temp_dir: &Path,
        base_filename: &str,
        progress_counter: Arc<AtomicU64>,
        segments_counter: Arc<AtomicU64>,
        state: Arc<Mutex<HlsDownloadState>>,
        output_path: &Path,
    ) -> Result<Vec<PathBuf>> {
        let total_segments = segment_urls.len();

        // Get already completed segments
        let completed: HashSet<usize> = state.lock().await.completed_segments.clone();
        let to_download: Vec<(usize, String)> = segment_urls
            .iter()
            .enumerate()
            .filter(|(idx, _)| !completed.contains(idx))
            .map(|(idx, url)| (idx, url.clone()))
            .collect();

        let already_downloaded = completed.len();
        let remaining = to_download.len();
        let concurrent = self.concurrent_segments;

        if remaining == 0 {
            info!(total = total_segments; "All segments already downloaded, skipping to merge");
        } else {
            info!(
                remaining,
                completed = already_downloaded,
                concurrent;
                "Downloading HLS segments"
            );
        }

        // Download remaining segments using buffer_unordered with state tracking
        let downloader = self.clone();
        let temp_dir_owned = temp_dir.to_path_buf();
        let base_filename_owned = base_filename.to_string();
        let output_path_owned = output_path.to_path_buf();

        let results: Vec<(usize, PathBuf, u64)> = stream::iter(to_download.into_iter())
            .map(|(idx, url)| {
                let segment_path = temp_dir_owned.join(format!("{base_filename_owned}.part{idx}"));
                let downloader = downloader.clone();
                let progress = progress_counter.clone();
                let segments = segments_counter.clone();
                let state = state.clone();
                let output_path = output_path_owned.clone();

                async move {
                    // Check if segment file already exists and is non-empty
                    if segment_path.exists() {
                        if let Ok(meta) = tokio::fs::metadata(&segment_path).await {
                            if meta.len() > 0 {
                                debug!(
                                    segment = idx,
                                    bytes = meta.len();
                                    "Segment already exists, skipping"
                                );
                                let bytes = meta.len();
                                // Mark as completed in state
                                {
                                    let mut state_guard = state.lock().await;
                                    state_guard.mark_completed(idx, bytes);
                                }
                                segments.fetch_add(1, Ordering::Relaxed);
                                progress.fetch_add(bytes, Ordering::Relaxed);
                                return Ok((idx, segment_path, bytes));
                            }
                        }
                    }

                    // Download segment with retry logic (now using backon)
                    let result = downloader
                        .download_segment_with_retry(
                            idx,
                            url,
                            segment_path.clone(),
                            progress.clone(),
                        )
                        .await;

                    match &result {
                        Ok((_, _, bytes)) => {
                            // Update state on success
                            let mut state_guard = state.lock().await;
                            state_guard.mark_completed(idx, *bytes);

                            // Save state every 50 segments for crash recovery
                            if state_guard.completed_segments.len() % 50 == 0 {
                                if let Err(e) = state_guard.save(&output_path).await {
                                    warn!("Failed to save HLS state: {e}");
                                }
                            }

                            segments.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            // Save state on error before propagating
                            if let Err(save_err) = state.lock().await.save(&output_path).await {
                                warn!("Failed to save HLS state on error: {save_err}");
                            }
                            warn!(segment = idx; "Segment download failed: {e}");
                        }
                    }

                    result
                }
            })
            .buffer_unordered(self.concurrent_segments)
            .try_collect()
            .await?;

        // Save final state
        if let Err(e) = state.lock().await.save(output_path).await {
            warn!("Failed to save final HLS state: {e}");
        }

        // Build complete list of segment paths (including pre-existing)
        let mut all_segment_paths: Vec<(usize, PathBuf)> = Vec::with_capacity(total_segments);

        // Add downloaded segments from this run
        for (idx, path, _) in results {
            all_segment_paths.push((idx, path));
        }

        // Add pre-existing segments
        for idx in completed {
            let segment_path = temp_dir.join(format!("{base_filename}.part{idx}"));
            if segment_path.exists() {
                all_segment_paths.push((idx, segment_path));
            }
        }

        // Sort by index for correct merge order
        all_segment_paths.sort_by_key(|(idx, _)| *idx);

        // Verify we have all segments
        let actual_count = all_segment_paths.len();
        if actual_count != total_segments {
            return Err(RdlpError::Download(format!(
                "Missing segments: expected {total_segments}, got {actual_count}"
            )));
        }

        let segment_paths: Vec<PathBuf> = all_segment_paths
            .into_iter()
            .map(|(_, path)| path)
            .collect();

        info!(total = total_segments; "All segments ready for merge");
        Ok(segment_paths)
    }

    /// Merge segment files into final output file
    ///
    /// Concatenates all segment files in order into the final video file.
    /// Uses buffered I/O for efficient merging.
    ///
    /// # Arguments
    /// * `segment_paths` - Paths to segment files (in order)
    /// * `output_path` - Final output file path
    ///
    /// # Returns
    /// * `Ok(u64)` - Total bytes written
    /// * `Err(_)` - I/O error during merge
    async fn merge_segments(&self, segment_paths: Vec<PathBuf>, output_path: &Path) -> Result<u64> {
        info!(segments = segment_paths.len(); "Merging segments into final file");

        let final_file = File::create(output_path).await.map_err(RdlpError::Io)?;

        let mut writer = BufWriter::with_capacity(self.buffer_size, final_file);
        let mut total_bytes = 0u64;

        for (idx, segment_path) in segment_paths.iter().enumerate() {
            let mut segment_file = File::open(segment_path).await.map_err(RdlpError::Io)?;

            let bytes = tokio::io::copy(&mut segment_file, &mut writer)
                .await
                .map_err(RdlpError::Io)?;

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

        writer.flush().await.map_err(RdlpError::Io)?;
        info!(mb = total_bytes / (1024 * 1024); "Merge complete");

        Ok(total_bytes)
    }

    /// Clean up temporary segment files
    ///
    /// Deletes all temporary segment files after successful merge.
    /// Logs deletion progress for transparency.
    ///
    /// # Arguments
    /// * `segment_paths` - Paths to segment files to delete
    async fn cleanup_segments(&self, segment_paths: Vec<PathBuf>) {
        debug!(count = segment_paths.len(); "Cleaning up segment files");

        let mut deleted = 0;
        for path in segment_paths {
            if tokio::fs::remove_file(&path).await.is_ok() {
                deleted += 1;
            }
        }

        debug!(deleted; "Segment cleanup complete");
    }
}

impl Default for HlsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for HlsDownloader {
    fn protocol(&self) -> &str {
        "hls"
    }

    fn supports(&self, url: &str) -> bool {
        url.ends_with(".m3u8") || url.contains("/playlist.m3u8") || url.contains(".m3u8?")
    }

    async fn get_size(&self, _url: &str) -> Result<Option<u64>> {
        // Size detection is handled by HlsSizeDetector in extractor layer
        Ok(None)
    }

    #[instrument(skip(self, progress), fields(url = %url, path = %path.display()))]
    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();

        // Step 1: Parse playlist
        let segment_urls = self.parse_playlist(url).await?;
        let total_segments = segment_urls.len();
        info!(segments = total_segments; "Parsed HLS playlist");

        // Step 2: Load or create state for resume support
        let state = match HlsDownloadState::load(path, url, total_segments).await {
            Some(existing_state) => {
                let completed = existing_state.completed_segments.len();
                let remaining = total_segments - completed;
                info!(
                    completed,
                    total = total_segments,
                    remaining;
                    "Resuming HLS download"
                );
                Arc::new(Mutex::new(existing_state))
            }
            None => {
                info!("Starting fresh HLS download");
                Arc::new(Mutex::new(HlsDownloadState::new(
                    url.to_string(),
                    total_segments,
                )))
            }
        };

        // Step 3: Setup progress tracking
        let downloaded = Arc::new(AtomicU64::new(state.lock().await.total_bytes_downloaded));
        let segments_completed = Arc::new(AtomicU64::new(
            state.lock().await.completed_segments.len() as u64,
        ));
        let total_segments_u64 = total_segments as u64;

        // Spawn progress reporter task with segment-based progress
        let progress_task = if let Some(callback) = progress {
            let downloaded_clone = downloaded.clone();
            let segments_clone = segments_completed.clone();
            let start_time_clone = start_time;
            Some(tokio::spawn(async move {
                let mut last_update = Instant::now();
                let update_interval = Duration::from_millis(100);

                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let now = Instant::now();
                    if now.duration_since(last_update) >= update_interval {
                        let bytes = downloaded_clone.load(Ordering::Relaxed);
                        let segments = segments_clone.load(Ordering::Relaxed);
                        let elapsed = now.duration_since(start_time_clone).as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            bytes as f64 / elapsed
                        } else {
                            0.0
                        };

                        // Use segment-based progress for HLS
                        let progress_info = DownloadProgress::new_with_segments(
                            bytes,
                            speed,
                            segments,
                            total_segments_u64,
                        );
                        callback.on_progress(&progress_info);
                        last_update = now;
                    }
                }
            }))
        } else {
            None
        };

        // Step 4: Download segments (with resume support)
        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let base_filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("download");

        let segment_paths = match self
            .download_segments_with_resume(
                segment_urls.clone(),
                temp_dir,
                base_filename,
                downloaded.clone(),
                segments_completed.clone(),
                state.clone(),
                path,
            )
            .await
        {
            Ok(paths) => paths,
            Err(e) => {
                // Save state on error (so we can resume later)
                if let Err(save_err) = state.lock().await.save(path).await {
                    warn!("Failed to save HLS state: {save_err}");
                }
                if let Some(task) = progress_task {
                    task.abort();
                }
                return Err(e);
            }
        };

        // Stop progress reporter
        if let Some(task) = progress_task {
            task.abort();
        }

        // Step 5: Merge segments
        let total_bytes = self.merge_segments(segment_paths.clone(), path).await?;

        // Step 6: Cleanup segments and state file
        self.cleanup_segments(segment_paths).await;
        if let Err(e) = HlsDownloadState::delete(path).await {
            warn!("Failed to delete HLS state file: {e}");
        }

        // Step 7: Return statistics
        let duration = start_time.elapsed();
        let stats = DownloadStats::new(total_bytes, duration, 0).with_fragments(total_segments);

        let duration_secs = duration.as_secs_f64();
        let speed_mbps = (total_bytes as f64 / duration_secs) / (1024.0 * 1024.0);
        info!(
            mb = total_bytes / (1024 * 1024),
            duration_secs:? = duration_secs,
            speed_mbps:? = speed_mbps;
            "HLS download complete"
        );

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hls_downloader_creation() {
        let downloader = HlsDownloader::new();
        assert_eq!(downloader.protocol(), "hls");
        assert_eq!(downloader.concurrent_segments, 8);
    }

    #[test]
    fn test_hls_downloader_builder() {
        let downloader = HlsDownloader::new()
            .with_concurrent_segments(16)
            .with_buffer_size(4 * 1024 * 1024);

        assert_eq!(downloader.concurrent_segments, 16);
        assert_eq!(downloader.buffer_size, 4 * 1024 * 1024);
    }

    #[test]
    fn test_supports_m3u8_urls() {
        let downloader = HlsDownloader::new();

        assert!(downloader.supports("https://example.com/video.m3u8"));
        assert!(downloader.supports("https://example.com/playlist.m3u8"));
        assert!(downloader.supports("https://example.com/index.m3u8?token=abc"));
        assert!(!downloader.supports("https://example.com/video.mp4"));
    }

    #[test]
    fn test_concurrent_segments_minimum() {
        let downloader = HlsDownloader::new().with_concurrent_segments(0);
        // Should be clamped to minimum of 1
        assert_eq!(downloader.concurrent_segments, 1);
    }
}
