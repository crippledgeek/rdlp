use async_trait::async_trait;
use futures::stream::{self, StreamExt, TryStreamExt};
use rdlp_core::{
    Downloader, DownloadProgress, DownloadStats, ProgressCallback,
    Result, RetryConfig, RdlpError,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

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
pub struct HlsDownloader {
    http_downloader: HttpDownloader,
    concurrent_segments: usize,
    buffer_size: usize,
    retry_config: Arc<RetryConfig>,
}

impl HlsDownloader {
    /// Create a new HLS downloader with default settings
    pub fn new() -> Self {
        Self {
            http_downloader: HttpDownloader::new(),
            concurrent_segments: 8, // Default: 8 parallel segments
            buffer_size: 2 * 1024 * 1024, // 2 MB buffer for merging
            retry_config: Arc::new(RetryConfig::default_config()),
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
        let playlist_text = self.http_downloader.client()
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

                let segments: Vec<String> = media.segments
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
                        segments.len(), MAX_SEGMENTS
                    )));
                }

                Ok(segments)
            }
            m3u8_rs::Playlist::MasterPlaylist(master) => {
                // Master playlist - select first variant
                if master.variants.is_empty() {
                    return Err(RdlpError::Extraction("Master playlist has no variants".into()));
                }

                let variant = &master.variants[0];
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

                let media_playlist_url = base_url
                    .join(&variant.uri)
                    .map_err(|e| RdlpError::Extraction(format!("Failed to join URL: {e}")))?
                    .to_string();

                eprintln!("[HLS] Master playlist detected, selecting variant: {} (bandwidth: {} bps)",
                    variant.uri, variant.bandwidth);

                // Recursively parse media playlist
                Box::pin(self.parse_playlist(&media_playlist_url)).await
            }
        }
    }

    /// Download all segments in parallel
    ///
    /// Downloads segments to temporary files using the HTTP downloader.
    /// Uses `buffer_unordered` for bounded parallelism.
    ///
    /// # Arguments
    /// * `segment_urls` - List of segment URLs to download
    /// * `temp_dir` - Directory to save temporary segment files
    /// * `base_filename` - Base filename for temporary files
    /// * `progress_counter` - Shared atomic counter for progress tracking
    ///
    /// # Returns
    /// * `Ok(Vec<PathBuf>)` - Paths to downloaded segment files (in order)
    /// * `Err(_)` - Download error (network, I/O, etc.)
    async fn download_segments(
        &self,
        segment_urls: Vec<String>,
        temp_dir: &Path,
        base_filename: &str,
        progress_counter: Arc<AtomicU64>,
    ) -> Result<Vec<PathBuf>> {
        let total_segments = segment_urls.len();

        eprintln!("📥 Downloading {} segments ({} concurrent)...",
            total_segments, self.concurrent_segments);

        // Download all segments using buffer_unordered for batch processing
        let results: Vec<(usize, PathBuf, u64)> = stream::iter(segment_urls.into_iter().enumerate())
            .map(|(idx, url)| {
                let segment_path = temp_dir.join(format!("{base_filename}.part{idx}"));
                let http_client = self.http_downloader.client().clone();
                let progress = progress_counter.clone();
                let buffer_size = self.buffer_size;

                async move {
                    if idx % 100 == 0 {
                        eprintln!("📥 Starting segment {}/{}", idx + 1, total_segments);
                    }

                    // Download segment to temporary file
                    let response = http_client
                        .get(&url)
                        .send()
                        .await
                        .map_err(|e| RdlpError::Network(format!("Segment {idx} failed: {e}")))?;

                    if !response.status().is_success() {
                        return Err(RdlpError::Network(format!(
                            "Segment {} returned HTTP {}", idx, response.status()
                        )));
                    }

                    // Stream segment to file with progress tracking
                    let file = File::create(&segment_path)
                        .await
                        .map_err(RdlpError::Io)?;

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

                    Ok::<(usize, PathBuf, u64), RdlpError>((idx, segment_path, downloaded))
                }
            })
            .buffer_unordered(self.concurrent_segments)
            .try_collect()
            .await?;

        // Sort results by index to ensure correct order
        let mut sorted_results = results;
        sorted_results.sort_by_key(|(idx, _, _)| *idx);

        // Extract paths and log progress
        let segment_paths: Vec<PathBuf> = sorted_results
            .into_iter()
            .enumerate()
            .map(|(count, (idx, path, _bytes))| {
                if (count + 1) % 100 == 0 || count == total_segments - 1 {
                    eprintln!("   ✓ Completed {}/{} segments", count + 1, total_segments);
                }
                (idx, path)
            })
            .map(|(_, path)| path)
            .collect();

        eprintln!("✓ All {total_segments} segments downloaded");
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
    async fn merge_segments(
        &self,
        segment_paths: Vec<PathBuf>,
        output_path: &Path,
    ) -> Result<u64> {
        eprintln!("📝 Merging {} segments into final file...", segment_paths.len());

        let final_file = File::create(output_path)
            .await
            .map_err(RdlpError::Io)?;

        let mut writer = BufWriter::with_capacity(self.buffer_size, final_file);
        let mut total_bytes = 0u64;

        for (idx, segment_path) in segment_paths.iter().enumerate() {
            let mut segment_file = File::open(segment_path)
                .await
                .map_err(RdlpError::Io)?;

            let bytes = tokio::io::copy(&mut segment_file, &mut writer)
                .await
                .map_err(RdlpError::Io)?;

            total_bytes += bytes;

            if (idx + 1) % 100 == 0 || idx == segment_paths.len() - 1 {
                eprintln!("   ✓ Merged {}/{} segments ({} MB total)",
                    idx + 1, segment_paths.len(), total_bytes / (1024 * 1024));
            }
        }

        writer.flush().await.map_err(RdlpError::Io)?;
        eprintln!("✓ Merge complete: {} MB", total_bytes / (1024 * 1024));

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
        eprintln!("🧹 Cleaning up {} segment files...", segment_paths.len());

        let mut deleted = 0;
        for path in segment_paths {
            if tokio::fs::remove_file(&path).await.is_ok() {
                deleted += 1;
            }
        }

        eprintln!("   ✓ Deleted {deleted} files");
    }

    /// Clean up segments on error
    ///
    /// Called when download fails to remove partial segment files.
    async fn cleanup_segments_on_error(&self, temp_dir: &Path, base_filename: &str, total_segments: usize) {
        eprintln!("🧹 Cleaning up partial segment files...");

        let mut deleted = 0;
        for idx in 0..total_segments {
            let path = temp_dir.join(format!("{base_filename}.part{idx}"));
            if tokio::fs::remove_file(&path).await.is_ok() {
                deleted += 1;
            }
        }

        if deleted > 0 {
            eprintln!("   ✓ Deleted {deleted} partial files");
        }
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

    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();

        // Step 1: Parse playlist
        let segment_urls = self.parse_playlist(url).await?;
        eprintln!("📋 Found {} segments in playlist", segment_urls.len());

        // Step 2: Setup progress tracking
        let downloaded = Arc::new(AtomicU64::new(0));

        // Spawn progress reporter task
        let progress_task = if let Some(callback) = progress {
            let downloaded_clone = downloaded.clone();
            let start_time_clone = start_time;
            Some(tokio::spawn(async move {
                let mut last_update = Instant::now();
                let update_interval = Duration::from_millis(100);

                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let now = Instant::now();
                    if now.duration_since(last_update) >= update_interval {
                        let bytes = downloaded_clone.load(Ordering::Relaxed);
                        let elapsed = now.duration_since(start_time_clone).as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            bytes as f64 / elapsed
                        } else {
                            0.0
                        };

                        let progress_info = DownloadProgress::new(bytes, None, speed);
                        callback.on_progress(&progress_info);
                        last_update = now;
                    }
                }
            }))
        } else {
            None
        };

        // Step 3: Download segments
        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let base_filename = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("download");

        let segment_paths = match self.download_segments(
            segment_urls.clone(),
            temp_dir,
            base_filename,
            downloaded.clone(),
        ).await {
            Ok(paths) => paths,
            Err(e) => {
                // Clean up partial downloads on error
                if let Some(task) = progress_task {
                    task.abort();
                }
                self.cleanup_segments_on_error(temp_dir, base_filename, segment_urls.len()).await;
                return Err(e);
            }
        };

        // Stop progress reporter
        if let Some(task) = progress_task {
            task.abort();
        }

        // Step 4: Merge segments
        let total_bytes = self.merge_segments(segment_paths.clone(), path).await?;

        // Step 5: Cleanup
        self.cleanup_segments(segment_paths).await;

        // Step 6: Return statistics
        let duration = start_time.elapsed();
        let stats = DownloadStats::new(total_bytes, duration, 0)
            .with_fragments(segment_urls.len());

        eprintln!("✅ HLS download complete: {} MB in {:.1}s ({:.1} MB/s)",
            total_bytes / (1024 * 1024),
            duration.as_secs_f64(),
            (total_bytes as f64 / duration.as_secs_f64()) / (1024.0 * 1024.0));

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
