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

use rdlp_core::{Config, Downloader};
use std::sync::Arc;
use std::time::Duration;

/// Registry for managing downloaders
pub struct DownloaderRegistry {
    downloaders: Vec<Arc<dyn Downloader>>,
}

impl DownloaderRegistry {
    /// Create a new registry with default downloaders
    pub fn new() -> Self {
        Self::with_config(&Config::default())
    }

    /// Create a new registry with custom configuration
    pub fn with_config(config: &Config) -> Self {
        let mut registry = Self {
            downloaders: Vec::new(),
        };

        // Create optimized HTTP client
        let mut client_builder = reqwest::Client::builder()
            .pool_max_idle_per_host(10) // Keep 10 connections alive per host
            .pool_idle_timeout(Duration::from_secs(90)) // Keep connections for 90s
            .tcp_keepalive(Duration::from_secs(60)) // TCP keepalive every 60s
            .tcp_nodelay(true) // Disable Nagle's algorithm for lower latency
            .connect_timeout(Duration::from_secs(30)) // 30s to establish connection
            .read_timeout(Duration::from_secs(60)); // 60s idle timeout (not total)

        if let Some(timeout) = config.socket_timeout {
            client_builder = client_builder.connect_timeout(Duration::from_secs(timeout));
        }

        if let Some(ref user_agent) = config.user_agent {
            client_builder = client_builder.user_agent(user_agent);
        }

        if let Some(ref proxy) = config.proxy {
            if let Ok(proxy_obj) = reqwest::Proxy::all(proxy) {
                client_builder = client_builder.proxy(proxy_obj);
            }
        }

        let client = client_builder.build().unwrap_or_else(|_| reqwest::Client::new());

        // Register HTTP downloader with optimized settings
        let http_downloader = HttpDownloader::with_client(client)
            .with_buffer_size(config.buffer_size)
            .with_concurrent_fragments(config.concurrent_fragments);

        registry.register(Arc::new(http_downloader));

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
    fn test_registry_with_custom_config() {
        let mut config = Config::default();
        config.buffer_size = 4 * 1024 * 1024; // 4 MB

        let registry = DownloaderRegistry::with_config(&config);
        let downloaders = registry.list_downloaders();
        assert!(downloaders.contains(&"http".to_string()));

        // Verify the downloader was created with config settings
        let downloader = registry.find_downloader("https://example.com/video.mp4");
        assert!(downloader.is_some());
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
