use async_trait::async_trait;
use futures::StreamExt;
use rdlp_core::{check_http_response, DownloadProgress, DownloadStats, Downloader, ProgressCallback, Result, RdlpError};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

/// HTTP/HTTPS downloader
pub struct HttpDownloader {
    client: reqwest::Client,
    buffer_size: usize,
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
        }
    }

    /// Set buffer size for downloads
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
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
        let start_time = Instant::now();

        // Make the request
        let response = self.client
            .get(url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch URL: {e}")))?;

        // Check status
        check_http_response(&response)?;

        // Get content length if available
        let total_size = response.content_length();

        // Create output file
        let mut file = File::create(path)
            .await
            .map_err(RdlpError::Io)?;

        // Download with progress tracking
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_update = Instant::now();
        let update_interval = Duration::from_millis(100); // Update every 100ms

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            file.write_all(&chunk)
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

        // Ensure all data is written
        file.flush().await.map_err(RdlpError::Io)?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(downloaded, duration, 0);

        // Notify completion
        if let Some(callback) = progress {
            callback.on_complete(&stats);
        }

        Ok(stats)
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn get_size(&self, url: &str) -> Result<Option<u64>> {
        let response = self.client
            .head(url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to HEAD request: {e}")))?;

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

        // Make request with Range header
        let response = self.client
            .get(url)
            .header("Range", format!("bytes={resume_from}-"))
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch URL: {e}")))?;

        // Check if server supports range requests
        if response.status().as_u16() != 206 {
            // Server doesn't support partial content, download from beginning
            return self.download_to_file(url, path, progress).await;
        }

        let content_length = response.content_length();
        let total_size = content_length.map(|size| size + resume_from);

        // Open file in append mode
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .map_err(RdlpError::Io)?;

        // Download remaining data
        let mut stream = response.bytes_stream();
        let mut downloaded = resume_from;
        let mut last_update = Instant::now();
        let update_interval = Duration::from_millis(100);

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| RdlpError::Network(format!("Failed to read chunk: {e}")))?;

            file.write_all(&chunk)
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

        file.flush().await.map_err(RdlpError::Io)?;

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
}
