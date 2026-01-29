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
use log::debug;
use rdlp_core::{RdlpError, Result};
use std::sync::Arc;

/// Maximum number of segments to process (security limit)
const MAX_SEGMENTS: usize = 10_000;

/// Default number of concurrent HTTP requests
const DEFAULT_CONCURRENCY: usize = 8;

/// Stream-level flags aggregated from HLS format detection
///
/// These flags represent properties of the entire stream, not individual formats.
/// They are aggregated during `detect_format_sizes()` and can be used to set
/// `InfoDict.is_live` or warn users about encrypted content.
#[derive(Debug, Clone, Default)]
pub struct HlsStreamFlags {
    /// True if any HLS format is a live stream (no EXT-X-ENDLIST tag)
    pub is_live: bool,
    /// True if any HLS format uses encryption (EXT-X-KEY)
    pub has_any_drm: bool,
}

/// Information about an HLS stream
#[derive(Debug, Clone)]
pub struct HlsInfo {
    /// Total size in bytes (sum of all segment sizes) - None if not detected
    pub total_size: Option<u64>,
    /// Number of segments in the playlist
    pub segment_count: usize,
    /// Total duration in seconds (sum of segment durations)
    pub total_duration: Option<f64>,
    /// Video resolution (width, height) from master playlist variant
    pub resolution: Option<(u64, u64)>,
    /// Parsed video codec name (e.g., "h264", "hevc", "vp9")
    pub video_codec: Option<String>,
    /// Parsed audio codec name (e.g., "aac", "ac3", "opus")
    pub audio_codec: Option<String>,
    /// Frame rate from master playlist variant
    pub frame_rate: Option<f64>,
    /// Peak bandwidth in bits per second from variant
    pub bandwidth: Option<u64>,
    /// Average bandwidth in bits per second from variant
    pub average_bandwidth: Option<u64>,
    /// Whether the stream is live (no EXT-X-ENDLIST tag)
    pub is_live: bool,
    /// Whether any segment uses encryption (EXT-X-KEY)
    pub has_encryption: bool,
    /// Detected segment container format (e.g., "ts", "mp4", "m4s")
    pub segment_container: Option<String>,
}

/// Media playlist metadata extracted without additional HTTP requests
struct MediaPlaylistInfo {
    segment_count: usize,
    total_duration: f64,
    is_live: bool,
    has_encryption: bool,
    segment_container: Option<String>,
}

/// Detect container format from segment URL extension
fn detect_segment_container(segment_uri: &str) -> Option<String> {
    let path = segment_uri.split('?').next().unwrap_or(segment_uri);
    path.rfind('.')
        .map(|pos| path[pos + 1..].to_lowercase())
        .filter(|ext| !ext.is_empty())
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
    #[must_use]
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
        Ok(self
            .detect_info(m3u8_url)
            .await?
            .and_then(|info| info.total_size))
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
            debug!(url:? = m3u8_url; "HLS counting segments");
        }

        // Parse playlist to extract segment URLs
        match self.parse_playlist(m3u8_url).await {
            Ok(urls) => {
                let count = urls.len();
                if self.verbose {
                    debug!(count; "HLS found segments");
                }
                Ok(Some(count))
            }
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to parse playlist: {e}");
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
            debug!(url:? = m3u8_url; "HLS detecting size");
        }

        // Step 1: Parse playlist to extract segment URLs
        let segment_urls = match self.parse_playlist(m3u8_url).await {
            Ok(urls) => urls,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to parse playlist: {e}");
                }
                return Ok(None);
            }
        };

        if segment_urls.is_empty() {
            if self.verbose {
                debug!("HLS no segments found in playlist");
            }
            return Ok(None);
        }

        let segment_count = segment_urls.len();

        // Detect container from first segment URL
        let segment_container = segment_urls.first().and_then(|url| detect_segment_container(url));

        // Step 2: Calculate total size from all segments
        let total_size = match self.sum_segment_sizes(segment_urls).await {
            Ok(size) => size,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to calculate total size: {e}");
                }
                return Ok(None);
            }
        };

        if self.verbose {
            let duration = start.elapsed();
            debug!(
                duration:? = duration,
                mb = total_size / 1_000_000,
                bytes = total_size,
                segments = segment_count;
                "HLS detection completed"
            );
        }

        Ok(Some(HlsInfo {
            total_size: Some(total_size),
            segment_count,
            total_duration: None,
            resolution: None,
            video_codec: None,
            audio_codec: None,
            frame_rate: None,
            bandwidth: None,
            average_bandwidth: None,
            is_live: false,
            has_encryption: false,
            segment_container,
        }))
    }

    /// Detect comprehensive HLS metadata from an M3U8 playlist
    ///
    /// Fetches and parses the M3U8 playlist, extracting all available metadata:
    /// - From master playlists: resolution, codecs, frame rate, bandwidth
    /// - From media playlists: segment count, total duration, live/VOD, encryption
    ///
    /// This is more comprehensive than `count_segments()` but does not fetch segment
    /// sizes (no HEAD requests). Performance is similar: 1-2 HTTP requests.
    ///
    /// # Arguments
    /// * `m3u8_url` - URL of the HLS m3u8 playlist
    ///
    /// # Returns
    /// * `Ok(Some(HlsInfo))` - Metadata extracted from the playlist
    /// * `Ok(None)` - Metadata could not be determined (non-fatal)
    /// * `Err(_)` - Fatal error (network failure, invalid playlist, etc.)
    pub async fn detect_hls_metadata(&self, m3u8_url: &str) -> Result<Option<HlsInfo>> {
        BaseExtractor::validate_url_security(m3u8_url)?;

        if self.verbose {
            debug!(url:? = m3u8_url; "HLS detecting metadata");
        }

        let playlist_text = match self.fetch_playlist_text(m3u8_url).await {
            Ok(text) => text,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to fetch playlist: {e}");
                }
                return Ok(None);
            }
        };

        let playlist = match m3u8_rs::parse_playlist_res(playlist_text.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to parse playlist: {e:?}");
                }
                return Ok(None);
            }
        };

        match playlist {
            m3u8_rs::Playlist::MasterPlaylist(master) => {
                if master.variants.is_empty() {
                    return Ok(None);
                }

                let variant = &master.variants[0];

                // Extract variant metadata
                let resolution = variant.resolution.as_ref().map(|r| (r.width, r.height));
                let (video_codec, audio_codec) = variant
                    .codecs
                    .as_deref()
                    .map(rdlp_core::parse_hls_codecs)
                    .unwrap_or((None, None));
                let frame_rate = variant.frame_rate;
                let bandwidth = Some(variant.bandwidth);
                let average_bandwidth = variant.average_bandwidth;

                if self.verbose {
                    debug!(
                        variants = master.variants.len(),
                        bandwidth = variant.bandwidth,
                        resolution:? = resolution,
                        codecs:? = variant.codecs,
                        frame_rate:? = frame_rate;
                        "HLS master playlist metadata extracted"
                    );
                }

                // Resolve and fetch the media playlist for segment info
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;
                let media_url = base_url
                    .join(&variant.uri)
                    .map_err(|e| {
                        RdlpError::Extraction(format!("Failed to join media playlist URL: {e}"))
                    })?
                    .to_string();

                BaseExtractor::validate_url_security(&media_url)?;

                let media_info = match self.fetch_and_extract_media_info(&media_url).await {
                    Ok(Some(info)) => info,
                    Ok(None) | Err(_) => {
                        // Return what we have from the master playlist
                        return Ok(Some(HlsInfo {
                            total_size: None,
                            segment_count: 0,
                            total_duration: None,
                            resolution,
                            video_codec: video_codec.map(String::from),
                            audio_codec: audio_codec.map(String::from),
                            frame_rate,
                            bandwidth,
                            average_bandwidth,
                            is_live: false,
                            has_encryption: false,
                            segment_container: None,
                        }));
                    }
                };

                Ok(Some(HlsInfo {
                    total_size: None,
                    segment_count: media_info.segment_count,
                    total_duration: Some(media_info.total_duration),
                    resolution,
                    video_codec: video_codec.map(String::from),
                    audio_codec: audio_codec.map(String::from),
                    frame_rate,
                    bandwidth,
                    average_bandwidth,
                    is_live: media_info.is_live,
                    has_encryption: media_info.has_encryption,
                    segment_container: media_info.segment_container,
                }))
            }
            m3u8_rs::Playlist::MediaPlaylist(media) => {
                let info = Self::extract_media_playlist_info(&media);

                if self.verbose {
                    debug!(
                        segments = info.segment_count,
                        duration = info.total_duration,
                        is_live = info.is_live,
                        encrypted = info.has_encryption;
                        "HLS media playlist metadata extracted"
                    );
                }

                Ok(Some(HlsInfo {
                    total_size: None,
                    segment_count: info.segment_count,
                    total_duration: Some(info.total_duration),
                    resolution: None,
                    video_codec: None,
                    audio_codec: None,
                    frame_rate: None,
                    bandwidth: None,
                    average_bandwidth: None,
                    is_live: info.is_live,
                    has_encryption: info.has_encryption,
                    segment_container: info.segment_container,
                }))
            }
        }
    }

    /// Fetch M3U8 playlist text from a URL
    async fn fetch_playlist_text(&self, m3u8_url: &str) -> Result<String> {
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

        response
            .text()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to read playlist response: {e}")))
    }

    /// Fetch a media playlist and extract its metadata
    async fn fetch_and_extract_media_info(
        &self,
        media_url: &str,
    ) -> Result<Option<MediaPlaylistInfo>> {
        let text = self.fetch_playlist_text(media_url).await?;
        let playlist = m3u8_rs::parse_playlist_res(text.as_bytes())
            .map_err(|e| RdlpError::Extraction(format!("M3U8 parse error: {e:?}")))?;

        match playlist {
            m3u8_rs::Playlist::MediaPlaylist(media) => {
                Ok(Some(Self::extract_media_playlist_info(&media)))
            }
            _ => Ok(None),
        }
    }

    /// Extract metadata from a parsed media playlist (no HTTP requests)
    fn extract_media_playlist_info(media: &m3u8_rs::MediaPlaylist) -> MediaPlaylistInfo {
        let segment_count = media.segments.len();
        let total_duration = media.segments.iter().map(|s| s.duration as f64).sum();
        let is_live = !media.end_list;
        let has_encryption = media.segments.iter().any(|s| s.key.is_some());

        // Detect container from first segment URL
        let segment_container = media
            .segments
            .first()
            .and_then(|seg| detect_segment_container(&seg.uri));

        MediaPlaylistInfo {
            segment_count,
            total_duration,
            is_live,
            has_encryption,
            segment_container,
        }
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
            debug!(url:? = m3u8_url; "HLS fetching playlist");
        }

        let playlist_text = self.fetch_playlist_text(m3u8_url).await?;

        if self.verbose {
            debug!(bytes = playlist_text.len(); "HLS playlist size");
        }

        // Parse with m3u8-rs
        let playlist = m3u8_rs::parse_playlist_res(playlist_text.as_bytes())
            .map_err(|e| RdlpError::Extraction(format!("M3U8 parse error: {e:?}")))?;

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
                    debug!(segments = segments.len(); "HLS found segments");
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
                    debug!(
                        variants = master.variants.len(),
                        uri:? = media_playlist_uri,
                        bandwidth = variant.bandwidth;
                        "HLS master playlist detected, selecting first variant"
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
                    debug!(status:? = response.status(); "HLS HEAD request returned non-success");
                }
            }
            Err(e) => {
                if self.verbose {
                    debug!("HLS HEAD request failed: {e}");
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
            debug!(
                segments = total_segments,
                concurrent = self.concurrent_requests;
                "HLS fetching segment sizes"
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
                        debug!("HLS segment size detection failed: {e}");
                    }
                    failed += 1;
                }
            }
        }

        let success_rate = successful as f64 / total_segments as f64;

        if self.verbose {
            debug!(
                successful,
                total = total_segments,
                success_rate:? = success_rate * 100.0;
                "HLS segment size results"
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
            debug!(
                successful,
                total = total_segments;
                "HLS low success rate, size may be inaccurate"
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
/// Tuple of (formats with sizes/segment counts populated, stream-level flags)
pub async fn detect_format_sizes(
    formats: Vec<rdlp_core::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
) -> (Vec<rdlp_core::Format>, HlsStreamFlags) {
    use futures::future::join_all;
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

                // Track stream-level flags from HLS detection
                let mut detected_is_live = None;
                let mut detected_has_encryption = None;

                if is_hls {
                    // Detect HLS metadata (segment count, codecs, resolution, etc.)
                    let result = timeout(
                        Duration::from_secs(5),
                        hls_detector.detect_hls_metadata(&url),
                    )
                    .await;

                    match result {
                        Ok(Ok(Some(info))) => {
                            format.filesize_approx = Some(info.segment_count as u64);

                            // Enrich format with M3U8 metadata (overrides hardcoded values)
                            if let Some((w, h)) = info.resolution {
                                format.width = Some(w as u32);
                                format.height = Some(h as u32);
                            }
                            if let Some(vc) = info.video_codec {
                                format.vcodec = Some(vc);
                            }
                            if let Some(ac) = info.audio_codec {
                                format.acodec = Some(ac);
                            }
                            if let Some(fr) = info.frame_rate {
                                format.fps = Some(fr);
                            }
                            if let Some(bw) = info.average_bandwidth.or(info.bandwidth) {
                                format.tbr = Some(bw as f64 / 1000.0);
                            }
                            if let Some(dur) = info.total_duration {
                                format.duration = Some(dur);
                            }
                            if info.has_encryption {
                                format.has_drm = Some(true);
                            }
                            if let Some(container) = info.segment_container {
                                format.container = Some(container);
                            }

                            // Capture stream-level flags for aggregation
                            detected_is_live = Some(info.is_live);
                            detected_has_encryption = Some(info.has_encryption);

                            if verbose {
                                debug!(
                                    extractor:? = extractor_name,
                                    format:? = format.format_id,
                                    segments = info.segment_count,
                                    resolution:? = info.resolution,
                                    video_codec:? = format.vcodec,
                                    audio_codec:? = format.acodec,
                                    fps:? = format.fps,
                                    tbr:? = format.tbr,
                                    is_live = info.is_live;
                                    "HLS metadata detected"
                                );
                            }
                        }
                        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                            if verbose {
                                debug!(extractor:? = extractor_name, url:? = url; "Could not detect HLS metadata");
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

                (format, detected_is_live, detected_has_encryption)
            }
        })
        .collect();

    let results = join_all(detection_futures).await;

    // Separate formats from flags and aggregate stream-level properties
    let mut formats = Vec::with_capacity(results.len());
    let mut flags = HlsStreamFlags::default();

    for (format, is_live, has_encryption) in results {
        formats.push(format);
        if is_live.unwrap_or(false) {
            flags.is_live = true;
        }
        if has_encryption.unwrap_or(false) {
            flags.has_any_drm = true;
        }
    }

    (formats, flags)
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
