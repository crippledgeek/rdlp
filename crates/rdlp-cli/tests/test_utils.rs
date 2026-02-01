//! Test utilities for orchestrator integration tests

use async_trait::async_trait;
use rdlp_core::{
    DownloadProgress, DownloadStats, Downloader, Format, InfoDict, InfoExtractor, ProgressCallback,
    RdlpError,
};
use rdlp_downloader::DownloaderRegistryTrait;
use rdlp_extractor::ExtractorRegistryTrait;
use regex::Regex;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

type Result<T> = std::result::Result<T, RdlpError>;

/// Mock extractor for testing
pub struct MockExtractor {
    name: String,
    url_pattern: String,
    url_regex: Regex,
    info_dict: InfoDict,
}

impl MockExtractor {
    pub fn new(
        name: impl Into<String>,
        url_pattern: impl Into<String>,
        info_dict: InfoDict,
    ) -> Self {
        let url_pattern_str = url_pattern.into();
        // Create a simple regex that matches if the URL contains the pattern
        // Escape special regex characters
        let escaped = url_pattern_str.replace(".", "\\.");
        let url_regex = Regex::new(&format!(".*{escaped}.*")).unwrap();

        Self {
            name: name.into(),
            url_pattern: url_pattern_str,
            url_regex,
            info_dict,
        }
    }
}

#[async_trait]
impl InfoExtractor for MockExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn valid_url(&self) -> &Regex {
        &self.url_regex
    }

    async fn extract(
        &self,
        _url: &str,
        _context: &rdlp_core::ExtractionContext,
    ) -> Result<InfoDict> {
        Ok(self.info_dict.clone())
    }

    fn suitable(&self, url: &str) -> bool {
        url.contains(&self.url_pattern)
    }

    fn priority(&self) -> i32 {
        100
    }
}

/// Mock extractor registry for testing
pub struct MockExtractorRegistry {
    extractors: Vec<Arc<dyn InfoExtractor>>,
}

impl Default for MockExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MockExtractorRegistry {
    pub fn new() -> Self {
        Self {
            extractors: Vec::new(),
        }
    }

    pub fn register(&mut self, extractor: Arc<dyn InfoExtractor>) {
        self.extractors.push(extractor);
    }
}

impl ExtractorRegistryTrait for MockExtractorRegistry {
    fn find_extractor(&self, url: &str) -> Option<Arc<dyn InfoExtractor>> {
        self.extractors
            .iter()
            .filter(|e| e.suitable(url))
            .max_by_key(|e| e.priority())
            .cloned()
    }

    fn list_extractors(&self) -> Vec<&str> {
        self.extractors.iter().map(|e| e.name()).collect()
    }
}

/// Mock downloader for testing
pub struct MockDownloader {
    protocol: String,
    should_succeed: bool,
    download_size: u64,
    download_delay_ms: u64,
}

impl MockDownloader {
    pub fn new(
        protocol: impl Into<String>,
        should_succeed: bool,
        download_size: u64,
        download_delay_ms: u64,
    ) -> Self {
        Self {
            protocol: protocol.into(),
            should_succeed,
            download_size,
            download_delay_ms,
        }
    }
}

#[async_trait]
impl Downloader for MockDownloader {
    fn protocol(&self) -> &str {
        &self.protocol
    }

    async fn download_to_file(
        &self,
        _url: &str,
        output_path: &Path,
        progress_callback: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        if !self.should_succeed {
            return Err(RdlpError::Download("Mock download failed".to_string()));
        }

        // Simulate download delay
        if self.download_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.download_delay_ms)).await;
        }

        // Create output file with mock content
        let content = vec![0u8; self.download_size as usize];
        tokio::fs::write(output_path, &content).await?;

        // Report progress if callback provided
        if let Some(callback) = progress_callback {
            let progress = DownloadProgress {
                bytes_downloaded: self.download_size,
                total_bytes: Some(self.download_size),
                speed: 0.0,
                eta: None,
                percentage: Some(100.0),
                segments_downloaded: None,
                total_segments: None,
                duration_downloaded: None,
                total_duration: None,
            };
            callback.on_progress(&progress);
        }

        Ok(DownloadStats::new(
            self.download_size,
            Duration::from_millis(self.download_delay_ms),
            0, // no retries
        ))
    }

    fn supports(&self, url: &str) -> bool {
        url.starts_with(&format!("{}://", self.protocol))
    }

    async fn get_size(&self, _url: &str) -> Result<Option<u64>> {
        Ok(Some(self.download_size))
    }

    async fn download_with_resume(
        &self,
        _url: &str,
        output_path: &Path,
        resume_from: u64,
        progress_callback: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        if !self.should_succeed {
            return Err(RdlpError::Download(
                "Mock download with resume failed".to_string(),
            ));
        }

        // Simulate download delay
        if self.download_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.download_delay_ms)).await;
        }

        // Read existing content or create new
        let mut content = tokio::fs::read(output_path)
            .await
            .unwrap_or_else(|_| Vec::new());

        // Extend content to full size
        let remaining = self.download_size.saturating_sub(resume_from);
        content.extend(vec![0u8; remaining as usize]);
        tokio::fs::write(output_path, &content).await?;

        // Report progress if callback provided
        if let Some(callback) = progress_callback {
            let progress = DownloadProgress {
                bytes_downloaded: self.download_size,
                total_bytes: Some(self.download_size),
                speed: 0.0,
                eta: None,
                percentage: Some(100.0),
                segments_downloaded: None,
                total_segments: None,
                duration_downloaded: None,
                total_duration: None,
            };
            callback.on_progress(&progress);
        }

        Ok(DownloadStats::new(
            remaining,
            Duration::from_millis(self.download_delay_ms),
            0, // no retries
        ))
    }
}

/// Mock downloader registry for testing
pub struct MockDownloaderRegistry {
    downloaders: Vec<Arc<dyn Downloader>>,
}

impl Default for MockDownloaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MockDownloaderRegistry {
    pub fn new() -> Self {
        Self {
            downloaders: Vec::new(),
        }
    }

    pub fn register(&mut self, downloader: Arc<dyn Downloader>) {
        self.downloaders.push(downloader);
    }
}

impl DownloaderRegistryTrait for MockDownloaderRegistry {
    fn find_downloader(&self, url: &str) -> Option<Arc<dyn Downloader>> {
        self.downloaders.iter().find(|d| d.supports(url)).cloned()
    }

    fn list_downloaders(&self) -> Vec<&str> {
        self.downloaders.iter().map(|d| d.protocol()).collect()
    }
}

/// Helper to create a test InfoDict
pub fn create_test_info_dict(title: impl Into<String>, url: impl Into<String>) -> InfoDict {
    let formats = vec![
        Format::new(
            "1".to_string(),
            "mock://example.com/video1.mp4".to_string(),
            "mp4".to_string(),
            rdlp_core::DownloadProtocol::Other("mock".to_string()),
        ),
        Format::new(
            "2".to_string(),
            "mock://example.com/video2.mp4".to_string(),
            "mp4".to_string(),
            rdlp_core::DownloadProtocol::Other("mock".to_string()),
        ),
    ];

    InfoDict {
        id: "test123".to_string(),
        title: title.into(),
        extractor: "MockExtractor".to_string(),
        webpage_url: url.into(),
        formats,
        description: Some("Test video description".to_string()),
        uploader: Some("Test Uploader".to_string()),
        duration: Some(120.0),
        upload_date: None,
        thumbnail: None,
        thumbnails: None,
        uploader_id: None,
        uploader_url: None,
        channel: None,
        channel_id: None,
        channel_url: None,
        view_count: None,
        like_count: None,
        dislike_count: None,
        comment_count: None,
        average_rating: None,
        age_limit: None,
        tags: None,
        categories: None,
        subtitles: None,
        automatic_captions: None,
        requested_formats: None,
        playlist: None,
        playlist_id: None,
        playlist_title: None,
        playlist_index: None,
        playlist_count: None,
        chapters: None,
        is_live: None,
        was_live: None,
        release_year: None,
        artist: None,
        album: None,
        track: None,
        extra: std::collections::HashMap::new(),
    }
}
