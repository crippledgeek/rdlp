//! Security utilities for rdlp
//!
//! This crate provides security-focused utilities including:
//! - SSRF (Server-Side Request Forgery) protection via URL validation
//! - Sensitive data sanitization for safe logging
//! - Private/internal host detection
//!
//! # Features
//!
//! ## SSRF Protection
//!
//! Prevents requests to private/internal networks that could be exploited:
//!
//! ```rust
//! use rdlp_security::validate_url_security;
//!
//! // Public URL - OK
//! assert!(validate_url_security("https://example.com/video.mp4").is_ok());
//!
//! // Private IP - Blocked
//! assert!(validate_url_security("http://192.168.1.1/secret").is_err());
//!
//! // Localhost - Blocked
//! assert!(validate_url_security("http://localhost/admin").is_err());
//! ```
//!
//! ## Safe Logging
//!
//! Redacts sensitive data from strings before logging:
//!
//! ```rust
//! use rdlp_security::sanitize_for_logging;
//!
//! let url = "https://api.example.com?token=secret123&key=abc456";
//! let safe = sanitize_for_logging(url);
//! assert_eq!(safe, "https://api.example.com?token=***&key=***");
//! ```
//!
//! # Architecture
//!
//! This crate is dependency-minimal and can be used standalone or integrated
//! into larger applications. It follows defense-in-depth principles:
//!
//! 1. **URL scheme validation** - Only http/https allowed
//! 2. **Length limits** - Prevents memory exhaustion attacks
//! 3. **Private IP blocking** - Prevents SSRF to internal networks
//! 4. **Pattern-based sanitization** - Redacts common sensitive patterns

#![warn(missing_docs)]

use regex::Regex;
use std::net::IpAddr;
use std::sync::LazyLock;
use thiserror::Error;

// ============================================================================
// Security Constants
// ============================================================================

/// Maximum URL length to prevent memory exhaustion attacks
pub const MAX_URL_LENGTH: usize = 8192;

// ============================================================================
// Error Types
// ============================================================================

/// Security-related errors
#[derive(Debug, Error)]
pub enum SecurityError {
    /// URL exceeds the maximum allowed length
    #[error("URL too long: {0} bytes (max: {MAX_URL_LENGTH})")]
    UrlTooLong(usize),

    /// URL parsing failed
    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    /// URL uses an unsupported scheme (not http/https)
    #[error("Invalid URL scheme: {0} (expected http or https)")]
    InvalidScheme(String),

    /// URL points to a private/internal host (SSRF protection)
    #[error("URL points to private/internal host: {0}")]
    PrivateHost(String),
}

/// Result type for security operations
pub type Result<T> = std::result::Result<T, SecurityError>;

// ============================================================================
// URL Validation (SSRF Protection)
// ============================================================================

/// Validate a URL for security concerns (SSRF protection)
///
/// This function performs multiple security checks:
/// 1. Validates URL length to prevent memory exhaustion
/// 2. Ensures the URL uses http or https scheme only
/// 3. Blocks requests to private/internal IP addresses
///
/// # Arguments
/// * `url` - The URL to validate
///
/// # Returns
/// `Ok(())` if the URL is safe, otherwise a `SecurityError`
///
/// # Examples
///
/// ```rust
/// use rdlp_security::validate_url_security;
///
/// // Valid public URL
/// assert!(validate_url_security("https://example.com/video.mp4").is_ok());
///
/// // Invalid: private IP
/// assert!(validate_url_security("http://192.168.1.1/file").is_err());
///
/// // Invalid: localhost
/// assert!(validate_url_security("http://localhost/admin").is_err());
///
/// // Invalid: non-HTTP scheme
/// assert!(validate_url_security("ftp://example.com/file").is_err());
/// ```
pub fn validate_url_security(url: &str) -> Result<()> {
    // Length check
    if url.len() > MAX_URL_LENGTH {
        return Err(SecurityError::UrlTooLong(url.len()));
    }

    // Parse URL
    let parsed = url::Url::parse(url)?;

    // Scheme check
    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https") {
        return Err(SecurityError::InvalidScheme(scheme.to_string()));
    }

    // Host check for private IPs (SSRF protection)
    if let Some(host) = parsed.host_str() {
        if is_private_host(host) {
            return Err(SecurityError::PrivateHost(host.to_string()));
        }
    }

    Ok(())
}

/// Check if a host is a private/internal address
///
/// This function detects various private/internal address patterns:
/// - Localhost variants: `localhost`, `127.0.0.1`, `::1`
/// - Internal hostnames: `*.local`, `*.internal`
/// - Private IPv4 ranges: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
/// - Loopback and link-local addresses
///
/// # Arguments
/// * `host` - The hostname or IP address to check
///
/// # Returns
/// `true` if the host is private/internal, `false` otherwise
///
/// # Examples
///
/// ```rust
/// use rdlp_security::is_private_host;
///
/// // Private hosts
/// assert!(is_private_host("localhost"));
/// assert!(is_private_host("127.0.0.1"));
/// assert!(is_private_host("192.168.1.1"));
/// assert!(is_private_host("10.0.0.1"));
/// assert!(is_private_host("myhost.local"));
///
/// // Public hosts
/// assert!(!is_private_host("example.com"));
/// assert!(!is_private_host("8.8.8.8"));
/// ```
#[must_use]
pub fn is_private_host(host: &str) -> bool {
    // Check for localhost (IP variants handled by the IP parser below)
    if host == "localhost" {
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

// ============================================================================
// URL Normalization
// ============================================================================

/// Normalize a URL by stripping query parameters and fragments
///
/// This is useful for comparing URLs that may have dynamic tokens,
/// session IDs, or other ephemeral query parameters. The comparison
/// should focus on the essential parts: scheme, host, port, and path.
///
/// # Arguments
/// * `url` - The URL to normalize
///
/// # Returns
/// The normalized URL string (scheme + host + port + path)
///
/// # Examples
///
/// ```rust
/// use rdlp_security::normalize_url;
///
/// // Strips query parameters
/// assert_eq!(
///     normalize_url("https://cdn.example.com/video.m3u8?token=abc123"),
///     "https://cdn.example.com/video.m3u8"
/// );
///
/// // Different tokens normalize to same URL
/// let url1 = "https://cdn.example.com/video.m3u8?token=abc";
/// let url2 = "https://cdn.example.com/video.m3u8?token=xyz";
/// assert_eq!(normalize_url(url1), normalize_url(url2));
///
/// // Preserves port if present
/// assert_eq!(
///     normalize_url("https://cdn.example.com:8080/video.m3u8?key=123"),
///     "https://cdn.example.com:8080/video.m3u8"
/// );
///
/// // Strips fragments
/// assert_eq!(
///     normalize_url("https://example.com/video.m3u8#section"),
///     "https://example.com/video.m3u8"
/// );
/// ```
#[must_use]
pub fn normalize_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            // Strip query and fragment, keep everything else
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.into()
        }
        Err(_) => {
            // If URL parsing fails, fall back to simple query string stripping
            url.split('?').next().unwrap_or(url).to_string()
        }
    }
}

/// Extract the path portion of a URL for CDN-agnostic comparison
///
/// CDNs often use different edge server hostnames for the same content
/// (e.g., `ev-h-ph.rdtcdn.com` vs `ev-a-ph.rdtcdn.com`). This function
/// extracts only the path, which uniquely identifies the content.
///
/// # Arguments
/// * `url` - The URL to extract the path from
///
/// # Returns
/// The path portion of the URL (e.g., `/hls/videos/123/master.m3u8`)
///
/// # Examples
///
/// ```rust
/// use rdlp_security::extract_url_path;
///
/// // Different CDN hostnames, same path
/// let url1 = "https://ev-h-ph.cdn.com/videos/123/master.m3u8?token=abc";
/// let url2 = "https://ev-a-ph.cdn.com/videos/123/master.m3u8?token=xyz";
/// assert_eq!(extract_url_path(url1), extract_url_path(url2));
///
/// // Extracts just the path
/// assert_eq!(
///     extract_url_path("https://cdn.example.com/hls/video.m3u8?key=123"),
///     "/hls/video.m3u8"
/// );
/// ```
#[must_use]
pub fn extract_url_path(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => parsed.path().to_string(),
        Err(_) => {
            // Fallback: find path after scheme://host
            url.split('?')
                .next()
                .and_then(|s| s.find("://").map(|i| &s[i + 3..]))
                .and_then(|s| s.find('/').map(|i| &s[i..]))
                .unwrap_or(url)
                .to_string()
        }
    }
}

// ============================================================================
// Sanitization for Safe Logging
// ============================================================================

/// Pre-compiled regex patterns for sensitive parameter redaction.
static SANITIZE_PATTERNS: LazyLock<[(Regex, &str); 5]> = LazyLock::new(|| {
    [
        (
            Regex::new(r"token=[^&\s]+").expect("valid regex"),
            "token=***",
        ),
        (Regex::new(r"key=[^&\s]+").expect("valid regex"), "key=***"),
        (
            Regex::new(r"password=[^&\s]+").expect("valid regex"),
            "password=***",
        ),
        (
            Regex::new(r"secret=[^&\s]+").expect("valid regex"),
            "secret=***",
        ),
        (
            Regex::new(r"api_key=[^&\s]+").expect("valid regex"),
            "api_key=***",
        ),
    ]
});

/// Sanitize a string for safe logging by redacting sensitive data
///
/// This function uses pattern matching to redact common sensitive parameters:
/// - `token=...` -> `token=***`
/// - `key=...` -> `key=***`
/// - `password=...` -> `password=***`
/// - `secret=...` -> `secret=***`
/// - `api_key=...` -> `api_key=***`
///
/// # Arguments
/// * `s` - The string to sanitize
///
/// # Returns
/// A sanitized string with sensitive data replaced by `***`
///
/// # Examples
///
/// ```rust
/// use rdlp_security::sanitize_for_logging;
///
/// let url = "https://api.example.com?token=secret123&other=value";
/// let safe = sanitize_for_logging(url);
/// assert_eq!(safe, "https://api.example.com?token=***&other=value");
///
/// let multi = "url?key=abc&password=xyz";
/// let safe = sanitize_for_logging(multi);
/// assert_eq!(safe, "url?key=***&password=***");
/// ```
#[must_use]
pub fn sanitize_for_logging(s: &str) -> String {
    let mut result = s.to_string();
    for (re, replacement) in SANITIZE_PATTERNS.iter() {
        result = re.replace_all(&result, *replacement).to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // URL Security Tests
    // ========================================================================

    #[test]
    fn test_validate_url_security_valid() {
        assert!(validate_url_security("https://example.com/video.mp4").is_ok());
        assert!(validate_url_security("http://cdn.example.com/file").is_ok());
    }

    #[test]
    fn test_validate_url_security_invalid_scheme() {
        assert!(validate_url_security("ftp://example.com/file").is_err());
        assert!(validate_url_security("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_security_private_ip() {
        assert!(validate_url_security("http://localhost/file").is_err());
        assert!(validate_url_security("http://127.0.0.1/file").is_err());
        assert!(validate_url_security("http://192.168.1.1/file").is_err());
        assert!(validate_url_security("http://10.0.0.1/file").is_err());
    }

    #[test]
    fn test_validate_url_security_length_limit() {
        let long_url = format!("https://example.com/{}", "a".repeat(MAX_URL_LENGTH));
        assert!(validate_url_security(&long_url).is_err());
    }

    #[test]
    fn test_is_private_host() {
        // Private hosts
        assert!(is_private_host("localhost"));
        assert!(is_private_host("127.0.0.1"));
        assert!(is_private_host("192.168.1.1"));
        assert!(is_private_host("10.0.0.1"));
        assert!(is_private_host("172.16.0.1"));
        assert!(is_private_host("::1"));
        assert!(is_private_host("myhost.local"));
        assert!(is_private_host("server.internal"));

        // Public hosts
        assert!(!is_private_host("example.com"));
        assert!(!is_private_host("8.8.8.8"));
        assert!(!is_private_host("cdn.example.com"));
    }

    // ========================================================================
    // Sanitization Tests
    // ========================================================================

    #[test]
    fn test_sanitize_for_logging() {
        assert_eq!(
            sanitize_for_logging("url?token=secret123&other=value"),
            "url?token=***&other=value"
        );
        assert_eq!(
            sanitize_for_logging("url?key=abc&password=xyz"),
            "url?key=***&password=***"
        );
    }

    #[test]
    fn test_sanitize_multiple_patterns() {
        let input = "api?token=tok123&api_key=key456&secret=sec789&normal=value";
        let output = sanitize_for_logging(input);
        assert!(output.contains("token=***"));
        assert!(output.contains("api_key=***"));
        assert!(output.contains("secret=***"));
        assert!(output.contains("normal=value"));
    }

    #[test]
    fn test_sanitize_preserves_non_sensitive_data() {
        let input = "https://example.com/video?id=12345&quality=720p";
        let output = sanitize_for_logging(input);
        assert_eq!(output, input); // No changes expected
    }
}
