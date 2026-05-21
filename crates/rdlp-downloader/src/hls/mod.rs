//! HLS (HTTP Live Streaming) downloader module.
//!
//! Downloads HLS streams by fetching pre-resolved fragments produced by
//! `expand_hls_in_place` in the extractor layer. Every `Format` with
//! `protocol: M3u8 | M3u8Native` that reaches this downloader MUST have
//! `Format.fragments` populated; absent fragments indicate a programmer error
//! (the extractor did not call the expander) and are surfaced as a typed
//! `RdlpError::Download`.

// `Duration::from_mins` / `from_hours` (lint's suggested replacements) need Rust 1.95;
// workspace MSRV is 1.85.
#![allow(clippy::duration_suboptimal_units)]

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use rdlp_core::{DownloadStats, Downloader, ProgressCallback, RdlpError, Result};

use crate::http::HttpDownloader;

/// HLS (HTTP Live Streaming) downloader
///
/// Downloads HLS streams from pre-resolved fragment lists produced by
/// `expand_hls_in_place`. Direct playlist parsing (the legacy path) has
/// been removed; all HLS formats MUST carry `Format.fragments` before
/// reaching this downloader.
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_downloader::HlsDownloader;
/// use rdlp_core::Downloader;
/// use std::path::Path;
///
/// # async fn example() -> rdlp_core::Result<()> {
/// let downloader = HlsDownloader::new();
/// downloader.download_to_file(
///     "https://example.com/playlist.m3u8",
///     Path::new("video.mp4"),
///     None
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct HlsDownloader {
    http_downloader: HttpDownloader,
    /// Kept for builder API compatibility; unused after legacy removal.
    concurrent_segments: usize,
    /// Kept for builder API compatibility; unused after legacy removal.
    buffer_size: usize,
    /// Total download timeout (entire operation must complete within this)
    download_timeout: Duration,
    /// Merge operation timeout; kept for API compatibility.
    merge_timeout: Duration,
    /// Kept for builder API compatibility; unused after legacy removal.
    max_segment_failures: usize,
}

impl HlsDownloader {
    /// Create a new HLS downloader with default settings
    #[must_use]
    pub fn new() -> Self {
        Self {
            http_downloader: HttpDownloader::new(),
            concurrent_segments: 8,
            buffer_size: 2 * 1024 * 1024,
            download_timeout: Duration::from_secs(3600),
            merge_timeout: Duration::from_secs(1800),
            max_segment_failures: 3,
        }
    }

    /// Set the HTTP downloader to use
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_http_downloader(mut self, http: HttpDownloader) -> Self {
        self.http_downloader = http;
        self
    }

    /// Set number of concurrent segment downloads
    #[must_use = "builder methods consume self and return a new instance"]
    #[deprecated(
        note = "no-op since #270; legacy parallel path removed; pre-resolved fragments path doesn't use this knob"
    )]
    pub fn with_concurrent_segments(mut self, count: usize) -> Self {
        self.concurrent_segments = count.max(1);
        self
    }

    /// Set buffer size for segment merging
    #[must_use = "builder methods consume self and return a new instance"]
    #[deprecated(
        note = "no-op since #270; legacy parallel path removed; pre-resolved fragments path doesn't use this knob"
    )]
    pub const fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Set total download timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub const fn with_download_timeout(mut self, timeout: Duration) -> Self {
        self.download_timeout = timeout;
        self
    }

    /// Set merge operation timeout
    #[must_use = "builder methods consume self and return a new instance"]
    #[deprecated(
        note = "no-op since #270; legacy parallel path removed; pre-resolved fragments path doesn't use this knob"
    )]
    pub const fn with_merge_timeout(mut self, timeout: Duration) -> Self {
        self.merge_timeout = timeout;
        self
    }

    /// Set maximum number of segment failures before aborting
    #[must_use = "builder methods consume self and return a new instance"]
    #[deprecated(
        note = "no-op since #270; legacy parallel path removed; pre-resolved fragments path doesn't use this knob"
    )]
    pub const fn with_max_segment_failures(mut self, max: usize) -> Self {
        self.max_segment_failures = max;
        self
    }

    /// Set extra HTTP headers sent with every request (delegates to inner `HttpDownloader`)
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_extra_headers(
        mut self,
        headers: Option<&std::collections::HashMap<String, String>>,
    ) -> Self {
        self.http_downloader = self.http_downloader.with_extra_headers(headers);
        self
    }
}

impl Default for HlsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for HlsDownloader {
    fn protocol(&self) -> &'static str {
        "hls"
    }

    fn supports(&self, url: &str) -> bool {
        std::path::Path::new(url)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("m3u8"))
            || url.contains("/playlist.m3u8")
            || url.contains(".m3u8?")
    }

    async fn download_format(
        &self,
        format: &rdlp_types::Format,
        output: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<DownloadStats> {
        // After #267, every M3u8 / M3u8Native row reaching the downloader has
        // Format.fragments populated by expand_hls_in_place. A row without
        // fragments indicates an extractor that did NOT call the expander —
        // programmer error, not a runtime case to handle gracefully.
        let Some(fragments) = format.fragments.as_deref() else {
            return Err(RdlpError::Download {
                message: format!(
                    "internal error: HLS Format reached HlsDownloader without \
                     pre-resolved fragments — extractor must call \
                     expand_hls_in_place. Format: {}",
                    format.format_id
                ),
                url: Some(format.url.clone()),
            });
        };

        crate::fragments::download_pre_resolved_fragments(
            &self.http_downloader,
            fragments,
            format.fragment_base_url.as_deref(),
            format.filesize,
            progress.as_deref(),
            output,
            cancel,
        )
        .await
    }

    async fn download_to_file(
        &self,
        url: &str,
        _path: &Path,
        _progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // The legacy playlist-parsing path has been removed in #267. All HLS
        // downloads must go through download_format with pre-resolved fragments.
        // This entry point is retained only to satisfy the Downloader trait; it
        // should never be reached in production.
        Err(RdlpError::Download {
            message: "internal error: HlsDownloader::download_to_file called directly — \
                      use download_format with pre-resolved fragments (expand_hls_in_place)"
                .to_string(),
            url: Some(url.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hls_downloader_creation() {
        let downloader = HlsDownloader::new();
        assert_eq!(downloader.protocol(), "hls");
        assert_eq!(downloader.concurrent_segments, 8);
    }

    #[test]
    #[allow(deprecated)] // exercising deprecated builder methods to verify they still compile
    fn test_hls_downloader_builder() {
        let downloader = HlsDownloader::new()
            .with_concurrent_segments(16)
            .with_buffer_size(4 * 1024 * 1024);

        assert_eq!(downloader.concurrent_segments, 16);
        assert_eq!(downloader.buffer_size, 4 * 1024 * 1024);
    }

    #[test]
    fn test_supports_m3u8_urls() {
        let downloader = HlsDownloader::new();

        assert!(downloader.supports("https://example.com/video.m3u8"));
        assert!(downloader.supports("https://example.com/playlist.m3u8"));
        assert!(downloader.supports("https://example.com/index.m3u8?token=abc"));
        assert!(!downloader.supports("https://example.com/video.mp4"));
    }

    #[test]
    #[allow(deprecated)] // exercising deprecated builder method to verify clamp behavior
    fn test_concurrent_segments_minimum() {
        let downloader = HlsDownloader::new().with_concurrent_segments(0);
        // Should be clamped to minimum of 1
        assert_eq!(downloader.concurrent_segments, 1);
    }
}
