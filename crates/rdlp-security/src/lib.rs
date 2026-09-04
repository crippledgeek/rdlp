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
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use thiserror::Error;

pub mod text;

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

    /// Proxy URL uses an unsupported scheme
    #[error("Invalid proxy scheme: {0} (expected http, https, socks5, or socks5h)")]
    InvalidProxyScheme(String),

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
/// # Errors
///
/// Returns [`SecurityError`] if the URL is too long, uses a non-HTTP scheme, or
/// targets a private/internal host.
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
    if let Some(host) = parsed.host_str()
        && is_private_host(host)
    {
        return Err(SecurityError::PrivateHost(host.to_string()));
    }

    Ok(())
}

/// Check if a host is a private/internal address
///
/// This function detects various private/internal address patterns:
/// - Localhost variants: `localhost`, `127.0.0.1`, `::1`
/// - Internal hostnames: `*.local`, `*.internal`
/// - Private IPv4 ranges: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
/// - Loopback, link-local, unspecified, broadcast and multicast addresses
/// - Reserved IPv4 ranges `std` has no stable predicate for: `100.64.0.0/10`
///   (CGNAT), `192.0.0.0/24` (IETF), `192.88.99.0/24` (6to4 relay),
///   `198.18.0.0/15` (benchmarking)
/// - IPv6 forms wrapping a private IPv4: mapped (`::ffff:`), the deprecated
///   compatible form (`::`), and the NAT64 Well-Known Prefix (`64:ff9b::`)
///
/// # What it does NOT do
///
/// It validates the host **string**, never a resolved address. A name that
/// resolves to a private address (`evil.example.com A 127.0.0.1`) passes,
/// and always will under this design — closing DNS rebinding requires
/// resolve → validate → connect-to-the-validated-IP, which the HTTP stack
/// does not expose. Accepted residual risk; see #662, #663.
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
    // Strip an optional trailing dot (FQDN form) and bracketed-IPv6 wrapper
    // before any other check. RFC 2606 / RFC 1535 allow `localhost.` and
    // `[::1]` as valid host renderings.
    let host = host.trim_end_matches('.');
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);

    // Check for localhost in lowercase + common variants.
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower == "ip6-localhost"
        || lower == "ip6-loopback"
        || lower.starts_with("localhost.")
    {
        return true;
    }

    // Check for common internal hostnames (`lower` is already ASCII-lowercased above).
    #[allow(clippy::case_sensitive_file_extension_comparisons)] // `lower` is already lowercased
    if lower.ends_with(".local") || lower.ends_with(".internal") {
        return true;
    }

    // Try to parse as IP address
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ipv4) => is_private_ipv4(ipv4),
            IpAddr::V6(ipv6) => {
                ipv6.is_loopback()           // ::1
                    || ipv6.is_unspecified() // ::
                    || is_ipv6_link_local(&ipv6) // fe80::/10
                    || is_ipv6_unique_local(&ipv6) // fc00::/7
                    || is_ipv6_multicast(&ipv6)    // ff00::/8
                    || is_ipv6_wrapping_private_v4(&ipv6) // ::ffff:/ ::/ 64:ff9b::
            }
        };
    }

    false
}

/// Whether an IPv4 address must not be fetched.
///
/// The single definition of "private IPv4" for this crate. Three call sites
/// need it — the bare-IPv4 host, an IPv4 wrapped in an IPv6 form, and the
/// NAT64-embedded address — and they must not be able to disagree.
const fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()           // 127.0.0.0/8
        || ip.is_private()     // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
        || ip.is_link_local()  // 169.254.0.0/16
        || ip.is_unspecified() // 0.0.0.0
        || ip.is_broadcast()   // 255.255.255.255
        || ip.is_multicast()   // 224.0.0.0/4
        || is_reserved_ipv4(ip)
}

/// Reserved IPv4 ranges that reach internal infrastructure but that `std`
/// does not expose a predicate for on stable.
///
/// Matched on octets rather than by calling `std`: `Ipv4Addr::is_shared` and
/// `Ipv4Addr::is_benchmarking` are unstable behind feature `ip`
/// (rust-lang/rust#27709, verified against rustc 1.97.0), and no
/// `is_ietf_protocol_assignment` exists on stable at all.
const fn is_reserved_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();

    // 100.64.0.0/10 — CGNAT shared address space (RFC 6598). Routable inside
    // a carrier network, so it reaches real hosts.
    (a == 100 && matches!(b, 64..=127))
        // 192.0.0.0/24 — IETF protocol assignments (RFC 6890 §2.2.2).
        || (a == 192 && b == 0 && c == 0)
        // 192.88.99.0/24 — 6to4 relay anycast, deprecated by RFC 7526 but
        // still routed on some networks.
        || (a == 192 && b == 88 && c == 99)
        // 198.18.0.0/15 — benchmarking (RFC 2544 §C.2.2).
        || (a == 198 && matches!(b, 18 | 19))
}

/// IPv6 link-local: `fe80::/10`. First 10 bits == `1111111010`.
#[inline]
const fn is_ipv6_link_local(ip: &std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// IPv6 Unique Local Address: `fc00::/7`. First 7 bits == `1111110`.
#[inline]
const fn is_ipv6_unique_local(ip: &std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

/// IPv6 multicast: `ff00::/8`.
#[inline]
const fn is_ipv6_multicast(ip: &std::net::Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xff00) == 0xff00
}

/// An IPv6 address that carries an IPv4 address inside it, rejected when
/// that inner address is one we would refuse on its own — otherwise an
/// attacker writes `[::ffff:127.0.0.1]` and walks around the IPv4 gate.
///
/// Three forms carry one, and each needs its own extraction:
///
/// - `::ffff:0:0/96` IPv4-**mapped** (RFC 4291 §2.5.5.2)
/// - `::/96` IPv4-**compatible** (RFC 4291 §2.5.5.1, deprecated). Distinct
///   from the mapped form: `to_ipv4_mapped()` returns `None` for it, so it
///   is invisible to a mapped-only check. `to_ipv4()` covers both.
/// - `64:ff9b::/96` the NAT64 Well-Known Prefix (RFC 6052 §2.1)
#[inline]
fn is_ipv6_wrapping_private_v4(ip: &Ipv6Addr) -> bool {
    ip.to_ipv4()
        .or_else(|| nat64_embedded_ipv4(ip))
        .is_some_and(is_private_ipv4)
}

/// The IPv4 address embedded in a `64:ff9b::/96` NAT64 address, if this is
/// one.
///
/// Only the Well-Known Prefix is recognised; a Network-Specific Prefix is
/// site-chosen and cannot be identified from the address alone.
///
/// RFC 6052 §3.1 is what makes rejecting a private inner address correct
/// rather than merely cautious: "The Well-Known Prefix MUST NOT be used to
/// represent non-global IPv4 addresses, such as those defined in [RFC1918]",
/// and translators "MUST drop these packets". A conformant NAT64 address
/// wrapping a public IPv4 stays allowed — refusing those would break NAT64
/// networks outright.
#[inline]
fn nat64_embedded_ipv4(ip: &Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    // §2.2: for a /96 prefix the IPv4 address occupies bits 96..=127, i.e.
    // the final two segments, with the first six forming the prefix.
    let is_well_known =
        segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0, 0, 0, 0];
    is_well_known.then(|| {
        let embedded = (u32::from(segments[6]) << 16) | u32::from(segments[7]);
        Ipv4Addr::from(embedded)
    })
}

/// Validate a proxy URL for security concerns.
///
/// Allows only `http`, `https`, `socks5`, and `socks5h` schemes and
/// blocks proxy targets that point to private/internal hosts (SSRF).
///
/// # Arguments
///
/// * `proxy` - The proxy URL string to validate.
///
/// # Errors
///
/// Returns [`SecurityError`] if the URL is too long, uses a disallowed scheme, or
/// targets a private/internal host.
///
/// # Returns
///
/// `Ok(())` if the proxy URL is safe, otherwise a [`SecurityError`].
///
/// # Examples
///
/// ```rust
/// use rdlp_security::validate_proxy_url;
///
/// assert!(validate_proxy_url("http://proxy.example.com:3128").is_ok());
/// assert!(validate_proxy_url("socks5://proxy.example.com:1080").is_ok());
/// assert!(validate_proxy_url("http://192.168.1.1:3128").is_err());
/// assert!(validate_proxy_url("ftp://proxy.example.com").is_err());
/// ```
pub fn validate_proxy_url(proxy: &str) -> Result<()> {
    if proxy.len() > MAX_URL_LENGTH {
        return Err(SecurityError::UrlTooLong(proxy.len()));
    }

    let parsed = url::Url::parse(proxy)?;

    let scheme = parsed.scheme();
    if !matches!(scheme, "http" | "https" | "socks5" | "socks5h") {
        return Err(SecurityError::InvalidProxyScheme(scheme.to_string()));
    }

    // Block proxies pointing to private/internal hosts.
    if let Some(host) = parsed.host_str()
        && is_private_host(host)
    {
        return Err(SecurityError::PrivateHost(host.to_string()));
    }

    Ok(())
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
    url::Url::parse(url).map_or_else(
        |_| {
            // If URL parsing fails, fall back to simple query string stripping
            url.split('?').next().unwrap_or(url).to_string()
        },
        |mut parsed| {
            // Strip query and fragment, keep everything else
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.into()
        },
    )
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
    url::Url::parse(url).map_or_else(
        |_| {
            // Fallback: find path after scheme://host
            url.split('?')
                .next()
                .and_then(|s| s.find("://").map(|i| &s[i + 3..]))
                .and_then(|s| s.find('/').map(|i| &s[i..]))
                .unwrap_or(url)
                .to_string()
        },
        |parsed| parsed.path().to_string(),
    )
}

// ============================================================================
// Sanitization for Safe Logging
// ============================================================================

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
    rdlp_redact::redact_str(s)
}

#[cfg(test)]
mod tests;
