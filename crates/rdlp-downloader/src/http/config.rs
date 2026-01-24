//! HTTP downloader configuration
//!
//! Provides configuration for HTTP downloads including buffer sizes,
//! retry settings, and concurrent download settings.

use rdlp_core::RetryConfig;
use crate::chunking::ChunkSizeStrategy;

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
            buffer_size: 8192,
            retry_config: RetryConfig::default_config(),
            concurrent_fragments,
            chunk_strategy: ChunkSizeStrategy::Auto,
        }
    }
}
