//! HTTP downloader configuration
//!
//! Provides configuration for HTTP downloads including buffer sizes,
//! retry settings, and concurrent download settings.

// `Duration::from_mins` / `from_hours` (lint's suggested replacements) need Rust 1.95;
// workspace MSRV is 1.85.
#![allow(clippy::duration_suboptimal_units)]

use crate::chunking::ChunkSizeStrategy;
use rdlp_core::RetryConfig;
use std::time::Duration;

// =============================================================================
// Constants
// =============================================================================

/// Default minimum file size to enable parallel downloads (10 MiB).
/// Used when `Config::parallel_threshold` is `None` and as the
/// `DownloaderConfig::default()` value.
pub(super) const DEFAULT_PARALLEL_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;

/// Size of the F3 initial range probe used to detect `Content-Length` and range support.
/// The body is discarded; only headers are consulted.
///
/// Rationale (per docs/superpowers/specs/2026-05-21-f3-f6-download-optimization-design.md):
/// - Matches `MIN_CHUNK_LEVEL=2` (256 KB) in `adaptive.rs` so the probe looks like a real
///   chunk to bot-detecting origins.
/// - Fits TCP slow-start delivery within ~3-4 RTTs from cwnd=10 (RFC 6928).
/// - 4× the H2 default connection window (RFC 7540 §6.9, 64 KiB) — requires a few
///   `WINDOW_UPDATE` frames but does not catastrophically starve concurrent streams.
///   1 MiB (16× the window) would actively block parallel chunks.
pub(crate) const PROBE_WINDOW_BYTES: u64 = 256 * 1024;

/// Progress callback update interval
pub(super) const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(100);

/// Default buffer size for I/O operations (2 MB)
const DEFAULT_BUFFER_SIZE: usize = 2 * 1024 * 1024;

/// Maximum concurrent connections cap
const MAX_CONCURRENT_CONNECTIONS: usize = 8;

/// Default per-read idle timeout (60 seconds)
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Default total download timeout (1 hour)
const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(3600);

/// Default merge operation timeout (30 minutes)
const DEFAULT_MERGE_TIMEOUT: Duration = Duration::from_secs(1800);

/// Downloader configuration (shared across clones via Arc)
///
/// This struct consolidates all config fields into a single Arc,
/// making `HttpDownloader` clones truly zero-cost (~5ns Arc clone vs ~24 bytes field copies).
///
/// **Memory optimization:**
/// - Before: 591 clones × 24 bytes = ~14 KB copied
/// - After: 591 Arc clones × 8 bytes = ~5 KB pointers
/// - **Savings: ~9 KB per download**
#[derive(Clone)]
pub struct DownloaderConfig {
    pub buffer_size: usize,
    pub retry_config: RetryConfig,
    pub concurrent_fragments: usize,
    pub chunk_strategy: ChunkSizeStrategy,
    /// Minimum file size at which `download_to_file` switches to parallel chunked
    /// download. Defaults to `DEFAULT_PARALLEL_THRESHOLD_BYTES` (10 MiB).
    pub parallel_threshold: u64,
    /// Per-read idle timeout (no data received for this long = abort)
    pub read_timeout: Duration,
    /// Total download timeout (entire operation must complete within this)
    pub download_timeout: Duration,
    /// Merge operation timeout (chunk/segment merge must complete within this)
    pub merge_timeout: Duration,
    /// Enable adaptive chunk sizing and connection tuning via AIMD controller.
    /// Forced to `false` when `chunk_strategy` is `Fixed`.
    pub adaptive: bool,
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        // Calculate optimal concurrent connections based on CPU threads
        // For I/O-bound workloads like HTTP downloads:
        // - Tokio can handle many more tasks than CPU cores
        // - Research shows: aria2 uses 4-16 connections, yt-dlp defaults to 1
        // - Formula: min(available_parallelism, MAX_CONCURRENT_CONNECTIONS)
        //   * Too few: underutilizes bandwidth
        //   * Too many: connection overhead, server rate limiting
        let concurrent_fragments = std::thread::available_parallelism()
            .map_or(4, |n| n.get().min(MAX_CONCURRENT_CONNECTIONS));

        Self {
            buffer_size: DEFAULT_BUFFER_SIZE,
            retry_config: RetryConfig::default_config(),
            concurrent_fragments,
            chunk_strategy: ChunkSizeStrategy::Auto,
            parallel_threshold: DEFAULT_PARALLEL_THRESHOLD_BYTES,
            read_timeout: DEFAULT_READ_TIMEOUT,
            download_timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            merge_timeout: DEFAULT_MERGE_TIMEOUT,
            adaptive: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_window_is_256_kib() {
        assert_eq!(PROBE_WINDOW_BYTES, 256 * 1024);
    }

    #[test]
    fn probe_window_under_one_mib() {
        // 1 MiB would be 16x the RFC 7540 default H2 connection window (64 KiB)
        // and would starve parallel streams.
        const { assert!(PROBE_WINDOW_BYTES < 1024 * 1024) };
    }
}
