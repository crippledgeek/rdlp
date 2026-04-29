//! HLS playlist size detector
//!
//! Provides `HlsSizeDetector` which fetches M3U8 playlists, parses them,
//! and calculates total size by aggregating segment sizes via parallel
//! HTTP HEAD requests.

use super::types::{HlsInfo, MediaPlaylistInfo, detect_segment_container};
use crate::base::common::BaseExtractor;
use log::debug;
use rdlp_core::{RdlpError, Result};
use std::sync::Arc;

/// Default number of concurrent HTTP requests
const DEFAULT_CONCURRENCY: usize = 8;

/// HLS playlist size detector
#[derive(Clone)]
pub struct HlsSizeDetector {
    pub(super) http_client: Arc<wreq::Client>,
    pub(super) concurrent_requests: usize,
    pub(super) verbose: bool,
    /// Optional default headers applied to every M3U8 fetch (e.g., Referer).
    default_headers: Option<wreq::header::HeaderMap>,
}

impl HlsSizeDetector {
    /// Create a new HLS size detector
    ///
    /// # Arguments
    /// * `http_client` - Shared HTTP client for making requests
    /// * `verbose` - Enable detailed logging
    #[must_use]
    pub fn new(http_client: Arc<wreq::Client>, verbose: bool) -> Self {
        Self {
            http_client,
            concurrent_requests: DEFAULT_CONCURRENCY,
            verbose,
            default_headers: None,
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

    /// Set default HTTP headers applied to every M3U8 playlist fetch.
    ///
    /// Useful for CDNs that require a `Referer` header to avoid challenges.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_default_headers(mut self, headers: wreq::header::HeaderMap) -> Self {
        self.default_headers = Some(headers);
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
    /// #     std::sync::Arc::new(wreq::Client::new()),
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

    /// Fetch M3U8 playlist text from a URL
    pub(super) async fn fetch_playlist_text(&self, m3u8_url: &str) -> Result<String> {
        let mut request = self.http_client.get(m3u8_url);
        if let Some(headers) = &self.default_headers {
            request = request.headers(headers.clone());
        }
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                RdlpError::Network {
                    message: format!("Timeout fetching playlist: {m3u8_url}"),
                    url: Some(m3u8_url.to_string()),
                }
            } else if e.is_connect() {
                RdlpError::Network {
                    message: format!("Connection failed for playlist: {m3u8_url}: {e}"),
                    url: Some(m3u8_url.to_string()),
                }
            } else {
                RdlpError::Network {
                    message: format!("Failed to fetch playlist: {e}"),
                    url: Some(m3u8_url.to_string()),
                }
            }
        })?;

        if !response.status().is_success() {
            // Use the typed `RdlpError::Http` so retry classification at
            // `is_retryable_error()` can pattern-match on integer status:
            // 4xx fast-fail, 5xx retry. The previous `Network { message }`
            // path was always treated as retryable, causing 4xx playlists
            // (404 / 410 / 403) to needlessly retry.
            return Err(RdlpError::Http {
                status: response.status().as_u16(),
                reason: format!("playlist fetch: {m3u8_url}"),
            });
        }

        response.text().await.map_err(|e| RdlpError::Network {
            message: format!("Failed to read playlist response: {e}"),
            url: Some(m3u8_url.to_string()),
        })
    }

    /// Fetch a media playlist and extract its metadata
    pub(super) async fn fetch_and_extract_media_info(
        &self,
        media_url: &str,
    ) -> Result<Option<MediaPlaylistInfo>> {
        let text = self.fetch_playlist_text(media_url).await?;
        let playlist = m3u8_rs::parse_playlist_res(text.as_bytes()).map_err(|e| {
            if !text.trim().starts_with("#EXTM3U") {
                let preview: String = text.chars().take(200).collect();
                RdlpError::Extraction {
                    message: format!(
                        "Server returned invalid M3U8 (likely expired token or CDN error): {preview}"
                    ),
                    url: Some(media_url.to_string()),
                }
            } else {
                RdlpError::Extraction {
                    message: format!("M3U8 parse error: {e:?}"),
                    url: Some(media_url.to_string()),
                }
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
    pub(super) fn extract_media_playlist_info(media: &m3u8_rs::MediaPlaylist) -> MediaPlaylistInfo {
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
}
