//! HLS segment URL parsing and size calculation
//!
//! Handles parsing M3U8 playlists to extract segment URLs and
//! calculating total stream size via parallel HTTP HEAD requests.

use super::detector::HlsSizeDetector;
use crate::base::common::BaseExtractor;
use futures::stream::{self, StreamExt};
use log::debug;
use rdlp_core::{RdlpError, Result};

/// Maximum number of segments to process (security limit)
const MAX_SEGMENTS: usize = 10_000;

impl HlsSizeDetector {
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
    pub(super) async fn parse_playlist(&self, m3u8_url: &str) -> Result<Vec<String>> {
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
                RdlpError::Extraction {
                    message: format!(
                        "Server returned invalid M3U8 (likely expired token or CDN error): {preview}"
                    ),
                    url: Some(m3u8_url.to_string().into()),
                }
            } else {
                RdlpError::Extraction {
                    message: format!("M3U8 parse error: {e:?}"),
                    url: Some(m3u8_url.to_string().into()),
                }
            }
        })?;

        // Extract segment URLs
        match playlist {
            m3u8_rs::Playlist::MediaPlaylist(media) => {
                let base_url = url::Url::parse(m3u8_url).map_err(|e| RdlpError::Extraction {
                    message: format!("Invalid base URL: {e}"),
                    url: Some(m3u8_url.to_string().into()),
                })?;

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
                    return Err(RdlpError::Extraction {
                        message: format!(
                            "Playlist has too many segments: {} (max: {MAX_SEGMENTS})",
                            segments.len()
                        ),
                        url: Some(m3u8_url.to_string().into()),
                    });
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
                    return Err(RdlpError::Extraction {
                        message: "Master playlist has no variants".to_string(),
                        url: Some(m3u8_url.to_string().into()),
                    });
                }

                // Select the non-I-frame variant with the highest bandwidth (best quality)
                // INVARIANT: the `.or_else` fallback re-iterates all variants, so
                // `None` is only possible if `master.variants` is empty — but that
                // is guarded by the early-return check above.
                #[allow(clippy::expect_used)]
                let variant = master
                    .variants
                    .iter()
                    .filter(|v| !v.is_i_frame)
                    .max_by_key(|v| v.bandwidth)
                    .or_else(|| master.variants.iter().max_by_key(|v| v.bandwidth))
                    .expect("master playlist has at least one variant");
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
                let base_url = url::Url::parse(m3u8_url).map_err(|e| RdlpError::Extraction {
                    message: format!("Invalid base URL: {e}"),
                    url: Some(m3u8_url.to_string().into()),
                })?;

                let media_playlist_url = base_url
                    .join(media_playlist_uri)
                    .map_err(|e| RdlpError::Extraction {
                        message: format!("Failed to join media playlist URL: {e}"),
                        url: Some(m3u8_url.to_string().into()),
                    })?
                    .to_string();

                // Validate the resolved media playlist URL (SSRF protection)
                BaseExtractor::validate_url_security(&media_playlist_url)?;

                // Recursively parse the media playlist. Each fetch is bounded
                // by the wreq Client's connect_timeout / read_timeout
                // (configured via Config::socket_timeout / Config::read_timeout).
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
    pub(super) async fn fetch_segment_size(&self, segment_url: &str) -> Result<u64> {
        // Try HEAD request first (fast, no download)
        match self.http_client.head(segment_url).send().await {
            Ok(response) if response.status().is_success() => {
                if let Some(size) = response.content_length()
                    && size > 0
                {
                    return Ok(size);
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
                    RdlpError::Network {
                        message: format!(
                            "Timeout for segment: {}",
                            rdlp_redact::RedactedUrl::new(&segment_url)
                        ),
                        url: Some(segment_url.to_string().into()),
                    }
                } else if e.is_connect() {
                    RdlpError::Network {
                        message: format!(
                            "Connection failed for segment: {}: {e}",
                            rdlp_redact::RedactedUrl::new(&segment_url)
                        ),
                        url: Some(segment_url.to_string().into()),
                    }
                } else {
                    RdlpError::Network {
                        message: format!("Failed to fetch segment: {e}"),
                        url: Some(segment_url.to_string().into()),
                    }
                }
            })?;

        if !range_response.status().is_success() {
            // Typed `Http` so retry classification can pattern-match on
            // integer status (`is_retryable_error`): 4xx fast-fail, 5xx
            // retry.
            return Err(RdlpError::Http {
                status: range_response.status().as_u16(),
                reason: format!(
                    "segment HEAD: {}",
                    rdlp_redact::RedactedUrl::new(&segment_url)
                ),
            });
        }

        // Try Content-Range header: "bytes 0-0/123456"
        if let Some(size) = range_response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').nth(1))
            .and_then(|total| total.parse::<u64>().ok())
        {
            return Ok(size);
        }

        // Last fallback: Content-Length from Range response
        if let Some(size) = range_response.content_length() {
            return Ok(size);
        }

        Err(RdlpError::Extraction {
            message: format!(
                "Could not detect segment size for: {}",
                rdlp_redact::RedactedUrl::new(&segment_url)
            ),
            url: Some(segment_url.to_string().into()),
        })
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
    pub(super) async fn sum_segment_sizes(&self, segment_urls: Vec<String>) -> Result<u64> {
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
            return Err(RdlpError::Extraction {
                message: format!(
                    "Too many segment failures: {failed}/{total_segments} failed (< 50% success rate)"
                ),
                url: None,
            });
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
