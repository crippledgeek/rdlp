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

use crate::base::common::BaseExtractor;
use futures::stream::{self, StreamExt};
use rdlp_core::{RdlpError, Result};
use std::sync::Arc;

/// Maximum number of segments to process (security limit)
const MAX_SEGMENTS: usize = 10_000;

/// Default number of concurrent HTTP requests
const DEFAULT_CONCURRENCY: usize = 8;

/// Information about an HLS stream
#[derive(Debug, Clone)]
pub struct HlsInfo {
    /// Total size in bytes (sum of all segment sizes) - None if not detected
    pub total_size: Option<u64>,
    /// Number of segments in the playlist
    pub segment_count: usize,
}

/// HLS playlist size detector
#[derive(Clone)]
pub struct HlsSizeDetector {
    http_client: Arc<reqwest::Client>,
    concurrent_requests: usize,
    verbose: bool,
}

impl HlsSizeDetector {
    /// Create a new HLS size detector
    ///
    /// # Arguments
    /// * `http_client` - Shared HTTP client for making requests
    /// * `verbose` - Enable detailed logging
    pub fn new(http_client: Arc<reqwest::Client>, verbose: bool) -> Self {
        Self {
            http_client,
            concurrent_requests: DEFAULT_CONCURRENCY,
            verbose,
        }
    }

    /// Set the number of concurrent HTTP requests
    ///
    /// # Arguments
    /// * `count` - Number of concurrent requests (min: 1, recommended: 8)
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_concurrency(mut self, count: usize) -> Self {
        self.concurrent_requests = count.max(1);
        self
    }

    /// Detect the total size of an HLS stream
    ///
    /// This is the main entry point for HLS size detection. It fetches the playlist,
    /// parses it, and calculates the total size by summing all segment sizes.
    ///
    /// # Arguments
    /// * `m3u8_url` - URL of the HLS m3u8 playlist
    ///
    /// # Returns
    /// * `Ok(Some(size))` - Total size in bytes
    /// * `Ok(None)` - Size could not be determined (non-fatal)
    /// * `Err(_)` - Fatal error (network failure, invalid playlist, etc.)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn example() -> rdlp_core::Result<()> {
    /// # let detector = rdlp_extractor::hls::HlsSizeDetector::new(
    /// #     std::sync::Arc::new(reqwest::Client::new()),
    /// #     false
    /// # );
    /// match detector.detect_size("https://example.com/playlist.m3u8").await {
    ///     Ok(Some(size)) => println!("Size: {} MB", size / 1_000_000),
    ///     Ok(None) => println!("Size could not be determined"),
    ///     Err(e) => eprintln!("Error: {e}"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn detect_size(&self, m3u8_url: &str) -> Result<Option<u64>> {
        // Use detect_info and extract just the size
        Ok(self.detect_info(m3u8_url).await?.and_then(|info| info.total_size))
    }

    /// Fast segment count detection (no size fetching)
    ///
    /// Only parses the m3u8 playlist to count segments. This is much faster
    /// than `detect_info` because it doesn't make HEAD requests to each segment.
    ///
    /// # Performance
    /// - 1-2 HTTP requests (master + media playlist)
    /// - Typical time: 100-500ms
    pub async fn count_segments(&self, m3u8_url: &str) -> Result<Option<usize>> {
        // Validate the input URL for security (SSRF protection)
        BaseExtractor::validate_url_security(m3u8_url)?;

        if self.verbose {
            eprintln!("[HLS] Counting segments for: {m3u8_url}");
        }

        // Parse playlist to extract segment URLs
        match self.parse_playlist(m3u8_url).await {
            Ok(urls) => {
                let count = urls.len();
                if self.verbose {
                    eprintln!("[HLS] Found {count} segments");
                }
                Ok(Some(count))
            }
            Err(e) => {
                if self.verbose {
                    eprintln!("[HLS] Failed to parse playlist: {e}");
                }
                Ok(None)
            }
        }
    }

    /// Detect the total size and segment count of an HLS stream
    ///
    /// This method fetches the playlist, parses it, and calculates both the total size
    /// and segment count. Use this when you need both pieces of information.
    ///
    /// # Arguments
    /// * `m3u8_url` - URL of the HLS m3u8 playlist
    ///
    /// # Returns
    /// * `Ok(Some(HlsInfo))` - Total size and segment count
    /// * `Ok(None)` - Info could not be determined (non-fatal)
    /// * `Err(_)` - Fatal error (network failure, invalid playlist, etc.)
    pub async fn detect_info(&self, m3u8_url: &str) -> Result<Option<HlsInfo>> {
        let start = std::time::Instant::now();

        // Validate the input URL for security (SSRF protection)
        BaseExtractor::validate_url_security(m3u8_url)?;

        if self.verbose {
            eprintln!("[HLS] Detecting size for: {m3u8_url}");
        }

        // Step 1: Parse playlist to extract segment URLs
        let segment_urls = match self.parse_playlist(m3u8_url).await {
            Ok(urls) => urls,
            Err(e) => {
                if self.verbose {
                    eprintln!("[HLS] Failed to parse playlist: {e}");
                }
                return Ok(None);
            }
        };

        if segment_urls.is_empty() {
            if self.verbose {
                eprintln!("[HLS] No segments found in playlist");
            }
            return Ok(None);
        }

        let segment_count = segment_urls.len();

        // Step 2: Calculate total size from all segments
        let total_size = match self.sum_segment_sizes(segment_urls).await {
            Ok(size) => size,
            Err(e) => {
                if self.verbose {
                    eprintln!("[HLS] Failed to calculate total size: {e}");
                }
                return Ok(None);
            }
        };

        if self.verbose {
            let duration = start.elapsed();
            eprintln!(
                "[HLS] Detection completed in {duration:?}: {} MB ({total_size} bytes), {segment_count} segments",
                total_size / 1_000_000
            );
        }

        Ok(Some(HlsInfo {
            total_size: Some(total_size),
            segment_count,
        }))
    }

    /// Parse m3u8 playlist and extract segment URLs
    ///
    /// This method fetches the playlist file, parses it using m3u8-rs, and extracts
    /// all segment URLs. It handles both absolute and relative URLs.
    ///
    /// # Arguments
    /// * `m3u8_url` - URL of the m3u8 playlist
    ///
    /// # Returns
    /// * `Ok(Vec<String>)` - List of segment URLs
    /// * `Err(_)` - Network error, parse error, or master playlist detected
    async fn parse_playlist(&self, m3u8_url: &str) -> Result<Vec<String>> {
        if self.verbose {
            eprintln!("[HLS] Fetching playlist from: {m3u8_url}");
        }

        // Fetch playlist text
        let response = self.http_client.get(m3u8_url).send().await.map_err(|e| {
            if e.is_timeout() {
                RdlpError::Network(format!("Timeout fetching playlist: {m3u8_url}"))
            } else if e.is_connect() {
                RdlpError::Network(format!("Connection failed for playlist: {m3u8_url}: {e}"))
            } else {
                RdlpError::Network(format!("Failed to fetch playlist: {e}"))
            }
        })?;

        if !response.status().is_success() {
            return Err(RdlpError::Network(format!(
                "HTTP {} for playlist: {m3u8_url}",
                response.status()
            )));
        }

        let playlist_text = response.text().await.map_err(|e| {
            RdlpError::Network(format!("Failed to read playlist response: {e}"))
        })?;

        if self.verbose {
            eprintln!(
                "[HLS] Playlist size: {} bytes",
                playlist_text.len()
            );
        }

        // Parse with m3u8-rs
        let playlist =
            m3u8_rs::parse_playlist_res(playlist_text.as_bytes()).map_err(|e| {
                RdlpError::Extraction(format!("M3U8 parse error: {e:?}"))
            })?;

        // Extract segment URLs
        match playlist {
            m3u8_rs::Playlist::MediaPlaylist(media) => {
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

                let segments: Vec<String> = media
                    .segments
                    .iter()
                    .map(|seg| {
                        // Join relative URLs with base URL
                        base_url
                            .join(&seg.uri)
                            .map(|u| u.to_string())
                            .unwrap_or_else(|_| seg.uri.clone())
                    })
                    .collect();

                // Security check: limit max segments
                if segments.len() > MAX_SEGMENTS {
                    return Err(RdlpError::Extraction(format!(
                        "Playlist has too many segments: {} (max: {MAX_SEGMENTS})",
                        segments.len()
                    )));
                }

                // Validate segment URLs using BaseExtractor SSRF protection
                // This catches: invalid URLs, bad schemes, private IPs, URL length limits
                for url in &segments {
                    BaseExtractor::validate_url_security(url)?;
                }

                if self.verbose {
                    eprintln!("[HLS] Found {} segments", segments.len());
                }

                Ok(segments)
            }
            m3u8_rs::Playlist::MasterPlaylist(master) => {
                // Master playlist contains variants - select the first one
                if master.variants.is_empty() {
                    return Err(RdlpError::Extraction(
                        "Master playlist has no variants".into(),
                    ));
                }

                // Select the first variant (usually the best quality)
                let variant = &master.variants[0];
                let media_playlist_uri = &variant.uri;

                if self.verbose {
                    eprintln!(
                        "[HLS] Master playlist detected with {} variants",
                        master.variants.len()
                    );
                    eprintln!(
                        "[HLS] Selecting first variant: {} (bandwidth: {} bps)",
                        media_playlist_uri,
                        variant.bandwidth
                    );
                }

                // Resolve relative URL for media playlist
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

                let media_playlist_url = base_url
                    .join(media_playlist_uri)
                    .map_err(|e| {
                        RdlpError::Extraction(format!("Failed to join media playlist URL: {e}"))
                    })?
                    .to_string();

                // Validate the resolved media playlist URL (SSRF protection)
                BaseExtractor::validate_url_security(&media_playlist_url)?;

                // Recursively parse the media playlist
                Box::pin(self.parse_playlist(&media_playlist_url)).await
            }
        }
    }

    /// Fetch a single segment's size using HEAD request with Range fallback
    ///
    /// This method tries HEAD request first (fast, no download), and falls back
    /// to a Range request if HEAD doesn't return Content-Length.
    ///
    /// # Arguments
    /// * `segment_url` - URL of the segment
    ///
    /// # Returns
    /// * `Ok(size)` - Segment size in bytes
    /// * `Err(_)` - Network error or size could not be determined
    async fn fetch_segment_size(&self, segment_url: &str) -> Result<u64> {
        // Try HEAD request first (fast, no download)
        match self.http_client.head(segment_url).send().await {
            Ok(response) if response.status().is_success() => {
                if let Some(size) = response.content_length() {
                    if size > 0 {
                        return Ok(size);
                    }
                }
            }
            Ok(response) => {
                if self.verbose {
                    eprintln!(
                        "[HLS] HEAD request returned HTTP {} for: {segment_url}",
                        response.status()
                    );
                }
            }
            Err(e) => {
                if self.verbose {
                    eprintln!("[HLS] HEAD request failed for {segment_url}: {e}");
                }
            }
        }

        // Fallback: Range request to parse Content-Range
        let range_response = self
            .http_client
            .get(segment_url)
            .header("Range", "bytes=0-0")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    RdlpError::Network(format!("Timeout for segment: {segment_url}"))
                } else if e.is_connect() {
                    RdlpError::Network(format!("Connection failed for segment: {segment_url}: {e}"))
                } else {
                    RdlpError::Network(format!("Failed to fetch segment: {e}"))
                }
            })?;

        if !range_response.status().is_success() {
            return Err(RdlpError::Network(format!(
                "HTTP {} for segment: {segment_url}",
                range_response.status()
            )));
        }

        // Try Content-Range header: "bytes 0-0/123456"
        if let Some(content_range) = range_response.headers().get("content-range") {
            if let Ok(range_str) = content_range.to_str() {
                if let Some(total) = range_str.split('/').nth(1) {
                    if let Ok(size) = total.parse::<u64>() {
                        return Ok(size);
                    }
                }
            }
        }

        // Last fallback: Content-Length from Range response
        if let Some(size) = range_response.content_length() {
            return Ok(size);
        }

        Err(RdlpError::Extraction(format!(
            "Could not detect segment size for: {segment_url}"
        )))
    }

    /// Calculate total size from all segments using parallel requests
    ///
    /// This method uses `buffer_unordered` to make parallel HEAD requests with
    /// bounded concurrency. It handles partial failures gracefully.
    ///
    /// # Arguments
    /// * `segment_urls` - List of segment URLs
    ///
    /// # Returns
    /// * `Ok(total_size)` - Sum of all segment sizes
    /// * `Err(_)` - If majority of segments fail (< 50% success rate)
    async fn sum_segment_sizes(&self, segment_urls: Vec<String>) -> Result<u64> {
        let total_segments = segment_urls.len();

        if self.verbose {
            eprintln!(
                "[HLS] Fetching sizes for {total_segments} segments ({} concurrent)...",
                self.concurrent_requests
            );
        }

        // Use buffer_unordered for parallel processing with bounded concurrency
        let results: Vec<Result<u64>> = stream::iter(segment_urls)
            .map(|url| {
                let detector = self.clone();
                async move { detector.fetch_segment_size(&url).await }
            })
            .buffer_unordered(self.concurrent_requests)
            .collect()
            .await;

        // Count successes and failures
        let mut total_size = 0u64;
        let mut successful = 0usize;
        let mut failed = 0usize;

        for result in results {
            match result {
                Ok(size) => {
                    total_size += size;
                    successful += 1;
                }
                Err(e) => {
                    if self.verbose {
                        eprintln!("[HLS] Segment size detection failed: {e}");
                    }
                    failed += 1;
                }
            }
        }

        let success_rate = successful as f64 / total_segments as f64;

        if self.verbose {
            eprintln!(
                "[HLS] Results: {successful}/{total_segments} successful ({:.1}% success rate)",
                success_rate * 100.0
            );
        }

        // Require at least 50% success rate
        if success_rate < 0.5 {
            return Err(RdlpError::Extraction(format!(
                "Too many segment failures: {failed}/{total_segments} failed (< 50% success rate)"
            )));
        }

        // Warn if success rate is below 90%
        if success_rate < 0.9 && self.verbose {
            eprintln!(
                "[HLS] Warning: Only {successful}/{total_segments} segments succeeded, size may be inaccurate"
            );
        }

        Ok(total_size)
    }
}

/// Detect file sizes and segment counts for all formats in parallel
///
/// This is a shared utility function used by multiple extractors to avoid code duplication.
/// HLS formats get fast segment counting (no size fetching), while other formats get
/// file size detection via HEAD requests.
///
/// # Arguments
/// * `formats` - Vector of formats to detect sizes for
/// * `ctx` - Extraction context with HTTP client and config
/// * `extractor_name` - Name of the extractor for logging (e.g., "PornHub", "RedTube")
///
/// # Returns
/// Vector of formats with sizes/segment counts populated
pub async fn detect_format_sizes(
    formats: Vec<rdlp_core::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
) -> Vec<rdlp_core::Format> {
    use futures::future::join_all;
    use rdlp_core::Fragment;
    use std::time::Duration;
    use tokio::time::timeout;

    let verbose = ctx.config.verbose;
    let hls_detector = HlsSizeDetector::new(ctx.http_client.clone(), verbose);
    let http_client = ctx.http_client.clone();
    let extractor_name = extractor_name.to_string();

    let detection_futures: Vec<_> = formats
        .into_iter()
        .map(|format| {
            let hls_detector = hls_detector.clone();
            let http_client = http_client.clone();
            let extractor_name = extractor_name.clone();

            async move {
                let mut format = format;
                let url = format.url.clone();
                let is_hls = format.ext == "hls" || url.contains(".m3u8") || url.contains("/hls/");

                if is_hls {
                    // Fast segment count only (parses m3u8, no size fetching)
                    let result = timeout(
                        Duration::from_secs(5),
                        hls_detector.count_segments(&url),
                    )
                    .await;

                    match result {
                        Ok(Ok(Some(segment_count))) => {
                            format.fragments = Some(
                                (0..segment_count)
                                    .map(|_| Fragment {
                                        url: String::new(),
                                        duration: None,
                                        filesize: None,
                                    })
                                    .collect(),
                            );
                            if verbose {
                                eprintln!(
                                    "[{extractor_name}] HLS {}: {segment_count} segments",
                                    format.format_note.as_deref().unwrap_or(&format.format_id),
                                );
                            }
                        }
                        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                            if verbose {
                                eprintln!("[{extractor_name}] Could not count segments for: {url}");
                            }
                        }
                    }
                } else {
                    // Non-HLS: HEAD request for file size
                    let result = timeout(
                        Duration::from_secs(5),
                        BaseExtractor::detect_file_size_with_client(&url, &http_client),
                    )
                    .await;

                    if let Ok(Some(size)) = result {
                        format.filesize = Some(size);
                    }
                }

                format
            }
        })
        .collect();

    join_all(detection_futures).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detector_creation() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client.clone(), false);

        assert_eq!(detector.concurrent_requests, DEFAULT_CONCURRENCY);
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
