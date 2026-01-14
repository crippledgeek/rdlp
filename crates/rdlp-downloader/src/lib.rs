//! # rdlp-downloader
//!
//! Download protocol implementations for rdlp.
//!
//! This crate provides downloaders for various streaming protocols:
//! - HTTP/HTTPS
//! - HLS (m3u8) - Coming soon
//! - DASH - Coming soon

pub mod http;

pub use http::HttpDownloader;

use rdlp_core::Downloader;
use std::sync::Arc;

/// Registry for managing downloaders
pub struct DownloaderRegistry {
    downloaders: Vec<Arc<dyn Downloader>>,
}

impl DownloaderRegistry {
    /// Create a new registry with default downloaders
    pub fn new() -> Self {
        let mut registry = Self {
            downloaders: Vec::new(),
        };

        // Register HTTP downloader
        registry.register(Arc::new(HttpDownloader::new()));

        registry
    }

    /// Register a new downloader
    pub fn register(&mut self, downloader: Arc<dyn Downloader>) {
        self.downloaders.push(downloader);
    }

    /// Find a suitable downloader for the given URL
    pub fn find_downloader(&self, url: &str) -> Option<Arc<dyn Downloader>> {
        self.downloaders
            .iter()
            .find(|d| d.supports(url))
            .cloned()
    }

    /// Get all registered downloaders
    pub fn list_downloaders(&self) -> Vec<String> {
        self.downloaders
            .iter()
            .map(|d| d.protocol().to_string())
            .collect()
    }
}

impl Default for DownloaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = DownloaderRegistry::new();
        let downloaders = registry.list_downloaders();
        assert!(downloaders.contains(&"http".to_string()));
    }

    #[test]
    fn test_find_downloader() {
        let registry = DownloaderRegistry::new();

        let http_downloader = registry.find_downloader("https://example.com/video.mp4");
        assert!(http_downloader.is_some());
        assert_eq!(http_downloader.unwrap().protocol(), "http");

        let unknown = registry.find_downloader("rtmp://example.com/stream");
        assert!(unknown.is_none());
    }
}
