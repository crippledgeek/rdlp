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

/// Per-variant information from an HLS master playlist.
///
/// Each variant represents a specific quality level (e.g., 720p, 1080p)
/// with its own media playlist URL and metadata.
#[derive(Debug, Clone)]
pub struct HlsVariantInfo {
    /// Resolved absolute URL to this variant's media playlist
    pub media_playlist_url: String,
    /// Video resolution (width, height)
    pub resolution: Option<(u64, u64)>,
    /// Parsed video codec name (e.g., "h264", "av1")
    pub video_codec: Option<String>,
    /// Parsed audio codec name (e.g., "aac", "opus")
    pub audio_codec: Option<String>,
    /// Frame rate
    pub frame_rate: Option<f64>,
    /// Peak bandwidth in bits per second
    pub bandwidth: u64,
    /// Average bandwidth in bits per second
    pub average_bandwidth: Option<u64>,
    // Shared fields (from one media playlist, applied to all variants):
    /// Number of segments
    pub segment_count: usize,
    /// Total duration in seconds
    pub total_duration: Option<f64>,
    /// Whether the stream is live
    pub is_live: bool,
    /// Whether segments use encryption
    pub has_encryption: bool,
    /// Detected segment container format
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
        let segment_container = segment_urls
            .first()
            .and_then(|url| detect_segment_container(url));

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

                // Select the non-I-frame variant with the highest bandwidth (best quality)
                let variant = master
                    .variants
                    .iter()
                    .filter(|v| !v.is_i_frame)
                    .max_by_key(|v| v.bandwidth)
                    .or_else(|| master.variants.iter().max_by_key(|v| v.bandwidth))
                    .unwrap(); // safe: checked non-empty above

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

    /// Detect all quality variants from an HLS master playlist.
    ///
    /// Returns one `HlsVariantInfo` per variant in a master playlist, each with
    /// a resolved media playlist URL and per-variant metadata. Shared metadata
    /// (segment count, duration, etc.) is fetched from the best variant's media
    /// playlist and applied to all entries.
    ///
    /// For media playlists (non-master), returns an empty Vec.
    pub async fn detect_hls_variants(&self, m3u8_url: &str) -> Result<Vec<HlsVariantInfo>> {
        BaseExtractor::validate_url_security(m3u8_url)?;

        let playlist_text = match self.fetch_playlist_text(m3u8_url).await {
            Ok(text) => text,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to fetch playlist for variant expansion: {e}");
                }
                return Ok(Vec::new());
            }
        };

        let playlist = match m3u8_rs::parse_playlist_res(playlist_text.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to parse playlist for variant expansion: {e:?}");
                }
                return Ok(Vec::new());
            }
        };

        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            m3u8_rs::Playlist::MediaPlaylist(_) => {
                // Not a master playlist — caller should fall back to detect_hls_metadata
                return Ok(Vec::new());
            }
        };

        if master.variants.is_empty() {
            return Ok(Vec::new());
        }

        let base_url = url::Url::parse(m3u8_url)
            .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

        // Resolve all variant media playlist URLs and extract per-variant metadata.
        // Skip I-frame-only variants (EXT-X-I-FRAME-STREAM-INF) — these are trick-play
        // playlists whose segments are not downloadable as standalone media.
        let mut variants: Vec<HlsVariantInfo> = Vec::with_capacity(master.variants.len());
        for variant in master.variants.iter().filter(|v| !v.is_i_frame) {
            let media_url = match base_url.join(&variant.uri) {
                Ok(u) => u.to_string(),
                Err(_) => continue,
            };
            let (video_codec, audio_codec) = variant
                .codecs
                .as_deref()
                .map(rdlp_core::parse_hls_codecs)
                .unwrap_or((None, None));

            variants.push(HlsVariantInfo {
                media_playlist_url: media_url,
                resolution: variant.resolution.as_ref().map(|r| (r.width, r.height)),
                video_codec: video_codec.map(String::from),
                audio_codec: audio_codec.map(String::from),
                frame_rate: variant.frame_rate,
                bandwidth: variant.bandwidth,
                average_bandwidth: variant.average_bandwidth,
                // Shared fields filled below
                segment_count: 0,
                total_duration: None,
                is_live: false,
                has_encryption: false,
                segment_container: None,
            });
        }

        // Fetch shared media info from the best non-I-frame variant (highest bandwidth)
        let best_variant = master
            .variants
            .iter()
            .filter(|v| !v.is_i_frame)
            .max_by_key(|v| v.bandwidth)
            .unwrap();
        let best_media_url = base_url
            .join(&best_variant.uri)
            .map_err(|e| RdlpError::Extraction(format!("Failed to join media URL: {e}")))?
            .to_string();

        if let Ok(Some(media_info)) = self.fetch_and_extract_media_info(&best_media_url).await {
            for v in &mut variants {
                v.segment_count = media_info.segment_count;
                v.total_duration = Some(media_info.total_duration);
                v.is_live = media_info.is_live;
                v.has_encryption = media_info.has_encryption;
                v.segment_container = media_info.segment_container.clone();
            }
        }

        if self.verbose {
            debug!(
                variants = variants.len(),
                url:? = m3u8_url;
                "HLS master playlist expanded into variants"
            );
        }

        Ok(variants)
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
        let playlist = m3u8_rs::parse_playlist_res(text.as_bytes()).map_err(|e| {
            if !text.trim().starts_with("#EXTM3U") {
                let preview: String = text.chars().take(200).collect();
                RdlpError::Extraction(format!(
                    "Server returned invalid M3U8 (likely expired token or CDN error): {preview}"
                ))
            } else {
                RdlpError::Extraction(format!("M3U8 parse error: {e:?}"))
            }
        })?;

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
        let playlist = m3u8_rs::parse_playlist_res(playlist_text.as_bytes()).map_err(|e| {
            if !playlist_text.trim().starts_with("#EXTM3U") {
                let preview: String = playlist_text.chars().take(200).collect();
                RdlpError::Extraction(format!(
                    "Server returned invalid M3U8 (likely expired token or CDN error): {preview}"
                ))
            } else {
                RdlpError::Extraction(format!("M3U8 parse error: {e:?}"))
            }
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

                // Select the non-I-frame variant with the highest bandwidth (best quality)
                let variant = master
                    .variants
                    .iter()
                    .filter(|v| !v.is_i_frame)
                    .max_by_key(|v| v.bandwidth)
                    .or_else(|| master.variants.iter().max_by_key(|v| v.bandwidth))
                    .unwrap(); // safe: checked non-empty above
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

/// Detect video or audio codec from a format ID string.
///
/// Checks for common codec names embedded in format IDs like "hls-av1-url"
/// or "hls-h264-fallback". Returns `None` if no codec is detected.
fn detect_codec_from_id(format_id: &str, is_video: bool) -> Option<String> {
    let id = format_id.to_lowercase();
    if is_video {
        if id.contains("av1") || id.contains("av01") {
            Some("av1".to_string())
        } else if id.contains("h264") || id.contains("avc") {
            Some("h264".to_string())
        } else if id.contains("h265") || id.contains("hevc") || id.contains("hvc") {
            Some("hevc".to_string())
        } else if id.contains("vp9") || id.contains("vp09") {
            Some("vp9".to_string())
        } else {
            None
        }
    } else if id.contains("aac") || id.contains("mp4a") {
        Some("aac".to_string())
    } else if id.contains("opus") {
        Some("opus".to_string())
    } else {
        None
    }
}

/// Enrich a single HLS format with metadata from `detect_hls_metadata()`.
///
/// Used as a fallback when the HLS URL is a media playlist (not a master)
/// or when variant expansion fails.
///
/// Returns `(Option<bool>, Option<bool>)` — `(is_live, has_encryption)`.
async fn enrich_single_hls_format(
    format: &mut rdlp_core::Format,
    hls_detector: &HlsSizeDetector,
    url: &str,
    extractor_name: &str,
    verbose: bool,
) -> (Option<bool>, Option<bool>) {
    use std::time::Duration;
    use tokio::time::timeout;

    let result = timeout(
        Duration::from_secs(10),
        hls_detector.detect_hls_metadata(url),
    )
    .await;

    let hls_info = match result {
        Ok(Ok(Some(info))) => info,
        _ => {
            if verbose {
                debug!(
                    extractor:? = extractor_name,
                    format:? = format.format_id;
                    "HLS metadata detection failed or timed out"
                );
            }
            return (None, None);
        }
    };

    // Enrich format with metadata
    if let Some((w, h)) = hls_info.resolution {
        format.width = Some(w as u32);
        format.height = Some(h as u32);
        format.format_note = Some(format!("{h}p"));
    }
    if let Some(vc) = &hls_info.video_codec {
        format.vcodec = Some(vc.clone());
    }
    if let Some(ac) = &hls_info.audio_codec {
        format.acodec = Some(ac.clone());
    }
    format.fps = hls_info.frame_rate;
    if let Some(bw) = hls_info.bandwidth {
        format.tbr = Some(bw as f64 / 1000.0);
    }
    format.duration = hls_info.total_duration;
    format.filesize_approx = Some(hls_info.segment_count as u64);
    format.container = hls_info.segment_container;
    if hls_info.has_encryption {
        format.has_drm = Some(true);
    }
    if let Some(h) = format.height {
        format.quality = Some((h / 100) as i32);
    }

    if verbose {
        debug!(
            extractor:? = extractor_name,
            format:? = format.format_id,
            resolution:? = hls_info.resolution,
            segments = hls_info.segment_count;
            "HLS single format enriched"
        );
    }

    (Some(hls_info.is_live), Some(hls_info.has_encryption))
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
                let url = format.url.clone();
                let is_hls = format.ext == "hls" || url.contains(".m3u8") || url.contains("/hls/");

                if is_hls {
                    // Try to expand master playlist into per-variant formats
                    let result = timeout(
                        Duration::from_secs(10),
                        hls_detector.detect_hls_variants(&url),
                    )
                    .await;

                    let variants = match result {
                        Ok(Ok(v)) if v.len() > 1 => v,
                        _ => {
                            // Not a master playlist or detection failed — fall back to
                            // single-format enrichment via detect_hls_metadata
                            let mut format = format;
                            let (is_live, has_enc) = enrich_single_hls_format(
                                &mut format,
                                &hls_detector,
                                &url,
                                &extractor_name,
                                verbose,
                            )
                            .await;
                            return vec![(format, is_live, has_enc)];
                        }
                    };

                    // Expand master playlist into one format per variant
                    let mut expanded = Vec::with_capacity(variants.len());
                    for variant in &variants {
                        let height = variant.resolution.map(|(_, h)| h as u32);
                        let width = variant.resolution.map(|(w, _)| w as u32);
                        let format_id = if let Some(h) = height {
                            format!("{}-{h}p", format.format_id)
                        } else {
                            format!("{}-{}k", format.format_id, variant.bandwidth / 1000)
                        };

                        let mut expanded_format = rdlp_core::Format::new(
                            &format_id,
                            &variant.media_playlist_url,
                            &format.ext,
                            format.protocol.clone(),
                        );
                        expanded_format.height = height;
                        expanded_format.width = width;
                        expanded_format.vcodec = variant
                            .video_codec
                            .clone()
                            .or_else(|| format.vcodec.clone())
                            .or_else(|| detect_codec_from_id(&format.format_id, true));
                        expanded_format.acodec = variant
                            .audio_codec
                            .clone()
                            .or_else(|| format.acodec.clone())
                            .or_else(|| detect_codec_from_id(&format.format_id, false));
                        expanded_format.fps = variant.frame_rate;
                        expanded_format.tbr = Some(variant.bandwidth as f64 / 1000.0);
                        expanded_format.http_headers = format.http_headers.clone();
                        expanded_format.filesize_approx = Some(variant.segment_count as u64);
                        expanded_format.duration = variant.total_duration;
                        expanded_format.container = variant.segment_container.clone();
                        if variant.has_encryption {
                            expanded_format.has_drm = Some(true);
                        }
                        if let Some(h) = height {
                            expanded_format.format_note = Some(format!("{h}p"));
                            expanded_format.quality = Some((h / 100) as i32);
                        }

                        let is_live = Some(variant.is_live);
                        let has_enc = Some(variant.has_encryption);
                        expanded.push((expanded_format, is_live, has_enc));
                    }

                    if verbose {
                        debug!(
                            extractor:? = extractor_name,
                            parent:? = format.format_id,
                            variants = expanded.len();
                            "HLS master expanded into per-quality formats"
                        );
                    }

                    expanded
                } else {
                    // Non-HLS: HEAD request for file size
                    let mut format = format;
                    let result = timeout(
                        Duration::from_secs(5),
                        BaseExtractor::detect_file_size_with_client(&url, &http_client),
                    )
                    .await;

                    if let Ok(Some(size)) = result {
                        format.filesize = Some(size);
                    }

                    vec![(format, None, None)]
                }
            }
        })
        .collect();

    let results = join_all(detection_futures).await;

    // Flatten expanded formats, deduplicate HLS CDN mirrors, aggregate flags
    let mut formats: Vec<rdlp_core::Format> = Vec::new();
    let mut flags = HlsStreamFlags::default();
    let mut seen_hls: std::collections::HashSet<(Option<u32>, Option<String>, Option<String>)> =
        std::collections::HashSet::new();

    for format_group in results {
        for (format, is_live, has_encryption) in format_group {
            if is_live.unwrap_or(false) {
                flags.is_live = true;
            }
            if has_encryption.unwrap_or(false) {
                flags.has_any_drm = true;
            }

            // Deduplicate expanded HLS formats: keep format with most segments per (height, vcodec, acodec),
            // collect other URLs as fallbacks on the kept format.
            // This handles CDN bugs where some sources have incomplete playlists (missing first segments).
            // Note: HLS segment count is stored in filesize_approx during extraction.
            if format.is_hls() {
                let key = (format.height, format.vcodec.clone(), format.acodec.clone());
                if !seen_hls.insert(key) {
                    // Find existing format with same key
                    if let Some(existing) = formats.iter_mut().find(|f| {
                        f.is_hls()
                            && f.height == format.height
                            && f.vcodec == format.vcodec
                            && f.acodec == format.acodec
                    }) {
                        // Compare segment counts (stored in filesize_approx for HLS)
                        // Keep the one with more segments (more complete playlist)
                        let existing_segments = existing.filesize_approx.unwrap_or(0);
                        let new_segments = format.filesize_approx.unwrap_or(0);

                        if new_segments > existing_segments {
                            // New format has more segments - swap: make existing the fallback
                            let old_url = std::mem::replace(&mut existing.url, format.url.clone());
                            existing.filesize_approx = format.filesize_approx;
                            existing.duration = format.duration;
                            existing.filesize = format.filesize;
                            existing
                                .fallback_urls
                                .get_or_insert_with(Vec::new)
                                .push(old_url);
                        } else {
                            // Existing has equal or more segments - keep it, add new as fallback
                            existing
                                .fallback_urls
                                .get_or_insert_with(Vec::new)
                                .push(format.url.clone());
                        }
                    }
                    continue;
                }
            }

            formats.push(format);
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
