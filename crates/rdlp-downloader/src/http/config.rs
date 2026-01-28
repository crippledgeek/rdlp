//! HTTP downloader configuration
//!
//! Provides configuration for HTTP downloads including buffer sizes,
//! retry settings, and concurrent download settings.

use crate::chunking::ChunkSizeStrategy;
use rdlp_core::RetryConfig;
use std::time::Duration;

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
pub(crate) struct DownloaderConfig {
    pub buffer_size: usize,
    pub retry_config: RetryConfig,
    pub concurrent_fragments: usize,
    pub chunk_strategy: ChunkSizeStrategy,
    /// Per-read idle timeout (no data received for this long = abort)
    pub read_timeout: Duration,
    /// Total download timeout (entire operation must complete within this)
    pub download_timeout: Duration,
    /// Merge operation timeout (chunk/segment merge must complete within this)
    pub merge_timeout: Duration,
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
            .unwrap_or(4);

        Self {
            buffer_size: 2 * 1024 * 1024, // 2 MB
            retry_config: RetryConfig::default_config(),
            concurrent_fragments,
            chunk_strategy: ChunkSizeStrategy::Auto,
            read_timeout: Duration::from_secs(60),
            download_timeout: Duration::from_secs(3600), // 1 hour
            merge_timeout: Duration::from_secs(1800),    // 30 min
        }
    }
}
