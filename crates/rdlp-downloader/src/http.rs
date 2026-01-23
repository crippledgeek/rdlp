use async_trait::async_trait;
use futures::StreamExt;
use rdlp_core::{check_http_response, retry_with_backoff, DownloadProgress, DownloadStats, Downloader, ProgressCallback, Result, RetryConfig, RdlpError};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use crate::chunking::{calculate_chunks, ChunkSizeStrategy};

/// Global atomic counter for generating unique download IDs
/// This prevents chunk file collisions when multiple downloads run concurrently
static DOWNLOAD_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// RAII guard for progress reporter tasks
///
/// Ensures the progress reporter task is aborted when the guard goes out of scope,
/// preventing task leaks on early returns or errors.
struct ProgressGuard(Option<tokio::task::JoinHandle<()>>);

impl ProgressGuard {
    fn new(task: Option<tokio::task::JoinHandle<()>>) -> Self {
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
async fn cleanup_chunk_files(temp_dir: &Path, filename: &str, download_id: u64, total_chunks: usize) {
    eprintln!("🧹 Cleaning up {total_chunks} partial chunk files...");
    let mut deleted = 0;
    for chunk_id in 0..total_chunks {
        let chunk_path = temp_dir.join(format!("{filename}.{download_id}.part{chunk_id}"));
        if tokio::fs::remove_file(&chunk_path).await.is_ok() {
            deleted += 1;
        }
    }
    eprintln!("   ✓ Deleted {deleted} chunk files");
}

/// Downloader configuration (shared across clones via Arc)
///
/// This struct consolidates all config fields into a single Arc,
/// making HttpDownloader clones truly zero-cost (~5ns Arc clone vs ~24 bytes field copies).
///
/// **Memory optimization:**
/// - Before: 591 clones × 24 bytes = ~14 KB copied
/// - After: 591 Arc clones × 8 bytes = ~5 KB pointers
/// - **Savings: ~9 KB per download**
#[derive(Clone)]
struct DownloaderConfig {
    buffer_size: usize,
    retry_config: RetryConfig,
    concurrent_fragments: usize,
    chunk_strategy: ChunkSizeStrategy,
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        // Calculate optimal concurrent connections based on CPU threads
        // For I/O-bound workloads like HTTP downloads:
        // - Tokio can handle many more tasks than CPU cores
        // - Research shows: aria2 uses 4-16 connections, yt-dlp defaults to 1
        // - Formula: min(available_parallelism, 8) for balanced I/O saturation
        //   * Too few: underutilizes bandwidth
        //   * Too many: connection overhead, server rate limiting
        let concurrent_fragments = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4); // Fallback to 4 if detection fails

        Self {
            buffer_size: 8192, // 8KB buffer
            retry_config: RetryConfig::default_config(),
            concurrent_fragments,
            chunk_strategy: ChunkSizeStrategy::Auto, // Use automatic power-of-two chunking
        }
    }
}

/// HTTP/HTTPS downloader
///
/// **Clone performance:** O(1) - both client and config use Arc internally
#[derive(Clone)]
pub struct HttpDownloader {
    client: reqwest::Client,
    config: Arc<DownloaderConfig>,
}

impl HttpDownloader {
    /// Create a new HTTP downloader
    pub fn new() -> Self {
        Self::with_client(reqwest::Client::new())
    }

    /// Create with custom client
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            config: Arc::new(DownloaderConfig::default()),
        }
    }

    /// Get reference to the HTTP client
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Set buffer size for downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        Arc::make_mut(&mut self.config).buffer_size = size;
        self
    }

    /// Set retry configuration
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        Arc::make_mut(&mut self.config).retry_config = config;
        self
    }

    /// Set number of concurrent fragment downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_concurrent_fragments(mut self, count: usize) -> Self {
        Arc::make_mut(&mut self.config).concurrent_fragments = count.max(1); // At least 1
        self
    }

    /// Set chunk size strategy
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_chunk_strategy(mut self, strategy: ChunkSizeStrategy) -> Self {
        Arc::make_mut(&mut self.config).chunk_strategy = strategy;
        self
    }

    /// Check if server supports range requests
    async fn supports_ranges(&self, url: &str) -> Result<bool> {
        let response = retry_with_backoff(&self.config.retry_config, "HTTP HEAD (range check)", |_attempt| {
            let client = self.client.clone();
            let url = url.to_string();
            async move {
                client
                    .head(&url)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Failed to HEAD request: {e}")))
            }
        })
        .await?;

        // Check for Accept-Ranges header
        Ok(response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .map(|v| v != "none")
            .unwrap_or(false))
    }

    /// Download a specific byte range with shared progress tracking
    ///
    /// # Arc<AtomicU64> Pattern for Lock-Free Concurrency
    ///
    /// The `progress_counter` uses `Arc<AtomicU64>` for efficient shared state:
    ///
    /// **Memory Model:**
    /// ```rust,ignore
    /// Arc<AtomicU64>
    /// │
    /// ├─ Arc: Atomic Reference Counting
    /// │  └─ Enables cheap cloning across parallel tasks (~5ns per clone)
    /// │  └─ Thread-safe sharing via atomic refcount
    /// │
    /// └─ AtomicU64: Lock-free atomic counter
    ///    └─ No Mutex needed (no lock contention)
    ///    └─ Updates via CPU atomic instructions (~2ns vs 50ns for Mutex)
    ///    └─ Ordering::Relaxed sufficient (no inter-thread synchronization needed)
    /// ```
    ///
    /// **Why not Arc<Mutex<u64>>?** (Phase 1 optimization)
    /// - Mutex: Async lock acquisition (~50ns) + contention overhead
    /// - AtomicU64: Single CPU instruction (~2ns) + zero contention
    /// - **25x faster** for simple counter increments
    ///
    /// **Hot path performance:**
    /// - Called hundreds to thousands of times per download
    /// - Phase 1 benchmark: 5,400 atomic updates with zero lock contention
    /// - Savings: ~48ns × 5,400 = 259μs per parallel download
    ///
    /// **Thread Safety:**
    /// - Each parallel chunk task holds an `Arc` clone
    /// - All chunks increment the same atomic counter concurrently
    /// - No locks, no contention, no await overhead
    async fn download_range_with_progress(
        &self,
        url: &str,
        start: u64,
        end: u64,
        chunk_path: &Path,
        progress_counter: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    ) -> Result<u64> {
        let response = retry_with_backoff(&self.config.retry_config, "HTTP GET (range)", |_attempt| {
            let client = self.client.clone();
            let url = url.to_string();
            async move {
                let response = client
                    .get(&url)
                    .header("Range", format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Failed to fetch range: {e}")))?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .await?;

        // Write chunk to temporary file
        let file = File::create(chunk_path).await.map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk).await.map_err(RdlpError::Io)?;
            downloaded += chunk.len() as u64;

            // Update shared progress counter in real-time (lock-free atomic operation)
            if let Some(ref counter) = progress_counter {
                counter.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }
        }

        writer.flush().await.map_err(RdlpError::Io)?;
        Ok(downloaded)
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpDownloader {
    /// Sequential download (original method)
    async fn download_sequential(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();

        // Make the request with retry logic
        let response = retry_with_backoff(&self.config.retry_config, "HTTP GET", |_attempt| {
            let client = self.client.clone();
            let url = url.to_string();
            async move {
                let response = client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Failed to fetch URL: {e}")))?;

                // Check status
                check_http_response(&response)?;

                Ok(response)
            }
        })
        .await?;

        // Get content length if available
        let total_size = response.content_length();

        // Create output file with buffered writer
        let file = File::create(path)
            .await
            .map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        // Download with progress tracking
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_update = Instant::now();
        let update_interval = Duration::from_millis(100); // Update every 100ms

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk)
                .await
                .map_err(RdlpError::Io)?;

            downloaded += chunk.len() as u64;

            // Update progress
            if let Some(ref callback) = progress {
                let now = Instant::now();
                if now.duration_since(last_update) >= update_interval {
                    let elapsed = now.duration_since(start_time).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        downloaded as f64 / elapsed
                    } else {
                        0.0
                    };

                    let progress_info = DownloadProgress::new(downloaded, total_size, speed);
                    callback.on_progress(&progress_info);
                    last_update = now;
                }
            }
        }

        // Ensure all data is written to disk
        writer.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(downloaded, duration, 0);

        // Notify completion
        if let Some(callback) = progress {
            callback.on_complete(&stats);
        }

        Ok(stats)
    }

    /// Parallel download using multiple range requests with fine-grained chunking
    ///
    /// Uses power-of-two chunk sizes for optimal performance and `buffer_unordered`
    /// for automatic batch processing with concurrency limiting.
    async fn download_parallel(
        &self,
        url: &str,
        path: &Path,
        total_size: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use futures::stream::{self, StreamExt, TryStreamExt};

        let start_time = Instant::now();

        // Generate unique download ID to prevent chunk file collisions
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Calculate optimal power-of-two chunk sizes
        let (chunk_size, total_chunks) = calculate_chunks(total_size, self.config.chunk_strategy);

        eprintln!("📊 Chunk analysis:");
        eprintln!("   - Download ID: {download_id}");
        eprintln!("   - Total size: {} MB", total_size / 1024 / 1024);
        eprintln!("   - Chunk size: {} KB (power-of-two aligned)", chunk_size / 1024);
        eprintln!("   - Total chunks: {total_chunks}");
        eprintln!("   - Concurrent downloads: {}", self.config.concurrent_fragments);
        eprintln!("   - Batches: {} (processing {} at a time)",
            total_chunks.div_ceil(self.config.concurrent_fragments),
            self.config.concurrent_fragments);

        // Create temporary directory for chunks
        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let filename = path.file_name()
            .ok_or_else(|| RdlpError::Download("Invalid output path: no filename".to_string()))?
            .to_string_lossy()
            .to_string();

        // Shared progress counter (lock-free atomic for better performance)
        let downloaded = Arc::new(AtomicU64::new(0));

        // Progress reporter task (wrapped in guard for automatic cleanup)
        let _progress_guard = ProgressGuard::new(if let Some(callback) = progress {
            let downloaded_clone = downloaded.clone();
            let start_time_clone = start_time;
            Some(tokio::spawn(async move {
                let mut last_update = Instant::now();
                let update_interval = Duration::from_millis(100);

                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let now = Instant::now();
                    if now.duration_since(last_update) >= update_interval {
                        let bytes_downloaded = downloaded_clone.load(Ordering::Relaxed);
                        let elapsed = now.duration_since(start_time_clone).as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            bytes_downloaded as f64 / elapsed
                        } else {
                            0.0
                        };

                        let progress_info = DownloadProgress::new(bytes_downloaded, Some(total_size), speed);
                        callback.on_progress(&progress_info);
                        last_update = now;

                        // Exit when complete
                        if bytes_downloaded >= total_size {
                            break;
                        }
                    }
                }
            }))
        } else {
            None
        });

        eprintln!("🚀 Starting batch download with {} concurrent connections...", self.config.concurrent_fragments);

        // Arc<str> optimization: Share URL across all chunks (saves ~59 KB for 591 chunks)
        let url_shared: Arc<str> = Arc::from(url);

        // Download all chunks using buffer_unordered for automatic batch processing
        let chunk_results: Vec<u64> = stream::iter(0..total_chunks)
            .map(|chunk_id| {
                let start = chunk_id as u64 * chunk_size as u64;
                let end = if chunk_id == total_chunks - 1 {
                    total_size - 1 // Last chunk gets remainder
                } else {
                    start + chunk_size as u64 - 1
                };

                let chunk_path = temp_dir.join(format!("{}.{}.part{}",
                    &filename, download_id, chunk_id));

                // Clone what we need for the async block (O(1) Arc clone)
                let downloader = Self {
                    client: self.client.clone(),
                    config: Arc::clone(&self.config),
                };
                let url = Arc::clone(&url_shared);  // Cheap Arc clone (~5ns)
                let progress_counter = Some(downloaded.clone());

                async move {
                    let result = downloader.download_range_with_progress(
                        &url, start, end, &chunk_path, progress_counter
                    ).await;
                    if let Err(ref e) = result {
                        eprintln!("\n❌ Chunk {chunk_id} failed: {e}");
                    }
                    result.map(|bytes| (chunk_id, bytes, chunk_path))
                }
            })
            .buffer_unordered(self.config.concurrent_fragments) // Automatic batch processing!
            .try_collect::<Vec<_>>()
            .await
            .inspect_err(|e| {
                eprintln!("❌ Download failed: {e}");
                // Note: Progress reporter is automatically cleaned up by _progress_guard on drop
                // Cleanup partial chunk files
                let temp_dir = temp_dir.to_path_buf();
                let filename = filename.clone();
                tokio::spawn(async move {
                    cleanup_chunk_files(&temp_dir, &filename, download_id, total_chunks).await;
                });
            })?
            .into_iter()
            .map(|(_, bytes, _)| bytes)
            .collect();

        // Sum up all downloaded bytes
        let total_downloaded: u64 = chunk_results.iter().sum();
        eprintln!("✓ All {} chunks completed ({} MB total)", total_chunks, total_downloaded / 1024 / 1024);

        // Note: Progress reporter is automatically cleaned up by _progress_guard on drop

        // Merge chunks into final file in correct order
        eprintln!("📝 Merging {total_chunks} chunks into final file...");
        let final_file = File::create(path).await.map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, final_file);

        let mut deleted_chunks = 0;
        for chunk_id in 0..total_chunks {
            let chunk_path = temp_dir.join(format!("{}.{}.part{}",
                &filename, download_id, chunk_id));

            let mut chunk_file = File::open(&chunk_path).await.map_err(RdlpError::Io)?;
            tokio::io::copy(&mut chunk_file, &mut writer)
                .await
                .map_err(RdlpError::Io)?;

            // Delete chunk file after merging
            if tokio::fs::remove_file(&chunk_path).await.is_ok() {
                deleted_chunks += 1;
            }

            if (chunk_id + 1) % 100 == 0 || chunk_id == total_chunks - 1 {
                eprintln!("   ✓ Merged {}/{} chunks", chunk_id + 1, total_chunks);
            }
        }
        eprintln!("🧹 Cleaned up {deleted_chunks} chunk files");

        writer.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        // Fix Issue #2: Use 0 for retry_count (we don't track retries at this level)
        let stats = DownloadStats::new(total_downloaded, duration, 0);

        eprintln!("✅ Download complete: {} MB in {:.1}s ({:.1} MB/s)",
            total_downloaded / 1024 / 1024,
            duration.as_secs_f64(),
            (total_downloaded as f64 / duration.as_secs_f64()) / 1024.0 / 1024.0);

        Ok(stats)
    }

    /// Parallel resume: downloads remaining chunks in parallel and appends to existing file
    ///
    /// This method keeps the already-downloaded portion and parallelizes the remaining download
    /// using power-of-two chunking and `buffer_unordered` for batch processing.
    async fn download_parallel_resume(
        &self,
        url: &str,
        path: &Path,
        resume_from: u64,
        total_size: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};
        use futures::stream::{self, StreamExt, TryStreamExt};

        let start_time = Instant::now();
        let remaining_size = total_size - resume_from;

        // Generate unique download ID to prevent chunk file collisions
        let download_id = DOWNLOAD_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Calculate optimal power-of-two chunk sizes for remaining data
        let (chunk_size, total_chunks) = calculate_chunks(remaining_size, self.config.chunk_strategy);

        eprintln!("🔄 Parallel resume mode:");
        eprintln!("   - Download ID: {download_id}");
        eprintln!("   - Already downloaded: {} MB ({:.1}%)",
            resume_from / 1024 / 1024,
            (resume_from as f64 / total_size as f64) * 100.0);
        eprintln!("   - Remaining: {} MB", remaining_size / 1024 / 1024);
        eprintln!("   - Chunk size: {} KB (power-of-two aligned)", chunk_size / 1024);
        eprintln!("   - Total chunks: {total_chunks}");
        eprintln!("   - Concurrent downloads: {}", self.config.concurrent_fragments);

        // Create temporary directory for chunks
        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let filename = path.file_name()
            .ok_or_else(|| RdlpError::Download("Invalid output path: no filename".to_string()))?
            .to_string_lossy()
            .to_string();

        // Shared progress counter (starts at resume_from, lock-free atomic)
        let downloaded = Arc::new(AtomicU64::new(resume_from));

        // Progress reporter task (shows total including already downloaded)
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
                        let bytes_downloaded = downloaded_clone.load(Ordering::Relaxed);
                        let elapsed = now.duration_since(start_time_clone).as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            (bytes_downloaded - resume_from) as f64 / elapsed
                        } else {
                            0.0
                        };

                        let progress_info = DownloadProgress::new(bytes_downloaded, Some(total_size), speed);
                        callback.on_progress(&progress_info);
                        last_update = now;

                        // Exit when complete
                        if bytes_downloaded >= total_size {
                            break;
                        }
                    }
                }
            }))
        } else {
            None
        };

        eprintln!("🚀 Starting batch download with {} concurrent connections...", self.config.concurrent_fragments);

        // Arc<str> optimization: Share URL across all chunks (saves ~59 KB for 591 chunks)
        let url_shared: Arc<str> = Arc::from(url);

        // Download remaining chunks using buffer_unordered for automatic batch processing
        let chunk_results: Vec<u64> = stream::iter(0..total_chunks)
            .map(|chunk_id| {
                let start = resume_from + (chunk_id as u64 * chunk_size as u64);
                let end = if chunk_id == total_chunks - 1 {
                    total_size - 1 // Last chunk gets remainder
                } else {
                    start + chunk_size as u64 - 1
                };

                let chunk_path = temp_dir.join(format!("{}.{}.resume{}",
                    &filename, download_id, chunk_id));

                // Clone what we need for the async block (O(1) Arc clone)
                let downloader = Self {
                    client: self.client.clone(),
                    config: Arc::clone(&self.config),
                };
                let url = Arc::clone(&url_shared);  // Cheap Arc clone (~5ns)
                let progress_counter = Some(downloaded.clone());

                async move {
                    let result = downloader.download_range_with_progress(
                        &url, start, end, &chunk_path, progress_counter
                    ).await;
                    if let Err(ref e) = result {
                        eprintln!("\n❌ Chunk {chunk_id} failed: {e}");
                    }
                    result.map(|bytes| (chunk_id, bytes, chunk_path))
                }
            })
            .buffer_unordered(self.config.concurrent_fragments) // Automatic batch processing!
            .try_collect::<Vec<_>>()
            .await
            .inspect_err(|e| {
                eprintln!("❌ Resume failed: {e}");
                // Stop progress reporter on error
                if let Some(task) = &progress_task {
                    task.abort();
                }
                // Cleanup partial chunk files
                let temp_dir = temp_dir.to_path_buf();
                let filename = filename.clone();
                tokio::spawn(async move {
                    cleanup_chunk_files(&temp_dir, &filename, download_id, total_chunks).await;
                });
            })?
            .into_iter()
            .map(|(_, bytes, _)| bytes)
            .collect();

        // Sum up all downloaded bytes (just the new chunks)
        let newly_downloaded: u64 = chunk_results.iter().sum();
        eprintln!("✓ All {} chunks completed ({} MB new data)", total_chunks, newly_downloaded / 1024 / 1024);

        // Stop progress reporter on success
        if let Some(task) = progress_task {
            task.abort();
        }

        // Append chunks to existing file in correct order
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        eprintln!("📝 Appending {total_chunks} chunks to existing file...");
        let mut deleted_chunks = 0;
        for chunk_id in 0..total_chunks {
            let chunk_path = temp_dir.join(format!("{}.{}.resume{}",
                &filename, download_id, chunk_id));

            let mut chunk_file = File::open(&chunk_path).await.map_err(RdlpError::Io)?;
            tokio::io::copy(&mut chunk_file, &mut writer)
                .await
                .map_err(RdlpError::Io)?;

            // Delete chunk file after appending
            if tokio::fs::remove_file(&chunk_path).await.is_ok() {
                deleted_chunks += 1;
            }

            if (chunk_id + 1) % 100 == 0 || chunk_id == total_chunks - 1 {
                eprintln!("   ✓ Appended {}/{} chunks", chunk_id + 1, total_chunks);
            }
        }
        eprintln!("🧹 Cleaned up {deleted_chunks} chunk files");

        writer.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        let total_downloaded = resume_from + newly_downloaded;
        // Fix Issue #2: Use 0 for retry_count (we don't track retries at this level)
        let stats = DownloadStats::new(total_downloaded, duration, 0);

        eprintln!("✅ Resume complete: {} MB total, {} MB new in {:.1}s ({:.1} MB/s)",
            total_downloaded / 1024 / 1024,
            newly_downloaded / 1024 / 1024,
            duration.as_secs_f64(),
            (newly_downloaded as f64 / duration.as_secs_f64()) / 1024.0 / 1024.0);

        Ok(stats)
    }
}

#[async_trait]
impl Downloader for HttpDownloader {
    fn protocol(&self) -> &str {
        "http"
    }

    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // Try to get file size and check for parallel support
        let size = self.get_size(url).await.ok().flatten();
        let supports_ranges = self.supports_ranges(url).await.unwrap_or(false);

        eprintln!("📊 Download analysis:");
        eprintln!("   - File size from HEAD: {} MB", size.map(|s| s / 1024 / 1024).unwrap_or(0));
        eprintln!("   - Concurrent fragments: {}", self.config.concurrent_fragments);
        eprintln!("   - Server supports ranges: {supports_ranges}");

        // If HEAD didn't return valid size, try a small Range request to get it
        let size = if (size.is_none() || size == Some(0)) && supports_ranges {
            eprintln!("   - HEAD didn't return valid size, trying Range request...");
            match retry_with_backoff(&self.config.retry_config, "HTTP GET (size check)", |_attempt| {
                let client = self.client.clone();
                let url = url.to_string();
                async move {
                    client
                        .get(&url)
                        .header("Range", "bytes=0-0")
                        .send()
                        .await
                        .map_err(|e| RdlpError::Network(format!("Failed to check size: {e}")))
                }
            })
            .await
            {
                Ok(response) => {
                    // Parse Content-Range: bytes 0-0/618618881
                    if let Some(content_range) = response.headers().get("content-range") {
                        if let Ok(range_str) = content_range.to_str() {
                            if let Some(total_str) = range_str.split('/').nth(1) {
                                let detected_size = total_str.parse::<u64>().ok();
                                eprintln!("   - Detected size from Range: {} MB", detected_size.map(|s| s / 1024 / 1024).unwrap_or(0));
                                detected_size
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            size
        };

        // Check if we have a valid size and should use parallel
        let use_parallel = match size {
            Some(s) if s > 10 * 1024 * 1024 => {
                self.config.concurrent_fragments > 1 && supports_ranges
            }
            _ => false,
        };

        if use_parallel {
            eprintln!("🚀 Using parallel download mode ({} connections)", self.config.concurrent_fragments);
            return self.download_parallel(url, path, size.unwrap(), progress).await;
        } else {
            // Use match with guards for cleaner Option handling (no unwrap)
            let reason = match size {
                None | Some(0) => "could not detect file size",
                Some(s) if s <= 10 * 1024 * 1024 => "file too small for parallel",
                Some(_) if self.config.concurrent_fragments <= 1 => "concurrent_fragments <= 1",
                Some(_) if !supports_ranges => "server doesn't support ranges",
                Some(_) => "unknown reason",
            };
            eprintln!("⚠️  Using sequential download - reason: {reason}");
            eprintln!("    (size: {:?} MB, fragments: {}, ranges: {supports_ranges})",
                size.map(|s| s / 1024 / 1024), self.config.concurrent_fragments);
        }

        // Fallback to sequential download
        self.download_sequential(url, path, progress).await
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn get_size(&self, url: &str) -> Result<Option<u64>> {
        let response = retry_with_backoff(&self.config.retry_config, "HTTP HEAD", |_attempt| {
            let client = self.client.clone();
            let url = url.to_string();
            async move {
                client
                    .head(&url)
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Failed to HEAD request: {e}")))
            }
        })
        .await?;

        Ok(response.content_length())
    }

    async fn download_with_resume(
        &self,
        url: &str,
        path: &Path,
        resume_from: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        let start_time = Instant::now();

        // Make request with Range header (with retry)
        let response = retry_with_backoff(&self.config.retry_config, "HTTP GET (resume)", |_attempt| {
            let client = self.client.clone();
            let url = url.to_string();
            async move {
                let response = client
                    .get(&url)
                    .header("Range", format!("bytes={resume_from}-"))
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network(format!("Failed to fetch URL: {e}")))?;

                // Check if server supports range requests
                if response.status().as_u16() != 206 {
                    // Server doesn't support partial content - DO NOT overwrite partial file
                    return Err(RdlpError::Download(format!(
                        "Server does not support resume (expected HTTP 206, got {}). \
                         Cannot continue download without overwriting existing data. \
                         Please delete the partial file and restart the download.",
                        response.status()
                    )));
                }

                Ok(response)
            }
        })
        .await?;

        // Try to get total size from Content-Range header (more reliable for resume)
        let total_size = if let Some(content_range) = response.headers().get("content-range") {
            // Content-Range: bytes 67108864-618618880/618618881
            if let Ok(range_str) = content_range.to_str() {
                if let Some(total_str) = range_str.split('/').nth(1) {
                    total_str.parse::<u64>().ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            // Fallback to Content-Length + resume_from
            response.content_length().map(|size| size + resume_from)
        };

        // Check if we should use parallel resume
        if let Some(total) = total_size {
            let progress_pct = (resume_from as f64 / total as f64) * 100.0;
            let remaining_size = total - resume_from;
            let supports_ranges = response.headers().get("accept-ranges")
                .and_then(|v| v.to_str().ok())
                .map(|v| v != "none")
                .unwrap_or(true); // Assume true if we got 206

            eprintln!("📊 Resume analysis:");
            eprintln!("   - Downloaded: {:.1}% ({} MB / {} MB)", progress_pct, resume_from / 1024 / 1024, total / 1024 / 1024);
            eprintln!("   - Remaining: {} MB", remaining_size / 1024 / 1024);
            eprintln!("   - Concurrent fragments: {}", self.config.concurrent_fragments);
            eprintln!("   - Server supports ranges: {supports_ranges}");

            let can_parallel = remaining_size > 10 * 1024 * 1024 // > 10MB remaining for parallel to be worth it
                && self.config.concurrent_fragments > 1
                && supports_ranges;

            // Use parallel resume if remaining size is large enough
            if can_parallel {
                eprintln!("🚀 Using parallel resume mode ({} connections) for faster speed...", self.config.concurrent_fragments);
                eprintln!("   Keeping {} MB already downloaded, parallelizing remaining {} MB",
                    resume_from / 1024 / 1024, remaining_size / 1024 / 1024);

                // Close the current response
                drop(response);

                // Resume with parallel download of remaining chunks
                return self.download_parallel_resume(url, path, resume_from, total, progress).await;
            } else if !can_parallel {
                eprintln!("⚠️  Parallel resume not available (remaining: {} MB, concurrent: {}, ranges: {})",
                    remaining_size / 1024 / 1024, self.config.concurrent_fragments, supports_ranges);
                eprintln!("   Continuing with sequential resume");
            }
        }

        let content_length = response.content_length();
        let total_size = total_size.or_else(|| content_length.map(|size| size + resume_from));

        // Open file in append mode with buffered writer
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        // Download remaining data
        let mut stream = response.bytes_stream();
        let mut downloaded = resume_from;
        let mut last_update = Instant::now();
        let update_interval = Duration::from_millis(100);

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk)
                .await
                .map_err(RdlpError::Io)?;

            downloaded += chunk.len() as u64;

            // Update progress
            if let Some(ref callback) = progress {
                let now = Instant::now();
                if now.duration_since(last_update) >= update_interval {
                    let elapsed = now.duration_since(start_time).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        (downloaded - resume_from) as f64 / elapsed
                    } else {
                        0.0
                    };

                    let progress_info = DownloadProgress::new(downloaded, total_size, speed);
                    callback.on_progress(&progress_info);
                    last_update = now;
                }
            }
        }

        writer.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(downloaded, duration, 0);

        if let Some(callback) = progress {
            callback.on_complete(&stats);
        }

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[allow(dead_code)]
    struct TestProgressCallback {
        updates: Arc<Mutex<Vec<u64>>>,
    }

    impl ProgressCallback for TestProgressCallback {
        fn on_progress(&self, progress: &DownloadProgress) {
            self.updates.lock().unwrap().push(progress.bytes_downloaded);
        }

        fn on_complete(&self, _stats: &DownloadStats) {
            // Test completion
        }

        fn on_error(&self, _error: &str) {
            // Test error
        }
    }

    #[tokio::test]
    async fn test_http_downloader_creation() {
        let downloader = HttpDownloader::new();
        assert_eq!(downloader.protocol(), "http");
        assert!(downloader.supports("http://example.com/video.mp4"));
        assert!(downloader.supports("https://example.com/video.mp4"));
        assert!(!downloader.supports("ftp://example.com/video.mp4"));
    }

    #[tokio::test]
    async fn test_buffer_size_configuration() {
        let downloader = HttpDownloader::new().with_buffer_size(16384);
        assert_eq!(downloader.config.buffer_size, 16384);
    }

    #[tokio::test]
    async fn test_resume_fails_when_server_returns_200() {
        use mockito::Server;
        use tempfile::NamedTempFile;

        let mut server = Server::new_async().await;

        // Mock endpoint that doesn't support Range requests (returns 200 instead of 206)
        let mock = server.mock("GET", "/video.mp4")
            .match_header("Range", "bytes=1000-")
            .with_status(200)  // Server ignores Range header
            .with_header("content-type", "video/mp4")
            .with_body("full content from beginning")
            .create_async()
            .await;

        // Create downloader with NO retries for this test (testing resume failure, not retry)
        let retry_config = RetryConfig::new(
            0,  // No retries
            Duration::from_millis(100),
            Duration::from_millis(100),
            2.0,
        );
        let downloader = HttpDownloader::new().with_retry_config(retry_config);
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Write some initial data to simulate a partial download
        tokio::fs::write(path, b"partial data").await.unwrap();

        // Try to resume - should fail with error, NOT overwrite the file
        let result = downloader
            .download_with_resume(&format!("{}/video.mp4", server.url()), path, 1000, None)
            .await;

        mock.assert_async().await;

        // Should return an error
        assert!(result.is_err());

        // Verify the error message mentions resume not supported
        let err = result.unwrap_err();
        assert!(matches!(err, RdlpError::Download(_)));
        assert!(err.to_string().contains("does not support resume"));
        assert!(err.to_string().contains("206"));

        // CRITICAL: Verify the partial file was NOT overwritten
        let file_contents = tokio::fs::read(path).await.unwrap();
        assert_eq!(file_contents, b"partial data", "Partial file should not be overwritten");
    }

    #[tokio::test]
    async fn test_parallel_download_error_propagation() {
        use mockito::Server;
        use tempfile::TempDir;

        let mut server = Server::new_async().await;
        let temp_dir = TempDir::new().unwrap();

        // Create test content (20 MB to trigger parallel mode)
        let chunk_size = 5 * 1024 * 1024; // 5 MB per chunk
        let total_size = 20 * 1024 * 1024; // 20 MB total
        let test_content = vec![0u8; chunk_size];

        // Mock HEAD request for size detection (allow multiple calls)
        let mock_head = server.mock("HEAD", "/test-video.mp4")
            .with_status(200)
            .with_header("Accept-Ranges", "bytes")
            .with_header("Content-Length", &total_size.to_string())
            .expect_at_least(1)
            .create_async()
            .await;

        // Mock Range request for size fallback detection
        let _mock_size_check = server.mock("GET", "/test-video.mp4")
            .match_header("range", "bytes=0-0")
            .with_status(206)
            .with_header("Content-Range", &format!("bytes 0-0/{total_size}"))
            .expect_at_most(1)
            .create_async()
            .await;

        // Mock successful Range requests for first 2 chunks
        let _mock_chunk_0 = server.mock("GET", "/test-video.mp4")
            .match_header("range", "bytes=0-5242879")
            .with_status(206)
            .with_header("content-range", "bytes 0-5242879/20971520")
            .with_body(&test_content)
            .create_async()
            .await;

        let _mock_chunk_1 = server.mock("GET", "/test-video.mp4")
            .match_header("range", "bytes=5242880-10485759")
            .with_status(206)
            .with_header("content-range", "bytes 5242880-10485759/20971520")
            .with_body(&test_content)
            .create_async()
            .await;

        // Mock FAILURE for chunk 2 (this should trigger fail-fast)
        let mock_chunk_2_fail = server.mock("GET", "/test-video.mp4")
            .match_header("range", "bytes=10485760-15728639")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        // Chunk 3 should be cancelled by try_join_all after chunk 2 fails
        // It might or might not be called depending on timing
        let _mock_chunk_3 = server.mock("GET", "/test-video.mp4")
            .match_header("range", "bytes=15728640-20971519")
            .with_status(206)
            .with_header("content-range", "bytes 15728640-20971519/20971520")
            .with_body(&test_content)
            .expect_at_most(1) // May not be called if cancelled early
            .create_async()
            .await;

        // Create downloader with 4 concurrent fragments and no retries for fast test
        // Use Legacy chunking strategy to maintain 4 large chunks for this test
        use rdlp_core::RetryConfig;
        use std::time::Duration;
        use crate::chunking::ChunkSizeStrategy;
        let no_retry_config = RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);

        let downloader = HttpDownloader::new()
            .with_concurrent_fragments(4)
            .with_retry_config(no_retry_config)
            .with_buffer_size(1024 * 1024) // 1 MB buffer
            .with_chunk_strategy(ChunkSizeStrategy::Legacy { chunk_count: 4 });

        let url = format!("{}/test-video.mp4", server.url());
        let output = temp_dir.path().join("output.mp4");

        // Attempt download - should fail on chunk 2
        let result = downloader.download_to_file(&url, &output, None).await;

        // Verify failure
        assert!(result.is_err(), "Download should fail due to chunk 2 error");

        let err = result.unwrap_err();
        assert!(matches!(err, RdlpError::Network(_)), "Should be a network error");

        // Verify mocks were called appropriately
        mock_head.assert_async().await;

        // Chunks 0 and 1 should have been called (they succeed)
        // Note: The exact order depends on async scheduling, but try_join_all
        // will stop all futures on first error

        // Chunk 2 (the failing one) should definitely have been called
        mock_chunk_2_fail.assert_async().await;

        // This test demonstrates that try_join_all provides fail-fast behavior:
        // When chunk 2 fails, the entire operation fails immediately and returns
        // the error, rather than waiting for all chunks to complete.
    }

    #[tokio::test]
    async fn test_parallel_resume() {
        use mockito::Server;
        use tempfile::TempDir;

        let mut server = Server::new_async().await;
        let temp_dir = TempDir::new().unwrap();

        // Simulate a 20 MB file with 5 MB already downloaded (25%)
        let total_size = 20 * 1024 * 1024; // 20 MB
        let already_downloaded = 5 * 1024 * 1024; // 5 MB
        let remaining_size = total_size - already_downloaded; // 15 MB

        // Create partial file with 5 MB of data
        let output = temp_dir.path().join("video.mp4");
        tokio::fs::write(&output, vec![0xAA; already_downloaded as usize])
            .await
            .unwrap();

        // Mock Range request for resume (server returns 206 with Content-Range)
        let _mock_resume = server.mock("GET", "/video.mp4")
            .match_header("range", "bytes=5242880-")
            .with_status(206)
            .with_header("Content-Range", &format!("bytes 5242880-20971519/{total_size}"))
            .with_header("Accept-Ranges", "bytes")
            .with_body(vec![0xBB; remaining_size as usize]) // Dummy data
            .expect(0) // Should NOT be called - we use parallel resume instead
            .create_async()
            .await;

        // Mock parallel resume chunks (4 chunks for 15 MB remaining)
        // Chunk 0: 5 MB - 8.75 MB (3.75 MB)
        let _mock_chunk_0 = server.mock("GET", "/video.mp4")
            .match_header("range", "bytes=5242880-9175039")
            .with_status(206)
            .with_header("Content-Range", "bytes 5242880-9175039/20971520")
            .with_body(vec![0xCC; (9175040 - 5242880) as usize])
            .create_async()
            .await;

        // Chunk 1: 8.75 MB - 12.5 MB (3.75 MB)
        let _mock_chunk_1 = server.mock("GET", "/video.mp4")
            .match_header("range", "bytes=9175040-13107199")
            .with_status(206)
            .with_header("Content-Range", "bytes 9175040-13107199/20971520")
            .with_body(vec![0xDD; (13107200 - 9175040) as usize])
            .create_async()
            .await;

        // Chunk 2: 12.5 MB - 16.25 MB (3.75 MB)
        let _mock_chunk_2 = server.mock("GET", "/video.mp4")
            .match_header("range", "bytes=13107200-17039359")
            .with_status(206)
            .with_header("Content-Range", "bytes 13107200-17039359/20971520")
            .with_body(vec![0xEE; (17039360 - 13107200) as usize])
            .create_async()
            .await;

        // Chunk 3: 16.25 MB - 20 MB (3.75 MB)
        let _mock_chunk_3 = server.mock("GET", "/video.mp4")
            .match_header("range", "bytes=17039360-20971519")
            .with_status(206)
            .with_header("Content-Range", "bytes 17039360-20971519/20971520")
            .with_body(vec![0xFF; (20971520 - 17039360) as usize])
            .create_async()
            .await;

        // Create downloader with 4 concurrent fragments and no retries
        // Use Legacy chunking strategy to maintain 4 large chunks for this test
        use rdlp_core::RetryConfig;
        use std::time::Duration;
        use crate::chunking::ChunkSizeStrategy;
        let no_retry_config = RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);

        let downloader = HttpDownloader::new()
            .with_concurrent_fragments(4)
            .with_retry_config(no_retry_config)
            .with_chunk_strategy(ChunkSizeStrategy::Legacy { chunk_count: 4 });

        let url = format!("{}/video.mp4", server.url());

        // Resume download - should use parallel resume
        let result = downloader.download_with_resume(&url, &output, already_downloaded, None).await;

        // Verify success
        assert!(result.is_ok(), "Parallel resume should succeed");
        let stats = result.unwrap();
        assert_eq!(stats.bytes_downloaded, total_size, "Should download full file size");

        // Verify file size
        let metadata = tokio::fs::metadata(&output).await.unwrap();
        assert_eq!(metadata.len(), total_size, "Final file should be 20 MB");

        // Verify file contents structure:
        // First 5 MB: 0xAA (already downloaded)
        // Next 3.75 MB: 0xCC (chunk 0)
        // Next 3.75 MB: 0xDD (chunk 1)
        // Next 3.75 MB: 0xEE (chunk 2)
        // Last 3.75 MB: 0xFF (chunk 3)
        let contents = tokio::fs::read(&output).await.unwrap();
        assert_eq!(contents.len(), total_size as usize);

        // Check first 5 MB is original data (0xAA)
        assert_eq!(contents[0], 0xAA, "First byte should be from original partial download");
        assert_eq!(contents[already_downloaded as usize - 1], 0xAA, "Last byte of partial should be 0xAA");

        // Check first byte of resumed data (chunk 0 starts with 0xCC)
        assert_eq!(contents[already_downloaded as usize], 0xCC, "First resumed byte should be from chunk 0");
    }
}
