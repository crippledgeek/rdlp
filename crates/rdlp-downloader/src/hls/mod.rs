//! HLS (HTTP Live Streaming) downloader module.
//!
//! Downloads HLS streams by parsing m3u8 playlists and downloading segments
//! in parallel using the HTTP downloader infrastructure.
//!
//! # Architecture
//!
//! 1. Parse m3u8 playlist to extract segment URLs
//! 2. Download segments in parallel using `buffer_unordered`
//! 3. Merge segments into final video file
//! 4. Clean up temporary segment files
//!
//! # Performance
//!
//! - Default: 8 concurrent segment downloads
//! - Typical: 500 MB video in 60-90 seconds
//! - Bottleneck: Server throttling (not client)

mod merge;
mod playlist;
mod segment;
mod types;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use log::{info, warn};
use rdlp_core::{DownloadStats, Downloader, ProgressCallback, RdlpError, Result, RetryConfig};
use tokio::sync::Mutex;
use tracing::instrument;

use crate::hls_state::HlsDownloadState;
use crate::http::HttpDownloader;
use crate::progress::{ProgressMetrics, ProgressReporterConfig, spawn_progress_reporter};

use self::merge::{cleanup_segments, download_segments_with_resume, merge_segments};
use self::playlist::parse_playlist;
use self::segment::download_init_segment;
use self::types::InitSegmentInfo;

/// HLS (HTTP Live Streaming) downloader
///
/// Downloads HLS streams by parsing m3u8 playlists and downloading segments
/// in parallel using the HTTP downloader infrastructure.
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
    /// Total download timeout (entire operation must complete within this)
    download_timeout: Duration,
    /// Merge operation timeout (segment merge must complete within this)
    merge_timeout: Duration,
    /// Maximum number of segment failures before aborting the download
    max_segment_failures: usize,
}

impl HlsDownloader {
    /// Create a new HLS downloader with default settings
    #[must_use]
    pub fn new() -> Self {
        Self {
            http_downloader: HttpDownloader::new(),
            concurrent_segments: 8,       // Default: 8 parallel segments
            buffer_size: 2 * 1024 * 1024, // 2 MB buffer for merging
            retry_config: Arc::new(RetryConfig::default_config()),
            expected_size: None,
            download_timeout: Duration::from_secs(3600), // 1 hour
            merge_timeout: Duration::from_secs(1800),    // 30 min
            max_segment_failures: 3,
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

    /// Set total download timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_download_timeout(mut self, timeout: Duration) -> Self {
        self.download_timeout = timeout;
        self
    }

    /// Set merge operation timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_merge_timeout(mut self, timeout: Duration) -> Self {
        self.merge_timeout = timeout;
        self
    }

    /// Set maximum number of segment failures before aborting
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_max_segment_failures(mut self, max: usize) -> Self {
        self.max_segment_failures = max;
        self
    }

    /// Set extra HTTP headers sent with every request (delegates to inner HttpDownloader)
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_extra_headers(
        mut self,
        headers: Option<&std::collections::HashMap<String, String>>,
    ) -> Self {
        self.http_downloader = self.http_downloader.with_extra_headers(headers);
        self
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
        let timeout = self.download_timeout;
        tokio::time::timeout(timeout, async {
            let start_time = Instant::now();

            // Step 1: Parse playlist
            let playlist = parse_playlist(&self.http_downloader, url).await?;
            let segments = playlist.segments;
            let has_init = segments.iter().any(|s| s.init_segment.is_some());
            let total_segments = segments.len();
            let total_duration: f64 = segments.iter().map(|s| s.duration).sum();
            info!(
                segments = total_segments,
                duration_secs = total_duration,
                fmp4 = has_init;
                "Parsed HLS playlist"
            );

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
            // Duration in centiseconds for atomic precision (f64 -> u64)
            let duration_completed = Arc::new(AtomicU64::new(0));
            let total_segments_u64 = total_segments as u64;

            // Spawn progress reporter task with duration-based progress
            let mut progress_guard = spawn_progress_reporter(
                progress,
                ProgressMetrics::with_duration(
                    downloaded.clone(),
                    segments_completed.clone(),
                    duration_completed.clone(),
                ),
                ProgressReporterConfig::hls(start_time, total_segments_u64, total_duration),
            );

            // Step 4: Download segments (with resume support)
            let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
            let base_filename = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("download");

            let segment_paths = match download_segments_with_resume(
                &self.http_downloader,
                self.retry_config.clone(),
                self.buffer_size,
                self.concurrent_segments,
                self.max_segment_failures,
                segments.clone(),
                temp_dir,
                base_filename,
                downloaded.clone(),
                segments_completed.clone(),
                duration_completed.clone(),
                state.clone(),
                path,
            )
            .await
            {
                Ok(paths) => paths,
                Err(e) => {
                    // Save state on error (so we can resume later)
                    let snapshot = state.lock().await.clone();
                    if let Err(save_err) = snapshot.save(path).await {
                        warn!("Failed to save HLS state: {save_err}");
                    }
                    progress_guard.abort();
                    return Err(e);
                }
            };

            // Progress guard will be dropped and abort the task automatically
            drop(progress_guard);

            // Step 5: Download unique init segments (fMP4 EXT-X-MAP)
            // Collect unique init segments and download each once.
            let mut init_file_map: HashMap<InitSegmentInfo, std::path::PathBuf> = HashMap::new();
            {
                let unique_inits: Vec<InitSegmentInfo> = segments
                    .iter()
                    .filter_map(|s| s.init_segment.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();

                for (i, init) in unique_inits.into_iter().enumerate() {
                    let init_path = temp_dir.join(format!("{base_filename}.init{i}"));
                    info!(index = i; "Downloading fMP4 init segment (EXT-X-MAP): {}", init.url);
                    download_init_segment(
                        &self.http_downloader,
                        &self.retry_config,
                        &init,
                        &init_path,
                    )
                    .await?;
                    init_file_map.insert(init, init_path);
                }
            }

            // Build per-segment init path mapping for merge
            let segment_init_paths: Vec<Option<std::path::PathBuf>> = segments
                .iter()
                .map(|s| {
                    s.init_segment
                        .as_ref()
                        .and_then(|init| init_file_map.get(init).cloned())
                })
                .collect();

            // Step 6: Merge segments (re-inserting init segment on change)
            let total_bytes = merge_segments(
                self.buffer_size,
                self.merge_timeout,
                &segment_paths,
                path,
                &segment_init_paths,
            )
            .await?;

            // Step 7: Cleanup segments, init segments, and state file
            cleanup_segments(&segment_paths).await;
            for init_path in init_file_map.values() {
                let _ = tokio::fs::remove_file(init_path).await;
            }
            if let Err(e) = HlsDownloadState::delete(path).await {
                warn!("Failed to delete HLS state file: {e}");
            }

            // Step 8: Return statistics
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
        })
        .await
        .map_err(|_| {
            RdlpError::Download(format!("Download timed out after {}s", timeout.as_secs()))
        })?
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
