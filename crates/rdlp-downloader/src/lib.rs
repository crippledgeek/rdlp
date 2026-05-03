//! # rdlp-downloader
//!
//! High-performance download protocol implementations with intelligent chunking.
//!
//! This crate provides downloaders for various streaming protocols:
//! - **HTTP/HTTPS**: Power-of-two chunking with fine-grained parallelism
//! - **HLS (m3u8)**: Parallel segment downloads with automatic playlist parsing
//! - **DASH** (static VoD): two paths — **pre-resolved fragments** (when
//!   `Format.fragments` is `Some(...)`, the extractor already expanded the MPD
//!   into per-Representation segment lists via `expand_dash_representations`;
//!   `download_format` fetches those directly, no re-parsing; `MergeStage`
//!   handles muxing when `bv+ba` selection picks two Formats) and
//!   **legacy MPD-URL** (when `Format.fragments` is `None`, `DashDownloader`
//!   parses the MPD itself, picks max-bandwidth video+audio, fetches segments,
//!   and muxes in-process via `FFmpeg` stream-copy)
//!
//! ## Overview
//!
//! rdlp-downloader implements a 7-layer optimization stack for maximum download
//! performance:
//!
//! 1. **Power-of-Two Chunking**: Memory-aligned chunk sizes (64 KB - 8 MB)
//! 2. **Fine-Grained Parallelism**: Batch processing with `buffer_unordered`
//! 3. **Multi-Threaded Runtime**: Tokio with 2x CPU cores for I/O workloads
//! 4. **Buffered I/O**: 2 MB write buffers for reduced syscalls
//! 5. **HTTP Optimizations**: Connection pooling, `TCP_NODELAY`, keepalive
//! 6. **Intelligent Size Detection**: HEAD/Range request fallbacks
//! 7. **Real-Time Progress**: Atomic counters across all parallel chunks
//!
//! ## Features
//!
//! ### Power-of-Two Chunking
//!
//! The downloader uses an intelligent chunking algorithm that:
//! - Targets ~1024 chunks per file for optimal parallelism
//! - Aligns chunk sizes to powers of two (64 KB, 128 KB, 256 KB, ..., 8 MB)
//! - Ensures minimum 64 KB (NTFS cluster size) and maximum 8 MB chunks
//! - Optimizes for memory pages, allocators, and filesystem clusters
//!
//! **Examples**:
//! - 5 MB file → 64 KB chunks (81 chunks)
//! - 200 MB file → 256 KB chunks (800 chunks)
//! - 1 GB file → 1 MB chunks (1024 chunks)
//! - 5 GB file → 8 MB chunks (640 chunks)
//!
//! ### Fine-Grained Parallelism
//!
//! - Automatic activation for files > 10 MB with Range support
//! - Configurable concurrent connections (default: 4)
//! - Batch processing prevents overwhelming the runtime
//! - Smart resume: switches to parallel if < 20% downloaded
//!
//! ### Resume Capability
//!
//! - Detects partial downloads automatically
//! - Supports both old-style (`file.part0`) and new-style (`file.0.part0`) chunks
//! - Automatic chunk merging and cleanup
//! - Prioritizes most recent download attempt (highest ID)
//!
//! ### Progress Tracking
//!
//! - Atomic shared counter across all chunks
//! - Real-time updates every 100ms
//! - Accurate speed and ETA calculations
//! - Transparent cleanup logging
//!
//! ## Performance
//!
//! **Benchmark**: 590 MB file download from `TNAFlix`
//!
//! | Optimization Level | Time | Speed | Improvement |
//! |-------------------|------|-------|-------------|
//! | Baseline (8KB buffer) | 35+ min | ~360 KB/s | 1x |
//! | + Buffered I/O (2MB) | ~15 min | ~650 KB/s | 2x |
//! | + Connection pooling | ~12 min | ~820 KB/s | 2.5x |
//! | + Parallel chunks (4) | ~9 min | ~1.1 MB/s | 3x |
//! | + Multi-threaded | ~6-8 min | ~1.5 MB/s | 4-5x |
//! | **+ Power-of-two** | **56.4s** | **10.5 MB/s** | **37x** |
//!
//! **Note**: Actual speeds depend on server throttling and network conditions.
//!
//! ## Examples
//!
//! ### Basic Usage
//!
//! ```rust,no_run
//! use rdlp_downloader::HttpDownloader;
//! use rdlp_core::Downloader;
//! use std::path::Path;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let downloader = HttpDownloader::new();
//!
//! downloader.download_to_file(
//!     "https://example.com/video.mp4",
//!     Path::new("video.mp4"),
//!     None
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Custom Chunk Strategy
//!
//! ```rust,no_run
//! use rdlp_downloader::{HttpDownloader, ChunkSizeStrategy};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Automatic power-of-two sizing (recommended)
//! let downloader = HttpDownloader::new()
//!     .with_chunk_strategy(ChunkSizeStrategy::Auto);
//!
//! // Fixed 1 MB chunks (must be power of two)
//! let downloader = HttpDownloader::new()
//!     .with_chunk_strategy(ChunkSizeStrategy::Fixed(1024 * 1024));
//!
//! // Legacy coarse-grained chunking (backward compatibility)
//! let downloader = HttpDownloader::new()
//!     .with_chunk_strategy(ChunkSizeStrategy::Legacy { chunk_count: 4 });
//! # Ok(())
//! # }
//! ```
//!
//! ### Using `DownloaderRegistry`
//!
//! ```rust,no_run
//! use rdlp_downloader::DownloaderRegistry;
//! use rdlp_types::Config;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create registry with custom configuration
//! let config = Config {
//!     concurrent_fragments: 8,
//!     buffer_size: 4 * 1024 * 1024, // 4 MB
//!     ..Default::default()
//! };
//!
//! let registry = DownloaderRegistry::with_config(&config);
//!
//! // Find appropriate downloader for URL
//! if let Some(downloader) = registry.find_downloader("https://example.com/video.mp4") {
//!     println!("Using {} downloader", downloader.protocol());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Progress Tracking
//!
//! ```rust,no_run
//! use rdlp_downloader::HttpDownloader;
//! use rdlp_core::{Downloader, ProgressCallback, DownloadProgress};
//!
//! struct MyProgress;
//! impl ProgressCallback for MyProgress {
//!     fn on_progress(&self, info: &DownloadProgress) {
//!         println!("Downloaded: {:.2} MB", info.bytes_downloaded as f64 / (1024.0 * 1024.0));
//!     }
//!     fn on_complete(&self, _stats: &rdlp_core::DownloadStats) {}
//!     fn on_error(&self, _error: &str) {}
//! }
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let downloader = HttpDownloader::new();
//!
//! // Download with progress tracking
//! downloader.download_to_file(
//!     "https://example.com/video.mp4",
//!     "video.mp4".as_ref(),
//!     Some(Box::new(MyProgress))
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Resume Capability
//!
//! The downloader automatically detects and resumes partial downloads:
//!
//! ```rust,no_run
//! use rdlp_downloader::HttpDownloader;
//! use rdlp_core::Downloader;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let downloader = HttpDownloader::new();
//!
//! // First attempt (interrupted)
//! let _ = downloader.download_to_file(
//!     "https://example.com/large.mp4",
//!     "large.mp4".as_ref(),
//!     None
//! ).await;
//!
//! // Second attempt (automatically resumes)
//! // Will detect partial file or chunk files and continue
//! downloader.download_to_file(
//!     "https://example.com/large.mp4",
//!     "large.mp4".as_ref(),
//!     None
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Architecture
//!
//! ### HTTP Client Optimizations
//!
//! The HTTP client is configured with:
//! - **Connection pooling**: 10 connections per host, 90s idle timeout
//! - **TCP keepalive**: 60-second intervals prevent connection drops
//! - **`TCP_NODELAY`**: Disables Nagle's algorithm for lower latency
//! - **Smart timeouts**: 30s connect, 60s idle (no total time limit)
//!
//! ### Chunk File Format
//!
//! - **New-style** (Phase 2.5+): `{filename}.{downloadid}.part{i}`
//!   - Example: `video.mp4.0.part0`, `video.mp4.0.part1`, ...
//!   - Supports 10,000+ fine-grained chunks
//!   - Unique download ID prevents collisions
//!
//! - **Old-style** (Phase 2): `{filename}.part{i}`
//!   - Example: `video.mp4.part0`, `video.mp4.part1`, ...
//!   - Maximum 10 coarse-grained chunks
//!   - Fully supported for backward compatibility
//!
//! ### Parallel Download Flow
//!
//! 1. **Detection**: Check file size and Range request support
//! 2. **Chunking**: Calculate optimal power-of-two chunk size
//! 3. **Download**: Process chunks in batches using `buffer_unordered`
//! 4. **Progress**: Update atomic counter from all chunks in real-time
//! 5. **Merge**: Combine chunks sequentially into final file
//! 6. **Cleanup**: Remove all temporary chunk files
//!
//! ## Configuration
//!
//! Key configuration options (via [`rdlp_types::Config`]):
//!
//! - `concurrent_fragments`: Number of parallel connections (default: 4)
//! - `buffer_size`: I/O buffer size in bytes (default: 2 MB)
//! - `socket_timeout`: Connection timeout in seconds (default: 30)
//! - `user_agent`: Custom User-Agent header
//! - `proxy`: HTTP/HTTPS proxy URL

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::indexing_slicing)]
// ── Crate-wide lint allowances ────────────────────────────────────────────────
//
// `clippy::cast_*`: download arithmetic (byte counts, offsets, progress ratios)
//   requires mixed u64/usize/f64 types matching tokio/reqwest API surfaces.
//   All casts are audited for valid-range invariants.
//
// `clippy::significant_drop_tightening`: adaptive controller locks are held
//   across branches intentionally to keep controller state consistent.
//
// `clippy::redundant_pub_crate`: `pub(crate)` in private modules is kept for
//   documentation of intended visibility.
//
// `clippy::indexing_slicing`: `CHUNK_LEVELS[level]` and similar accesses are
//   guarded by prior clamping to valid index ranges.
//
// `clippy::items_after_statements`: test helper closures follow setup statements.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::significant_drop_tightening,
    clippy::redundant_pub_crate,
    clippy::indexing_slicing,
    clippy::items_after_statements
)]

/// Adaptive chunk sizing and connection tuning (AIMD controller)
pub(crate) mod adaptive;
/// Intelligent chunk size calculation for optimal download performance
pub mod chunking;
/// DASH (Dynamic Adaptive Streaming over HTTP) downloader for static VoD MPDs
pub mod dash;
/// Shared fragment-list downloader for pre-resolved segment URLs (DASH + HLS)
pub mod fragments;
/// HLS (HTTP Live Streaming) downloader with parallel segment downloads
pub mod hls;
/// HTTP/HTTPS downloader with parallel chunk support
pub mod http;
/// Shared progress reporting infrastructure
pub mod progress;

pub use chunking::{ChunkSizeStrategy, calculate_chunks, chunk_size_for_file};
pub use dash::DashDownloader;
pub use hls::HlsDownloader;
pub use http::HttpDownloader;
pub use progress::{
    ProgressGuard, ProgressMetrics, ProgressReporterConfig, spawn_progress_reporter,
};

use rdlp_core::Downloader;
use rdlp_http::HttpClientFactory;
use rdlp_ratelimit::RateLimiter;
use rdlp_types::Config;
use std::sync::Arc;

/// Trait for downloader registries to enable mocking in tests
pub trait DownloaderRegistryTrait: Send + Sync {
    /// Find a suitable downloader for the given URL
    fn find_downloader(&self, url: &str) -> Option<Arc<dyn Downloader>>;

    /// Find a downloader and apply extra HTTP headers (e.g. Referer for CDN auth).
    /// Default falls back to `find_downloader` (ignoring headers).
    fn find_downloader_with_headers(
        &self,
        url: &str,
        _headers: Option<&std::collections::HashMap<String, String>>,
    ) -> Option<Arc<dyn Downloader>> {
        self.find_downloader(url)
    }

    /// Get all registered downloader protocol names
    fn list_downloaders(&self) -> Vec<&str>;
}

/// Registry for managing downloaders
pub struct DownloaderRegistry {
    downloaders: Vec<Arc<dyn Downloader>>,
    /// Stored concrete HTTP downloader for creating copies with headers
    http_base: HttpDownloader,
    /// Stored concrete HLS downloader for creating copies with headers
    hls_base: HlsDownloader,
}

impl DownloaderRegistry {
    /// Create a new registry with default downloaders
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(&Config::default())
    }

    /// Create a new registry with custom configuration
    #[must_use]
    pub fn with_config(config: &Config) -> Self {
        let client = HttpClientFactory::from_rdlp_config(config).build();
        Self::build_registry(config, client)
    }

    /// Create a new registry with custom configuration and shared cookie jar
    ///
    /// The cookie jar is shared with the extraction HTTP client so that
    /// cookies obtained during extraction (including Cloudflare clearance
    /// and session cookies) are automatically sent during downloads.
    #[must_use]
    pub fn with_config_and_cookies(config: &Config, cookie_jar: Arc<wreq::cookie::Jar>) -> Self {
        let client = HttpClientFactory::from_rdlp_config(config).build_with_cookies(cookie_jar);
        Self::build_registry(config, client)
    }

    /// Internal helper to build registry from a pre-configured HTTP client
    fn build_registry(config: &Config, client: wreq::Client) -> Self {
        // Create rate limiter if configured
        let rate_limiter = config.rate_limit.map(|bps| Arc::new(RateLimiter::new(bps)));

        // Create HTTP downloader with optimized settings
        let http_downloader = HttpDownloader::with_client(client)
            .with_buffer_size(config.buffer_size)
            .with_concurrent_fragments(config.concurrent_fragments)
            .with_rate_limiter(rate_limiter)
            .with_adaptive(config.adaptive_downloads);

        // Create HLS downloader. concurrent_segments/buffer_size were no-ops on the
        // legacy parallel path (deleted in #270); the pre-resolved fragments path
        // doesn't use them. See issue #271.
        let hls_downloader = HlsDownloader::new().with_http_downloader(http_downloader.clone());

        // Create DASH downloader
        let dash_downloader = DashDownloader::new()
            .with_http_downloader(http_downloader.clone())
            .with_concurrent_segments(config.concurrent_fragments)
            .with_buffer_size(config.buffer_size);

        let mut registry = Self {
            downloaders: Vec::new(),
            http_base: http_downloader.clone(),
            hls_base: hls_downloader.clone(),
        };

        // Register HLS downloader FIRST (specific matcher for .m3u8 URLs)
        registry.register(Arc::new(hls_downloader));

        // Register DASH downloader SECOND (specific matcher for .mpd URLs;
        // MUST come before HTTP because HTTP's supports() matches every
        // https URL and would eat .mpd otherwise)
        registry.register(Arc::new(dash_downloader));

        // Register HTTP downloader LAST (fallback for generic HTTP/HTTPS URLs)
        registry.register(Arc::new(http_downloader));

        registry
    }

    /// Register a new downloader
    ///
    /// # Arguments
    /// * `downloader` - Arc-wrapped downloader implementing Downloader trait
    pub fn register(&mut self, downloader: Arc<dyn Downloader>) {
        self.downloaders.push(downloader);
    }

    /// Find a suitable downloader for the given URL
    ///
    /// Returns the first downloader that supports the given URL's protocol.
    /// Returns `None` if no downloader supports the URL.
    ///
    /// # Arguments
    /// * `url` - The URL to find a downloader for
    ///
    /// # Returns
    /// An `Arc<dyn Downloader>` if a suitable downloader is found, `None` otherwise
    ///
    /// # Examples
    /// ```no_run
    /// use rdlp_downloader::DownloaderRegistry;
    ///
    /// let registry = DownloaderRegistry::new();
    /// let downloader = registry.find_downloader("https://example.com/video.mp4");
    /// assert!(downloader.is_some());
    /// ```
    #[must_use]
    pub fn find_downloader(&self, url: &str) -> Option<Arc<dyn Downloader>> {
        self.downloaders.iter().find(|d| d.supports(url)).cloned()
    }

    /// Get all registered downloader protocol names
    ///
    /// # Returns
    /// A vector of protocol names (e.g., \["http", "hls", "dash"\])
    #[must_use]
    pub fn list_downloaders(&self) -> Vec<&str> {
        self.downloaders.iter().map(|d| d.protocol()).collect()
    }
}

impl Default for DownloaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DownloaderRegistryTrait for DownloaderRegistry {
    fn find_downloader(&self, url: &str) -> Option<Arc<dyn Downloader>> {
        self.find_downloader(url)
    }

    fn find_downloader_with_headers(
        &self,
        url: &str,
        headers: Option<&std::collections::HashMap<String, String>>,
    ) -> Option<Arc<dyn Downloader>> {
        // If no headers, use shared downloader
        if headers.is_none_or(std::collections::HashMap::is_empty) {
            return self.find_downloader(url);
        }

        // Create a fresh downloader clone with the extra headers applied
        let base = self.downloaders.iter().find(|d| d.supports(url))?;

        match base.protocol() {
            "hls" => {
                let new_http = self.http_base.clone().with_extra_headers(headers);
                let new_hls = self.hls_base.clone().with_http_downloader(new_http);
                Some(Arc::new(new_hls))
            }
            "http" => Some(Arc::new(self.http_base.clone().with_extra_headers(headers))),
            _ => self.find_downloader(url),
        }
    }

    fn list_downloaders(&self) -> Vec<&str> {
        self.list_downloaders()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = DownloaderRegistry::new();
        let downloaders = registry.list_downloaders();
        assert!(downloaders.contains(&"http"));
    }

    #[test]
    fn test_registry_with_custom_config() {
        let config = Config {
            buffer_size: 4 * 1024 * 1024, // 4 MB
            ..Default::default()
        };

        let registry = DownloaderRegistry::with_config(&config);
        let downloaders = registry.list_downloaders();
        assert!(downloaders.contains(&"http"));

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

        let hls_downloader = registry.find_downloader("https://example.com/playlist.m3u8");
        assert!(hls_downloader.is_some());
        assert_eq!(hls_downloader.unwrap().protocol(), "hls");

        let unknown = registry.find_downloader("rtmp://example.com/stream");
        assert!(unknown.is_none());
    }

    #[test]
    fn test_registry_lists_all_downloaders() {
        let registry = DownloaderRegistry::new();
        let downloaders = registry.list_downloaders();

        assert!(downloaders.contains(&"http"));
        assert!(downloaders.contains(&"hls"));
        assert!(downloaders.contains(&"dash"));
        assert_eq!(downloaders.len(), 3);
    }
}
