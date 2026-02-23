//! HLS (HTTP Live Streaming) size detection module
//!
//! This module provides functionality to detect the total file size of HLS streams
//! by parsing m3u8 playlists and aggregating segment sizes using parallel HTTP requests.
//!
//! # Architecture
//!
//! The size detection process follows these steps:
//! 1. Fetch the m3u8 playlist file
//! 2. Parse the playlist using m3u8-rs to extract segment URLs
//! 3. Make parallel HEAD requests to fetch each segment's Content-Length
//! 4. Sum all segment sizes to get the total file size
//!
//! # Performance
//!
//! - Concurrency: 8 parallel HTTP requests (configurable)
//! - Typical detection time: 2-5 seconds for 100-500 segment playlists
//! - Memory usage: O(n) where n = segment count (~20-200 KB)
//!
//! # Example
//!
//! ```no_run
//! use rdlp_extractor::hls::HlsSizeDetector;
//! use std::sync::Arc;
//!
//! # async fn example() -> rdlp_core::Result<()> {
//! let http_client = Arc::new(reqwest::Client::new());
//! let detector = HlsSizeDetector::new(http_client, false);
//!
//! let m3u8_url = "https://example.com/playlist.m3u8";
//! if let Some(size) = detector.detect_size(m3u8_url).await? {
//!     println!("Total size: {} MB", size / 1_000_000);
//! }
//! # Ok(())
//! # }
//! ```

mod detector;
mod format_detection;
mod segments;
mod types;
mod variants;

// Re-export public API unchanged
pub use detector::HlsSizeDetector;
pub use format_detection::detect_format_sizes;
pub use types::{HlsInfo, HlsStreamFlags, HlsVariantInfo};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_detector_creation() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client.clone(), false);

        assert_eq!(detector.concurrent_requests, 8);
        assert!(!detector.verbose);
    }

    #[test]
    fn test_detector_with_concurrency() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client, false).with_concurrency(16);

        assert_eq!(detector.concurrent_requests, 16);
    }

    #[test]
    fn test_detector_concurrency_minimum() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client, false).with_concurrency(0);

        // Should be clamped to minimum of 1
        assert_eq!(detector.concurrent_requests, 1);
    }

    // Note: Integration tests with real URLs are in the redtube extractor tests
    // because they require network access and are marked with #[ignore]
}
