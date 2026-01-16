use async_trait::async_trait;
use futures::StreamExt;
use rdlp_core::{check_http_response, retry_with_backoff, DownloadProgress, DownloadStats, Downloader, ProgressCallback, Result, RetryConfig, RdlpError};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

/// HTTP/HTTPS downloader
pub struct HttpDownloader {
    client: reqwest::Client,
    buffer_size: usize,
    retry_config: RetryConfig,
    concurrent_fragments: usize,
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
            buffer_size: 8192, // 8KB buffer
            retry_config: RetryConfig::default_config(),
            concurrent_fragments: 4, // Default to 4 parallel connections
        }
    }

    /// Set buffer size for downloads
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Set retry configuration
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Set number of concurrent fragment downloads
    pub fn with_concurrent_fragments(mut self, count: usize) -> Self {
        self.concurrent_fragments = count.max(1); // At least 1
        self
    }

    /// Check if server supports range requests
    async fn supports_ranges(&self, url: &str) -> Result<bool> {
        let response = retry_with_backoff(&self.retry_config, "HTTP HEAD (range check)", |_attempt| {
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

    /// Download a specific byte range with progress tracking
    async fn download_range_with_progress(
        &self,
        url: &str,
        start: u64,
        end: u64,
        chunk_path: &Path,
        progress_counter: Option<std::sync::Arc<tokio::sync::Mutex<u64>>>,
    ) -> Result<u64> {
        let response = retry_with_backoff(&self.retry_config, "HTTP GET (range)", |_attempt| {
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
        let mut writer = BufWriter::with_capacity(self.buffer_size, file);

        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            writer.write_all(&chunk).await.map_err(RdlpError::Io)?;
            downloaded += chunk.len() as u64;

            // Update shared progress counter in real-time
            if let Some(ref counter) = progress_counter {
                let mut total = counter.lock().await;
                *total += chunk.len() as u64;
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
        let response = retry_with_backoff(&self.retry_config, "HTTP GET", |_attempt| {
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
        let mut writer = BufWriter::with_capacity(self.buffer_size, file);

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

    /// Parallel download using multiple range requests
    async fn download_parallel(
        &self,
        url: &str,
        path: &Path,
        total_size: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let start_time = Instant::now();
        let chunk_count = self.concurrent_fragments.min(10); // Max 10 connections
        let chunk_size = total_size / chunk_count as u64;

        // Create temporary directory for chunks
        let temp_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let chunk_paths: Vec<_> = (0..chunk_count)
            .map(|i| temp_dir.join(format!("{}.part{}", path.file_name().unwrap().to_string_lossy(), i)))
            .collect();

        // Shared progress counter
        let downloaded = Arc::new(Mutex::new(0u64));
        let mut tasks = vec![];

        // Spawn download tasks for each chunk
        eprintln!("🚀 Starting {} parallel downloads...", chunk_count);
        for (i, chunk_path) in chunk_paths.iter().enumerate() {
            let start = i as u64 * chunk_size;
            let end = if i == chunk_count - 1 {
                total_size - 1 // Last chunk gets remainder
            } else {
                (i + 1) as u64 * chunk_size - 1
            };

            eprintln!("   Chunk {}: {} MB - {} MB ({} MB)",
                i, start / 1024 / 1024, end / 1024 / 1024, (end - start + 1) / 1024 / 1024);

            let downloader = Self {
                client: self.client.clone(),
                buffer_size: self.buffer_size,
                retry_config: self.retry_config.clone(),
                concurrent_fragments: 1, // No recursion
            };
            let url = url.to_string();
            let chunk_path = chunk_path.clone();
            let progress_counter = Some(downloaded.clone());
            let chunk_id = i;

            let task = tokio::spawn(async move {
                eprintln!("📥 Starting chunk {}...", chunk_id);
                let result = downloader.download_range_with_progress(&url, start, end, &chunk_path, progress_counter).await;
                match &result {
                    Ok(_) => eprintln!("✅ Chunk {} complete", chunk_id),
                    Err(e) => eprintln!("❌ Chunk {} failed: {}", chunk_id, e),
                }
                result
            });

            tasks.push(task);
        }

        // Progress reporter task
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
                        let bytes_downloaded = *downloaded_clone.lock().await;
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
        };

        // Wait for all chunks to complete
        let mut total_downloaded = 0u64;
        for (i, task) in tasks.into_iter().enumerate() {
            match task.await {
                Ok(Ok(bytes)) => {
                    total_downloaded += bytes;
                    eprintln!("✓ Chunk {} completed ({} MB)", i, bytes / 1024 / 1024);
                    // Progress is already updated in real-time by download_range_with_progress
                }
                Ok(Err(e)) => {
                    eprintln!("❌ Chunk {} failed: {}", i, e);
                    // Don't clean up - keep chunks for debugging
                    return Err(RdlpError::Download(format!("Parallel download failed on chunk {}: {}", i, e)));
                }
                Err(e) => {
                    eprintln!("❌ Chunk {} task panicked: {}", i, e);
                    // Don't clean up - keep chunks for debugging
                    return Err(RdlpError::Download(format!("Task {i} panicked: {e}")));
                }
            }
        }

        // Stop progress reporter
        if let Some(task) = progress_task {
            task.abort();
        }

        // Merge chunks into final file
        let final_file = File::create(path).await.map_err(RdlpError::Io)?;
        let mut writer = BufWriter::with_capacity(self.buffer_size, final_file);

        for chunk_path in &chunk_paths {
            let mut chunk_file = File::open(chunk_path).await.map_err(RdlpError::Io)?;
            tokio::io::copy(&mut chunk_file, &mut writer)
                .await
                .map_err(RdlpError::Io)?;

            // Delete chunk file after merging
            tokio::fs::remove_file(chunk_path).await.map_err(RdlpError::Io)?;
        }

        writer.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(total_downloaded, duration, chunk_count);

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
        eprintln!("   - Concurrent fragments: {}", self.concurrent_fragments);
        eprintln!("   - Server supports ranges: {}", supports_ranges);

        // If HEAD didn't return valid size, try a small Range request to get it
        let size = if (size.is_none() || size == Some(0)) && supports_ranges {
            eprintln!("   - HEAD didn't return valid size, trying Range request...");
            match retry_with_backoff(&self.retry_config, "HTTP GET (size check)", |_attempt| {
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
                self.concurrent_fragments > 1 && supports_ranges
            }
            _ => false,
        };

        if use_parallel {
            eprintln!("🚀 Using parallel download mode ({} connections)", self.concurrent_fragments);
            return self.download_parallel(url, path, size.unwrap(), progress).await;
        } else {
            let reason = if size.is_none() || size == Some(0) {
                "could not detect file size"
            } else if size.unwrap() <= 10 * 1024 * 1024 {
                "file too small for parallel"
            } else if self.concurrent_fragments <= 1 {
                "concurrent_fragments <= 1"
            } else if !supports_ranges {
                "server doesn't support ranges"
            } else {
                "unknown reason"
            };
            eprintln!("⚠️  Using sequential download - reason: {}", reason);
            eprintln!("    (size: {:?} MB, fragments: {}, ranges: {})",
                size.map(|s| s / 1024 / 1024), self.concurrent_fragments, supports_ranges);
        }

        // Fallback to sequential download
        self.download_sequential(url, path, progress).await
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn get_size(&self, url: &str) -> Result<Option<u64>> {
        let response = retry_with_backoff(&self.retry_config, "HTTP HEAD", |_attempt| {
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
        let response = retry_with_backoff(&self.retry_config, "HTTP GET (resume)", |_attempt| {
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

        // Check if we should switch to parallel download
        if let Some(total) = total_size {
            let progress_pct = (resume_from as f64 / total as f64) * 100.0;
            let supports_ranges = response.headers().get("accept-ranges")
                .and_then(|v| v.to_str().ok())
                .map(|v| v != "none")
                .unwrap_or(true); // Assume true if we got 206

            eprintln!("📊 Resume analysis:");
            eprintln!("   - Downloaded: {:.1}% ({} MB / {} MB)", progress_pct, resume_from / 1024 / 1024, total / 1024 / 1024);
            eprintln!("   - Concurrent fragments: {}", self.concurrent_fragments);
            eprintln!("   - Server supports ranges: {}", supports_ranges);

            let can_parallel = total > 10 * 1024 * 1024 // > 10MB for parallel to be worth it
                && self.concurrent_fragments > 1
                && supports_ranges;

            // If we can use parallel and haven't downloaded much yet (< 20%), restart with parallel
            if can_parallel && progress_pct < 20.0 {
                eprintln!("🚀 Switching to parallel download mode ({} connections) for faster speed...", self.concurrent_fragments);
                eprintln!("   Discarding {} MB partial download to enable parallel mode", resume_from / 1024 / 1024);

                // Close the current response
                drop(response);

                // Delete partial file and start fresh with parallel download
                let _ = tokio::fs::remove_file(path).await;
                return self.download_parallel(url, path, total, progress).await;
            } else if !can_parallel {
                eprintln!("⚠️  Parallel mode not available (file size: {} MB, concurrent: {}, ranges: {})",
                    total / 1024 / 1024, self.concurrent_fragments, supports_ranges);
            } else {
                eprintln!("⏩ Too much downloaded ({:.1}%), continuing with sequential resume", progress_pct);
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
        let mut writer = BufWriter::with_capacity(self.buffer_size, file);

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
        assert_eq!(downloader.buffer_size, 16384);
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
}
