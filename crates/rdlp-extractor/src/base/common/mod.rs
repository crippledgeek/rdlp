//! Common base extractor utilities for all site extractors
//!
//! This module provides shared extraction logic that can be used by any extractor,
//! reducing code duplication and ensuring consistent behavior across sites.
//!
//! # Architecture
//!
//! The base extractor provides a three-tier hierarchy:
//! 1. **BaseExtractor** - Generic utilities for all extractors (this module)
//! 2. **Network Bases** - Protocol-specific logic (e.g., TnaFlixNetworkBase)
//! 3. **Site Extractors** - Site-specific implementations
//!
//! # Features
//!
//! - **Webpage Fetching**: Standardized HTTP requests with error handling and verbose logging
//! - **URL ID Extraction**: Generic regex-based ID extraction from URLs
//! - **Size Detection**: HEAD request with Range fallback for filesize detection
//! - **Format Building**: Standard format creation with quality parsing
//! - **Logging Utilities**: Consistent verbose output across extractors
//!
//! # Example
//!
//! ```rust,ignore
//! use rdlp_extractor::base::common::BaseExtractor;
//!
//! // Fetch a webpage with automatic error handling and verbose logging
//! let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
//!
//! // Extract video ID from URL
//! let id = BaseExtractor::extract_id_from_url(url, &MY_URL_PATTERN, "id");
//!
//! // Detect file size with fallback strategies
//! let size = BaseExtractor::detect_file_size(&video_url, &ctx.http_client, None).await;
//! ```

pub mod dash;
pub mod json_ld;
mod metadata;
mod parsing;
pub(crate) mod protocol;
pub mod selector_macro;
mod selectors;
mod string_utils;
#[cfg(test)]
mod tests;

use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result, check_http_response};
use rdlp_types::Codec;
use rdlp_types::Format;
use regex::Regex;

// Re-export selectors, patterns, and constants from submodule
pub(crate) use selectors::*;
pub(crate) use protocol::protocol_for_url;

/// Maximum URL length to prevent memory exhaustion attacks
/// Re-exported from rdlp-security for backward compatibility
pub(crate) use rdlp_security::MAX_URL_LENGTH;

/// Maximum bytes a single webpage fetch will accept before aborting.
///
/// Adversarial servers (or compromised CDNs) can stream gigabytes of
/// payload at us; without a cap, `response.text().await` would buffer
/// the entire body and OOM the host. 50 MB covers any realistic HTML +
/// JSON-LD payload with multiple orders of magnitude of headroom.
pub(crate) const MAX_WEBPAGE_BYTES: usize = 50 * 1024 * 1024;

// ============================================================================
// Base Extractor
// ============================================================================

/// Common base extractor providing shared utilities for all extractors
///
/// This struct provides static methods for common extraction tasks like
/// fetching webpages, extracting IDs from URLs, detecting file sizes, etc.
///
/// # Usage
///
/// ```rust,ignore
/// use rdlp_extractor::base::common::BaseExtractor;
///
/// // In your extractor's extract() method:
/// let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
/// let id = BaseExtractor::extract_id_from_url(url, &MY_PATTERN, "id")
///     .ok_or_else(|| RdlpError::Extraction {
///         message: "Could not extract video ID".to_string(),
///         url: Some(url.to_string()),
///     })?;
/// ```
pub struct BaseExtractor;

/// Read an HTTP response body as UTF-8 with a size cap.
///
/// Streams via `bytes_stream()` so the cap fires the moment cumulative
/// bytes exceed `MAX_WEBPAGE_BYTES`. The previous `response.text()` path
/// buffered the entire response before any check, allowing an
/// adversarial server to OOM the host with a 10 GB body.
async fn read_capped_text(response: wreq::Response, url: &str) -> Result<String> {
    use futures::StreamExt;
    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| RdlpError::Network {
            message: format!("Failed to read response body: {e}"),
            url: Some(url.to_string()),
        })?;
        let chunk_ref: &[u8] = bytes.as_ref();
        if buf.len().saturating_add(chunk_ref.len()) > MAX_WEBPAGE_BYTES {
            return Err(RdlpError::Network {
                message: format!(
                    "Response body exceeds {MAX_WEBPAGE_BYTES}-byte cap (host integrity guard)"
                ),
                url: Some(url.to_string()),
            });
        }
        buf.extend_from_slice(chunk_ref);
    }
    String::from_utf8(buf).map_err(|e| RdlpError::Network {
        message: format!("Response body is not valid UTF-8: {e}"),
        url: Some(url.to_string()),
    })
}

impl BaseExtractor {
    // ========================================================================
    // Webpage Fetching
    // ========================================================================

    /// Fetch a webpage with standardized error handling and verbose logging
    ///
    /// This method:
    /// 1. Validates URL length for security
    /// 2. Makes the HTTP GET request
    /// 3. Checks the response status
    /// 4. Reads the response body
    /// 5. Logs debug output if verbose mode is enabled
    ///
    /// # Arguments
    /// * `url` - The URL to fetch
    /// * `ctx` - Extraction context with HTTP client and config
    ///
    /// # Returns
    /// The webpage content as a string
    ///
    /// # Errors
    /// - `RdlpError::Extraction` if URL is too long
    /// - `RdlpError::Network` if the request fails
    /// - `RdlpError::Http` if the response has a non-2xx status
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
    /// let html = Html::parse_document(&webpage);
    /// ```
    pub(crate) async fn fetch_webpage(url: &str, ctx: &ExtractionContext) -> Result<String> {
        // Security: Validate URL length
        if url.len() > MAX_URL_LENGTH {
            return Err(RdlpError::Extraction {
                message: format!("URL too long: {} bytes (max: {MAX_URL_LENGTH})", url.len()),
                url: Some(url.to_string()),
            });
        }

        let response = ctx
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| RdlpError::Network {
                message: format!("Failed to fetch webpage: {e}"),
                url: Some(url.to_string()),
            })?;

        check_http_response(&response)?;

        let webpage = read_capped_text(response, url).await?;

        // Debug output if verbose
        if ctx.config.verbose {
            crate::utils::debug_print_webpage_sample(&webpage, DEFAULT_DEBUG_SAMPLE_SIZE);
        }

        Ok(webpage)
    }

    /// Fetch a webpage with custom headers
    ///
    /// Same as `fetch_webpage` but allows specifying additional headers.
    ///
    /// # Arguments
    /// * `url` - The URL to fetch
    /// * `headers` - Slice of (name, value) header tuples
    /// * `ctx` - Extraction context
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let webpage = BaseExtractor::fetch_webpage_with_headers(
    ///     url,
    ///     &[("Referer", "https://example.com"), ("X-Requested-With", "XMLHttpRequest")],
    ///     ctx
    /// ).await?;
    /// ```
    pub(crate) async fn fetch_webpage_with_headers(
        url: &str,
        headers: &[(&str, &str)],
        ctx: &ExtractionContext,
    ) -> Result<String> {
        // Security: Validate URL length
        if url.len() > MAX_URL_LENGTH {
            return Err(RdlpError::Extraction {
                message: format!("URL too long: {} bytes (max: {MAX_URL_LENGTH})", url.len()),
                url: Some(url.to_string()),
            });
        }

        let mut request = ctx.http_client.get(url);

        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = request.send().await.map_err(|e| RdlpError::Network {
            message: format!("Failed to fetch webpage: {e}"),
            url: Some(url.to_string()),
        })?;

        check_http_response(&response)?;

        let webpage = read_capped_text(response, url).await?;

        if ctx.config.verbose {
            crate::utils::debug_print_webpage_sample(&webpage, DEFAULT_DEBUG_SAMPLE_SIZE);
        }

        Ok(webpage)
    }

    // ========================================================================
    // URL Validation (Security)
    // ========================================================================
    // Note: These functions have been moved to rdlp-security crate
    // Wrappers provided here for backward compatibility

    /// Validate a URL for security concerns (SSRF protection)
    ///
    /// **Note**: This function delegates to [`rdlp_security::validate_url_security`].
    /// It wraps the SecurityError into RdlpError for backward compatibility.
    ///
    /// Checks that the URL:
    /// 1. Uses http or https scheme
    /// 2. Does not point to private/internal IP addresses
    /// 3. Is within length limits
    ///
    /// # Arguments
    /// * `url` - The URL to validate
    ///
    /// # Returns
    /// `Ok(())` if the URL is safe, otherwise an error describing the issue
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// BaseExtractor::validate_url_security(segment_url)?;
    /// ```
    pub(crate) fn validate_url_security(url: &str) -> Result<()> {
        rdlp_security::validate_url_security(url).map_err(|e| RdlpError::Extraction {
            message: e.to_string(),
            url: Some(url.to_string()),
        })
    }

    // ========================================================================
    // URL ID Extraction
    // ========================================================================

    /// Extract an ID from a URL using a regex pattern
    ///
    /// This is a common pattern across all extractors - matching a URL against
    /// a regex and extracting a named capture group.
    ///
    /// # Arguments
    /// * `url` - The URL to extract from
    /// * `pattern` - The regex pattern with a named capture group
    /// * `group_name` - The name of the capture group to extract
    ///
    /// # Returns
    /// The extracted ID if the pattern matches, `None` otherwise
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// static VIDEO_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    ///     Regex::new(r"example\.com/video/(?P<id>\d+)").unwrap()
    /// });
    ///
    /// let id = BaseExtractor::extract_id_from_url(url, &VIDEO_PATTERN, "id");
    /// ```
    #[must_use]
    pub(crate) fn extract_id_from_url(
        url: &str,
        pattern: &Regex,
        group_name: &str,
    ) -> Option<String> {
        pattern
            .captures(url)
            .and_then(|cap| cap.name(group_name))
            .map(|m| m.as_str().to_string())
    }

    /// Extract an ID from a URL using positional capture groups
    ///
    /// Tries multiple capture groups in order, returning the first match.
    /// Useful when the ID can be in different positions depending on URL format.
    ///
    /// # Arguments
    /// * `url` - The URL to extract from
    /// * `pattern` - The regex pattern with positional capture groups
    /// * `group_indices` - Slice of group indices to try (1-based)
    ///
    /// # Returns
    /// The extracted ID from the first matching group, `None` if no match
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Pattern where ID can be in group 1 or group 2
    /// static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    ///     Regex::new(r"example\.com/(?:video/(\d+)|v(\d+))").unwrap()
    /// });
    ///
    /// let id = BaseExtractor::extract_id_positional(url, &PATTERN, &[1, 2]);
    /// ```
    #[must_use]
    pub(crate) fn extract_id_positional(
        url: &str,
        pattern: &Regex,
        group_indices: &[usize],
    ) -> Option<String> {
        pattern.captures(url).and_then(|cap| {
            for &idx in group_indices {
                if let Some(m) = cap.get(idx) {
                    let value = m.as_str();
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
            None
        })
    }

    // ========================================================================
    // Size Detection
    // ========================================================================

    /// Detect file size using HEAD request with Range fallback.
    ///
    /// Tries two strategies:
    /// 1. HEAD request to get Content-Length header
    /// 2. Range request (bytes=0-0) to parse Content-Range header
    ///
    /// # Arguments
    /// * `url` - The URL to check
    /// * `http_client` - The HTTP client to use
    /// * `log_prefix` - Optional prefix for debug log messages
    ///
    /// # Returns
    /// File size in bytes if detected, `None` otherwise
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let size = BaseExtractor::detect_file_size(&url, &ctx.http_client, None).await;
    /// let size = BaseExtractor::detect_file_size(&url, &client, Some("HLS")).await;
    /// ```
    pub(crate) async fn detect_file_size(
        url: &str,
        http_client: &wreq::Client,
        log_prefix: Option<&str>,
    ) -> Option<u64> {
        // Strategy 1: HEAD request
        if let Ok(response) = http_client.head(url).send().await
            && let Some(size) = response.content_length().filter(|&s| s > 0)
        {
            if let Some(prefix) = log_prefix {
                debug!(size, method = "HEAD"; "[{prefix}] Detected Content-Length");
            }
            return Some(size);
        }

        // Strategy 2: Range request fallback
        if let Ok(response) = http_client
            .get(url)
            .header("Range", "bytes=0-0")
            .send()
            .await
            && let Some(size) = Self::parse_content_range_total(response.headers())
        {
            if let Some(prefix) = log_prefix {
                debug!(size, method = "Range"; "[{prefix}] Detected Content-Range");
            }
            return Some(size);
        }

        None
    }

    /// Parse total file size from a Content-Range header.
    ///
    /// Parses the format `bytes 0-0/123456` and returns the total size.
    fn parse_content_range_total(headers: &wreq::header::HeaderMap) -> Option<u64> {
        headers
            .get("content-range")?
            .to_str()
            .ok()?
            .split('/')
            .nth(1)?
            .parse()
            .ok()
    }

    // ========================================================================
    // Format Building
    // ========================================================================

    /// Calculate width from height assuming 16:9 aspect ratio.
    ///
    /// Uses a lookup table for standard resolutions to return the exact
    /// display width (e.g. 480p → 854, not 853). Falls back to integer
    /// math for non-standard heights.
    ///
    /// # Arguments
    /// * `height` - Video height in pixels
    ///
    /// # Returns
    /// Calculated width in pixels
    #[must_use]
    #[inline]
    pub(crate) fn width_from_height(height: u32) -> u32 {
        match height {
            240 => 426,
            360 => 640,
            480 => 854,
            720 => 1280,
            1080 => 1920,
            1440 => 2560,
            2160 => 3840,
            _ => (height * 16) / 9,
        }
    }

    /// Parse quality height from a string (e.g., "720p" -> 720)
    ///
    /// # Arguments
    /// * `quality_str` - Quality string like "720p", "1080P", "720"
    ///
    /// # Returns
    /// Parsed height as u32, `None` if parsing fails
    #[must_use]
    pub(crate) fn parse_quality_height(quality_str: &str) -> Option<u32> {
        quality_str.trim_end_matches(['p', 'P']).parse::<u32>().ok()
    }

    /// Parse quality from URL using common patterns
    ///
    /// Looks for patterns like "720p", "1080P", etc. in the URL.
    ///
    /// # Arguments
    /// * `url` - The URL to parse
    ///
    /// # Returns
    /// Parsed quality height, `None` if not found
    #[allow(dead_code)]
    pub(crate) fn parse_quality_from_url(url: &str) -> Option<u32> {
        QUALITY_FROM_URL_PATTERN
            .captures(url)
            .and_then(|cap| cap.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
    }

    /// Build a standard format with quality metadata
    ///
    /// Creates a Format struct with common fields populated based on quality.
    ///
    /// # Arguments
    /// * `format_id` - Unique identifier for this format
    /// * `url` - Video URL
    /// * `ext` - File extension (mp4, webm, etc.)
    /// * `height` - Video height in pixels (optional)
    ///
    /// # Returns
    /// A Format struct with quality metadata populated
    #[must_use]
    pub(crate) fn build_format(
        format_id: impl Into<String>,
        url: impl Into<String>,
        ext: impl Into<String>,
        height: Option<u32>,
    ) -> Format {
        let ext = ext.into();
        let mut format = Format::new(format_id, url, &ext, rdlp_types::DownloadProtocol::Https);

        if let Some(h) = height {
            format.height = Some(h);
            format.width = Some(Self::width_from_height(h));
            format.format_note = Some(format!("{h}p"));

            // Set quality score (higher resolution = higher quality)
            format.quality = Some((h / 100) as i32);
        }

        // Set default codecs for common formats
        match ext.as_str() {
            "mp4" => {
                format.vcodec = Codec::from("h264".to_string());
                format.acodec = Codec::from("aac".to_string());
            }
            "webm" => {
                format.vcodec = Codec::from("vp9".to_string());
                format.acodec = Codec::from("opus".to_string());
            }
            _ => {}
        }

        format
    }

    /// Ensure format_ids are unique by appending "-2", "-3", etc. for duplicates.
    ///
    /// The desktop UI uses `format_id` for row selection (`===` comparison),
    /// so duplicate IDs cause multiple rows to highlight on a single click.
    pub(crate) fn dedup_format_ids(formats: &mut [Format]) {
        let mut id_counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        for format in formats.iter_mut() {
            let count = id_counts.entry(format.format_id.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                format.format_id = format!("{}-{}", format.format_id, count);
            }
        }
    }
}
