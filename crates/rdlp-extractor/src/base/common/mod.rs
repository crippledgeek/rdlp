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
//! let size = BaseExtractor::detect_file_size(&video_url, ctx).await;
//! ```

#[cfg(test)]
mod tests;

use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, Format, Result, RdlpError};
use regex::Regex;
use scraper::{Html, Selector};
use std::net::IpAddr;

// ============================================================================
// Security Constants
// ============================================================================

/// Maximum URL length to prevent memory exhaustion attacks
pub const MAX_URL_LENGTH: usize = 8192;

/// Maximum title length for extracted metadata
pub const MAX_TITLE_LENGTH: usize = 500;

/// Maximum description length for extracted metadata
pub const MAX_DESCRIPTION_LENGTH: usize = 10_000;

/// Maximum number of videos in a playlist
pub const MAX_PLAYLIST_SIZE: usize = 1000;

/// Default sample size for debug output
pub const DEFAULT_DEBUG_SAMPLE_SIZE: usize = 5000;

// ============================================================================
// Common Static Selectors
// ============================================================================

/// Selector for Open Graph title: `<meta property="og:title" content="...">`
pub static OG_TITLE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:title"]"#).expect("Valid OG title selector")
});

/// Selector for Open Graph description: `<meta property="og:description" content="...">`
pub static OG_DESCRIPTION_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:description"]"#).expect("Valid OG description selector")
});

/// Selector for Open Graph image: `<meta property="og:image" content="...">`
pub static OG_IMAGE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:image"]"#).expect("Valid OG image selector")
});

/// Selector for meta description: `<meta name="description" content="...">`
pub static META_DESCRIPTION_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="description"]"#).expect("Valid meta description selector")
});

/// Selector for Twitter title: `<meta name="twitter:title" content="...">`
pub static TWITTER_TITLE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="twitter:title"]"#).expect("Valid Twitter title selector")
});

/// Selector for Twitter image: `<meta name="twitter:image" content="...">`
pub static TWITTER_IMAGE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="twitter:image"]"#).expect("Valid Twitter image selector")
});

/// Selector for HTML title tag: `<title>...</title>`
pub static TITLE_TAG_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("title").expect("Valid title selector")
});

/// Selector for H1 heading: `<h1>...</h1>`
pub static H1_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("h1").expect("Valid h1 selector")
});

/// Selector for JSON-LD scripts: `<script type="application/ld+json">`
pub static JSONLD_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"script[type="application/ld+json"]"#).expect("Valid JSON-LD selector")
});

/// Selector for canonical link: `<link rel="canonical" href="...">`
pub static CANONICAL_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"link[rel="canonical"]"#).expect("Valid canonical selector")
});

// ============================================================================
// Common Static Patterns
// ============================================================================

/// Pattern to extract quality from URL (e.g., "720p", "1080P")
pub static QUALITY_FROM_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d+)[pP]").expect("Valid quality pattern")
});

/// Pattern to extract bitrate from URL (e.g., "720P_4000K")
pub static BITRATE_FROM_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d+)[pP]_(\d+)[kK]").expect("Valid bitrate pattern")
});

/// Pattern for ISO 8601 duration (e.g., "PT1H2M3S")
pub static ISO8601_DURATION_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?$").expect("Valid ISO8601 duration pattern")
});

/// Pattern for ISO 8601 date (e.g., "2024-01-15" or "2024-01-15T10:30:00Z")
pub static ISO8601_DATE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(\d{4})-(\d{2})-(\d{2})").expect("Valid ISO8601 date pattern")
});

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
///     .ok_or_else(|| RdlpError::Extraction("Could not extract video ID".into()))?;
/// ```
pub struct BaseExtractor;

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
    /// - `RdlpError::Network` if the request fails or returns non-2xx status
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
    /// let html = Html::parse_document(&webpage);
    /// ```
    pub async fn fetch_webpage(url: &str, ctx: &ExtractionContext) -> Result<String> {
        // Security: Validate URL length
        if url.len() > MAX_URL_LENGTH {
            return Err(RdlpError::Extraction(format!(
                "URL too long: {} bytes (max: {MAX_URL_LENGTH})",
                url.len()
            )));
        }

        let response = ctx
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch webpage: {e}")))?;

        Self::check_http_response(&response)?;

        let webpage = response
            .text()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to read response body: {e}")))?;

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
    pub async fn fetch_webpage_with_headers(
        url: &str,
        headers: &[(&str, &str)],
        ctx: &ExtractionContext,
    ) -> Result<String> {
        // Security: Validate URL length
        if url.len() > MAX_URL_LENGTH {
            return Err(RdlpError::Extraction(format!(
                "URL too long: {} bytes (max: {MAX_URL_LENGTH})",
                url.len()
            )));
        }

        let mut request = ctx.http_client.get(url);

        for (name, value) in headers {
            request = request.header(*name, *value);
        }

        let response = request
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch webpage: {e}")))?;

        Self::check_http_response(&response)?;

        let webpage = response
            .text()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to read response body: {e}")))?;

        if ctx.config.verbose {
            crate::utils::debug_print_webpage_sample(&webpage, DEFAULT_DEBUG_SAMPLE_SIZE);
        }

        Ok(webpage)
    }

    /// Check HTTP response status and return appropriate error
    ///
    /// # Arguments
    /// * `response` - The HTTP response to check
    ///
    /// # Returns
    /// `Ok(())` if status is 2xx, otherwise an appropriate error
    pub fn check_http_response(response: &reqwest::Response) -> Result<()> {
        let status = response.status();

        if status.is_success() {
            return Ok(());
        }

        let error_msg = match status.as_u16() {
            403 => "Access forbidden (403) - may require authentication or cookies",
            404 => "Page not found (404) - video may have been removed",
            429 => "Rate limited (429) - too many requests, try again later",
            451 => "Unavailable for legal reasons (451) - content blocked in your region",
            500..=599 => "Server error - the website may be experiencing issues",
            _ => "Unexpected HTTP status",
        };

        Err(RdlpError::Network(format!(
            "{error_msg}: HTTP {status}"
        )))
    }

    // ========================================================================
    // URL Validation (Security)
    // ========================================================================

    /// Validate a URL for security concerns (SSRF protection)
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
    pub fn validate_url_security(url: &str) -> Result<()> {
        // Length check
        if url.len() > MAX_URL_LENGTH {
            return Err(RdlpError::Extraction(format!(
                "URL too long: {} bytes (max: {MAX_URL_LENGTH})",
                url.len()
            )));
        }

        // Parse URL
        let parsed = url::Url::parse(url)
            .map_err(|e| RdlpError::Extraction(format!("Invalid URL: {e}")))?;

        // Scheme check
        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(RdlpError::Extraction(format!(
                "Invalid URL scheme: {scheme} (expected http or https)"
            )));
        }

        // Host check for private IPs (SSRF protection)
        if let Some(host) = parsed.host_str() {
            if Self::is_private_host(host) {
                return Err(RdlpError::Extraction(format!(
                    "URL points to private/internal host: {host}"
                )));
            }
        }

        Ok(())
    }

    /// Check if a host is a private/internal address
    pub(crate) fn is_private_host(host: &str) -> bool {
        // Check for localhost variants
        if host == "localhost" || host == "127.0.0.1" || host == "::1" {
            return true;
        }

        // Check for common internal hostnames
        if host.ends_with(".local") || host.ends_with(".internal") {
            return true;
        }

        // Try to parse as IP address
        if let Ok(ip) = host.parse::<IpAddr>() {
            return match ip {
                IpAddr::V4(ipv4) => {
                    ipv4.is_loopback()           // 127.0.0.0/8
                        || ipv4.is_private()     // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                        || ipv4.is_link_local()  // 169.254.0.0/16
                        || ipv4.is_unspecified() // 0.0.0.0
                }
                IpAddr::V6(ipv6) => {
                    ipv6.is_loopback()           // ::1
                        || ipv6.is_unspecified() // ::
                }
            };
        }

        false
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
    /// static VIDEO_PATTERN: Lazy<Regex> = Lazy::new(|| {
    ///     Regex::new(r"example\.com/video/(?P<id>\d+)").unwrap()
    /// });
    ///
    /// let id = BaseExtractor::extract_id_from_url(url, &VIDEO_PATTERN, "id");
    /// ```
    pub fn extract_id_from_url(url: &str, pattern: &Regex, group_name: &str) -> Option<String> {
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
    /// static PATTERN: Lazy<Regex> = Lazy::new(|| {
    ///     Regex::new(r"example\.com/(?:video/(\d+)|v(\d+))").unwrap()
    /// });
    ///
    /// let id = BaseExtractor::extract_id_positional(url, &PATTERN, &[1, 2]);
    /// ```
    pub fn extract_id_positional(url: &str, pattern: &Regex, group_indices: &[usize]) -> Option<String> {
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
    // Metadata Extraction
    // ========================================================================

    /// Extract content from a meta tag
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the meta tag
    ///
    /// # Returns
    /// The content attribute value if found and non-empty
    pub fn extract_meta_content(html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .and_then(|elem| elem.value().attr("content"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract href from a link element
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the link element
    ///
    /// # Returns
    /// The href attribute value if found and non-empty
    pub fn extract_link_href(html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .and_then(|elem| elem.value().attr("href"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract text content from an element
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the element
    ///
    /// # Returns
    /// The text content if found and non-empty
    pub fn extract_element_text(html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .map(|elem| elem.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract title using multiple fallback strategies
    ///
    /// Tries in order:
    /// 1. Open Graph title (`og:title`)
    /// 2. Twitter title (`twitter:title`)
    /// 3. HTML title tag
    /// 4. H1 heading
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Title from the first successful strategy, `None` if all fail
    pub fn extract_title_multi_strategy(html: &Html) -> Option<String> {
        // Strategy 1: Open Graph
        if let Some(title) = Self::extract_meta_content(html, &OG_TITLE_SELECTOR) {
            return Some(Self::truncate_string(title, MAX_TITLE_LENGTH));
        }

        // Strategy 2: Twitter
        if let Some(title) = Self::extract_meta_content(html, &TWITTER_TITLE_SELECTOR) {
            return Some(Self::truncate_string(title, MAX_TITLE_LENGTH));
        }

        // Strategy 3: HTML title tag
        if let Some(title) = Self::extract_element_text(html, &TITLE_TAG_SELECTOR) {
            return Some(Self::truncate_string(title, MAX_TITLE_LENGTH));
        }

        // Strategy 4: H1
        Self::extract_element_text(html, &H1_SELECTOR)
            .map(|t| Self::truncate_string(t, MAX_TITLE_LENGTH))
    }

    /// Extract description using multiple fallback strategies
    ///
    /// Tries in order:
    /// 1. Open Graph description
    /// 2. Meta description
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Description from the first successful strategy, `None` if all fail
    pub fn extract_description_multi_strategy(html: &Html) -> Option<String> {
        // Strategy 1: Open Graph
        if let Some(desc) = Self::extract_meta_content(html, &OG_DESCRIPTION_SELECTOR) {
            return Some(Self::truncate_string(desc, MAX_DESCRIPTION_LENGTH));
        }

        // Strategy 2: Meta description
        Self::extract_meta_content(html, &META_DESCRIPTION_SELECTOR)
            .map(|d| Self::truncate_string(d, MAX_DESCRIPTION_LENGTH))
    }

    /// Extract thumbnail URL using multiple fallback strategies
    ///
    /// Tries in order:
    /// 1. Open Graph image
    /// 2. Twitter image
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Thumbnail URL from the first successful strategy, `None` if all fail
    pub fn extract_thumbnail_multi_strategy(html: &Html) -> Option<String> {
        // Strategy 1: Open Graph
        if let Some(thumb) = Self::extract_meta_content(html, &OG_IMAGE_SELECTOR) {
            return Some(thumb);
        }

        // Strategy 2: Twitter
        Self::extract_meta_content(html, &TWITTER_IMAGE_SELECTOR)
    }

    // ========================================================================
    // Size Detection
    // ========================================================================

    /// Detect file size using HEAD request with Range fallback
    ///
    /// This method tries two strategies:
    /// 1. HEAD request to get Content-Length header
    /// 2. Range request (bytes=0-0) to parse Content-Range header
    ///
    /// # Arguments
    /// * `url` - The URL to check
    /// * `ctx` - Extraction context
    ///
    /// # Returns
    /// File size in bytes if detected, `None` otherwise
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// if let Some(size) = BaseExtractor::detect_file_size(&format.url, ctx).await {
    ///     format.filesize = Some(size);
    /// }
    /// ```
    pub async fn detect_file_size(url: &str, ctx: &ExtractionContext) -> Option<u64> {
        // Strategy 1: HEAD request
        if let Ok(response) = ctx.http_client.head(url).send().await {
            if let Some(size) = response.content_length() {
                if size > 0 {
                    if ctx.config.verbose {
                        eprintln!("[BaseExtractor] HEAD Content-Length: {size}");
                    }
                    return Some(size);
                }
            }
        }

        // Strategy 2: Range request fallback
        if let Ok(response) = ctx
            .http_client
            .get(url)
            .header("Range", "bytes=0-0")
            .send()
            .await
        {
            if let Some(content_range) = response.headers().get("content-range") {
                if let Ok(range_str) = content_range.to_str() {
                    // Parse "bytes 0-0/123456"
                    if let Some(total) = range_str.split('/').nth(1) {
                        if let Ok(size) = total.parse::<u64>() {
                            if ctx.config.verbose {
                                eprintln!("[BaseExtractor] Range Content-Range: {size}");
                            }
                            return Some(size);
                        }
                    }
                }
            }
        }

        None
    }

    /// Detect file size using a provided HTTP client
    ///
    /// This variant is useful for parallel detection where you need to pass
    /// the client directly instead of the full context.
    pub async fn detect_file_size_with_client(
        url: &str,
        http_client: &std::sync::Arc<reqwest::Client>,
    ) -> Option<u64> {
        // Strategy 1: HEAD request
        if let Ok(response) = http_client.head(url).send().await {
            if let Some(size) = response.content_length() {
                if size > 0 {
                    return Some(size);
                }
            }
        }

        // Strategy 2: Range request fallback
        if let Ok(response) = http_client
            .get(url)
            .header("Range", "bytes=0-0")
            .send()
            .await
        {
            if let Some(content_range) = response.headers().get("content-range") {
                if let Ok(range_str) = content_range.to_str() {
                    // Parse "bytes 0-0/123456"
                    if let Some(total) = range_str.split('/').nth(1) {
                        if let Ok(size) = total.parse::<u64>() {
                            return Some(size);
                        }
                    }
                }
            }
        }

        None
    }

    /// Detect file size with verbose logging prefix
    ///
    /// Same as `detect_file_size` but with a custom log prefix for clarity.
    pub async fn detect_file_size_verbose(
        url: &str,
        ctx: &ExtractionContext,
        log_prefix: &str,
    ) -> Option<u64> {
        // Strategy 1: HEAD request
        if let Ok(response) = ctx.http_client.head(url).send().await {
            if let Some(size) = response.content_length() {
                if size > 0 {
                    if ctx.config.verbose {
                        eprintln!("[{log_prefix}] HEAD Content-Length: {size}");
                    }
                    return Some(size);
                }
            }
        }

        // Strategy 2: Range request fallback
        if let Ok(response) = ctx
            .http_client
            .get(url)
            .header("Range", "bytes=0-0")
            .send()
            .await
        {
            if let Some(content_range) = response.headers().get("content-range") {
                if let Ok(range_str) = content_range.to_str() {
                    if let Some(total) = range_str.split('/').nth(1) {
                        if let Ok(size) = total.parse::<u64>() {
                            if ctx.config.verbose {
                                eprintln!("[{log_prefix}] Range Content-Range: {size}");
                            }
                            return Some(size);
                        }
                    }
                }
            }
        }

        None
    }

    // ========================================================================
    // Format Building
    // ========================================================================

    /// Calculate width from height assuming 16:9 aspect ratio
    ///
    /// # Arguments
    /// * `height` - Video height in pixels
    ///
    /// # Returns
    /// Calculated width in pixels
    #[inline]
    pub fn width_from_height(height: u32) -> u32 {
        (height * 16) / 9
    }

    /// Parse quality height from a string (e.g., "720p" -> 720)
    ///
    /// # Arguments
    /// * `quality_str` - Quality string like "720p", "1080P", "720"
    ///
    /// # Returns
    /// Parsed height as u32, `None` if parsing fails
    pub fn parse_quality_height(quality_str: &str) -> Option<u32> {
        quality_str
            .trim_end_matches(['p', 'P'])
            .parse::<u32>()
            .ok()
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
    pub fn parse_quality_from_url(url: &str) -> Option<u32> {
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
    pub fn build_format(format_id: String, url: String, ext: String, height: Option<u32>) -> Format {
        let mut format = Format::new(format_id, url, ext.clone(), "https".to_string());

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
                format.vcodec = Some("h264".to_string());
                format.acodec = Some("aac".to_string());
            }
            "webm" => {
                format.vcodec = Some("vp9".to_string());
                format.acodec = Some("opus".to_string());
            }
            _ => {}
        }

        format
    }

    // ========================================================================
    // Logging Utilities
    // ========================================================================

    /// Log a message if verbose mode is enabled
    ///
    /// # Arguments
    /// * `ctx` - Extraction context
    /// * `prefix` - Log prefix (e.g., extractor name)
    /// * `message` - Message to log
    #[inline]
    pub fn log_if_verbose(ctx: &ExtractionContext, prefix: &str, message: &str) {
        if ctx.config.verbose {
            eprintln!("[{prefix}] {message}");
        }
    }

    /// Log content with truncation if verbose mode is enabled
    ///
    /// # Arguments
    /// * `ctx` - Extraction context
    /// * `prefix` - Log prefix
    /// * `label` - Label for the content
    /// * `content` - Content to log (will be truncated)
    /// * `max_length` - Maximum characters to show
    pub fn log_content_if_verbose(
        ctx: &ExtractionContext,
        prefix: &str,
        label: &str,
        content: &str,
        max_length: usize,
    ) {
        if ctx.config.verbose {
            eprintln!("\n=== [{prefix}] {label} ===");
            eprintln!("{}", &content.chars().take(max_length).collect::<String>());
            if content.len() > max_length {
                eprintln!("... (truncated, {} total chars)", content.len());
            }
            eprintln!("=== END ===\n");
        }
    }

    // ========================================================================
    // String Utilities
    // ========================================================================

    /// Truncate a string to a maximum length
    ///
    /// Truncates at character boundaries to avoid breaking Unicode.
    ///
    /// # Arguments
    /// * `s` - String to truncate
    /// * `max_len` - Maximum length in characters
    ///
    /// # Returns
    /// Truncated string (or original if already shorter)
    pub fn truncate_string(s: String, max_len: usize) -> String {
        if s.chars().count() <= max_len {
            s
        } else {
            s.chars().take(max_len).collect()
        }
    }

    /// Sanitize a string for safe logging (redact sensitive data)
    ///
    /// Redacts common sensitive patterns like tokens, keys, passwords.
    ///
    /// # Arguments
    /// * `s` - String to sanitize
    ///
    /// # Returns
    /// Sanitized string with sensitive data redacted
    pub fn sanitize_for_logging(s: &str) -> String {
        // Common patterns to redact
        let patterns = [
            (r"token=[^&\s]+", "token=***"),
            (r"key=[^&\s]+", "key=***"),
            (r"password=[^&\s]+", "password=***"),
            (r"secret=[^&\s]+", "secret=***"),
            (r"api_key=[^&\s]+", "api_key=***"),
        ];

        let mut result = s.to_string();
        for (pattern, replacement) in patterns {
            if let Ok(re) = Regex::new(pattern) {
                result = re.replace_all(&result, replacement).to_string();
            }
        }
        result
    }

    // ========================================================================
    // Date/Time Parsing
    // ========================================================================

    /// Parse ISO 8601 duration to seconds
    ///
    /// Supports formats like:
    /// - PT30S (30 seconds)
    /// - PT5M (5 minutes)
    /// - PT1H (1 hour)
    /// - PT1H30M45S (1 hour, 30 minutes, 45 seconds)
    ///
    /// # Arguments
    /// * `duration_str` - ISO 8601 duration string
    ///
    /// # Returns
    /// Duration in seconds, `None` if parsing fails
    pub fn parse_iso8601_duration(duration_str: &str) -> Option<f64> {
        if !duration_str.starts_with("PT") {
            return None;
        }

        let caps = ISO8601_DURATION_PATTERN.captures(duration_str)?;

        let hours: f64 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
        let minutes: f64 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
        let seconds: f64 = caps.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);

        Some(hours * 3600.0 + minutes * 60.0 + seconds)
    }

    /// Parse ISO 8601 date to YYYYMMDD format
    ///
    /// Supports formats like:
    /// - 2024-01-15
    /// - 2024-01-15T10:30:00Z
    ///
    /// # Arguments
    /// * `date_str` - ISO 8601 date string
    ///
    /// # Returns
    /// Date in YYYYMMDD format, `None` if parsing fails
    pub fn parse_iso8601_date(date_str: &str) -> Option<String> {
        let caps = ISO8601_DATE_PATTERN.captures(date_str)?;

        let year = caps.get(1)?.as_str();
        let month = caps.get(2)?.as_str();
        let day = caps.get(3)?.as_str();

        Some(format!("{year}{month}{day}"))
    }
}
