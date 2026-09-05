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

use ipnet::Ipv4Net;
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
/// - Every IPv4 prefix the IANA Special-Purpose Address Registry marks as not
///   globally reachable — RFC 1918 private space, loopback, link-local, CGNAT,
///   benchmarking, documentation, `240.0.0.0/4` and the rest — plus multicast
/// - IPv6 loopback, unspecified, unique-local, link-local and multicast
/// - The five IPv6 forms whose own prefix identifies them as carrying an IPv4
///   address, judged by that inner address: mapped (`::ffff:0:0/96`), the
///   deprecated compatible form (`::/96`), the NAT64 Well-Known Prefix
///   (`64:ff9b::/96`), 6to4 (`2002::/16`) and Teredo (`2001:0000::/32`)
///
/// # What it does NOT do
///
/// It validates the host **string**, never a resolved address. A name that
/// resolves to a private address (`evil.example.com A 127.0.0.1`) passes,
/// and always will under this design — closing DNS rebinding requires
/// resolve → validate → connect-to-the-validated-IP, which the HTTP stack
/// does not expose. Accepted residual risk; tracked in #662.
///
/// It unwraps only the forms a prefix identifies. ISATAP (RFC 5214) and 6rd
/// (RFC 5969) also embed an IPv4 address but under a site-chosen prefix, as
/// does a NAT64 Network-Specific Prefix — nothing in such an address marks it
/// as carrying an IPv4, so it is indistinguishable from ordinary IPv6 here.
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
            IpAddr::V6(ipv6) => is_private_ipv6(ipv6),
        };
    }

    false
}

/// The registry's `Globally Reachable` column, named so each row says which
/// it is at the point of writing rather than by the polarity of a bare bool.
#[derive(Clone, Copy)]
enum Reach {
    /// Reachable from the public internet — an ordinary fetch target.
    Public,
    /// Not globally reachable: it resolves somewhere inside the network we
    /// are running on, which is the whole SSRF concern.
    Blocked,
}

/// The IANA IPv4 Special-Purpose Address Registry.
///
/// This table *is* the definition of "reserved IPv4" for this crate. Deriving
/// the predicate from the registry rather than from a hand-picked list is what
/// stops it drifting back into an arbitrary subset: the four ranges it
/// replaced were chosen one at a time, and left `0.0.0.0/8` and
/// `240.0.0.0/4` reachable simply because nobody had reached for them.
///
/// Two kinds of registry row are deliberately absent, because neither can
/// change an answer: a `Blocked` row wholly inside another `Blocked` row, and
/// a `Public` row with no `Blocked` parent — an address matching no row is
/// allowed anyway. The `Public` rows that ARE here sit inside a `Blocked`
/// parent, where they must win.
///
/// The rule is asserted rather than merely claimed here: each omitted row is
/// listed in `omitted_registry_rows_resolve_as_the_rule_claims`, which fails
/// if one of them ever stops resolving the way the rule says it does. That
/// test proves the listed rows behave; it cannot prove the list still matches
/// IANA, which stays a hand-maintained transcription.
const IPV4_SPECIAL_PURPOSE: &[(Ipv4Net, Reach)] = &[
    // "This network" (RFC 791 §3.2). 0.x.y.z reaches the local host on
    // several stacks, so the whole /8 goes, not just 0.0.0.0.
    (
        Ipv4Net::new_assert(Ipv4Addr::UNSPECIFIED, 8),
        Reach::Blocked,
    ),
    // Private-Use (RFC 1918).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(10, 0, 0, 0), 8),
        Reach::Blocked,
    ),
    // Shared Address Space / CGNAT (RFC 6598). Routable inside a carrier
    // network, so it reaches real hosts.
    (
        Ipv4Net::new_assert(Ipv4Addr::new(100, 64, 0, 0), 10),
        Reach::Blocked,
    ),
    // Loopback (RFC 1122 §3.2.1.3).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(127, 0, 0, 0), 8),
        Reach::Blocked,
    ),
    // Link Local (RFC 3927) — the cloud metadata endpoint lives here.
    (
        Ipv4Net::new_assert(Ipv4Addr::new(169, 254, 0, 0), 16),
        Reach::Blocked,
    ),
    // Private-Use (RFC 1918).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(172, 16, 0, 0), 12),
        Reach::Blocked,
    ),
    // IETF Protocol Assignments (RFC 6890 §2.1).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 0), 24),
        Reach::Blocked,
    ),
    // Port Control Protocol Anycast (RFC 7723) — globally reachable, and
    // inside the blocked /24 above, so it needs its own row to survive.
    (
        Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 9), 32),
        Reach::Public,
    ),
    // TURN Anycast (RFC 8155) — same carve-out.
    (
        Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 10), 32),
        Reach::Public,
    ),
    // Documentation, TEST-NET-1 (RFC 5737).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 2, 0), 24),
        Reach::Blocked,
    ),
    // 6to4 Relay Anycast, deprecated by RFC 7526 but still routed on some
    // networks. Pairs with `six_to_four_ipv4` — blocking this range while
    // passing `2002::/16` was the gap that motivated the table.
    (
        Ipv4Net::new_assert(Ipv4Addr::new(192, 88, 99, 0), 24),
        Reach::Blocked,
    ),
    // Private-Use (RFC 1918).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(192, 168, 0, 0), 16),
        Reach::Blocked,
    ),
    // Benchmarking (RFC 2544).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(198, 18, 0, 0), 15),
        Reach::Blocked,
    ),
    // Documentation, TEST-NET-2 (RFC 5737).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(198, 51, 100, 0), 24),
        Reach::Blocked,
    ),
    // Documentation, TEST-NET-3 (RFC 5737).
    (
        Ipv4Net::new_assert(Ipv4Addr::new(203, 0, 113, 0), 24),
        Reach::Blocked,
    ),
    // Reserved (RFC 1112 §4), and with it the limited broadcast address.
    (
        Ipv4Net::new_assert(Ipv4Addr::new(240, 0, 0, 0), 4),
        Reach::Blocked,
    ),
];

/// Whether an IPv4 address must not be fetched.
///
/// The single definition of "private IPv4" for this crate. Two call sites need
/// it — the bare-IPv4 host, and an IPv4 unwrapped from an IPv6 form — and they
/// must not be able to disagree.
fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    // Multicast is not in the special-purpose registry — it is its own
    // assignment (RFC 5771) — so it is asked separately.
    ip.is_multicast() || registry_blocks(ip)
}

/// Longest-prefix match against [`IPV4_SPECIAL_PURPOSE`].
///
/// Most specific wins, which is what lets `192.0.0.9/32` stay reachable inside
/// the blocked `192.0.0.0/24`. An address matching no row at all is ordinary
/// public space.
fn registry_blocks(ip: Ipv4Addr) -> bool {
    IPV4_SPECIAL_PURPOSE
        .iter()
        .filter(|(net, _)| net.contains(&ip))
        .max_by_key(|(net, _)| net.prefix_len())
        .is_some_and(|(_, reach)| matches!(reach, Reach::Blocked))
}

/// Whether an IPv6 address must not be fetched.
///
/// Five `std` predicates decide the prefix families; three of them replaced
/// hand-rolled equivalents (`is_unique_local` and `is_unicast_link_local` are
/// stable since 1.84, below this crate's 1.85 MSRV, and `is_multicast` was
/// always available). `is_loopback` and `is_unspecified` were already `std`.
/// The tunnel prefixes reached through `embedded_ipv4s` are still matched by
/// hand, because `std` has no notion of them.
fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()                  // ::1
        || ip.is_unspecified()        // ::
        || ip.is_multicast()          // ff00::/8
        || ip.is_unique_local()       // fc00::/7
        || ip.is_unicast_link_local() // fe80::/10
        || embedded_ipv4s(ip).any(is_private_ipv4)
}

/// Every IPv4 address an IPv6 address carries under a prefix that identifies
/// it as carrying one.
///
/// Each of these forms actually reaches its embedded address, so naming one
/// names that address: `[2002:7f00:1::]` *is* 127.0.0.1. Judging the inner
/// address is what stops all five walking around the IPv4 gate.
///
/// A form whose prefix is site-chosen — ISATAP, 6rd, a NAT64
/// Network-Specific Prefix — also carries an IPv4 and is deliberately not
/// returned here: nothing in such an address distinguishes it from ordinary
/// IPv6. See `is_private_host`'s "What it does NOT do".
///
/// Teredo carries two — the server's and the client's; every other form
/// carries one.
fn embedded_ipv4s(ip: Ipv6Addr) -> impl Iterator<Item = Ipv4Addr> {
    [
        // `::ffff:0:0/96` IPv4-mapped (RFC 4291 §2.5.5.2) and `::/96`
        // IPv4-compatible (§2.5.5.1, deprecated). `to_ipv4_mapped()` sees only
        // the first, which would leave the compatible form invisible;
        // `to_ipv4()` covers both.
        ip.to_ipv4(),
        nat64_well_known_ipv4(ip),
        six_to_four_ipv4(ip),
        teredo_server_ipv4(ip),
        teredo_client_ipv4(ip),
    ]
    .into_iter()
    .flatten()
}

/// The IPv4 address embedded in a `64:ff9b::/96` NAT64 address (RFC 6052 §2.1).
///
/// RFC 6052 §3.1 is what makes rejecting a private inner address correct
/// rather than merely cautious: "The Well-Known Prefix MUST NOT be used to
/// represent non-global IPv4 addresses, such as those defined in [RFC1918]",
/// and translators "MUST drop these packets". A conformant NAT64 address
/// wrapping a public IPv4 stays allowed — refusing those would break NAT64
/// networks outright.
fn nat64_well_known_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    /// A /96 prefix occupies the first six segments. All six must match:
    /// RFC 8215's local-use `64:ff9b:1::/48` differs only in segment 2 and is
    /// explicitly *not* the Well-Known Prefix, so RFC 6052 §3.1's guarantee
    /// does not cover it.
    const WELL_KNOWN_PREFIX: [u16; 6] = [0x0064, 0xff9b, 0, 0, 0, 0];

    let s = ip.segments();
    (s[..6] == WELL_KNOWN_PREFIX).then(|| ipv4_from_segments(s[6], s[7]))
}

/// The IPv4 address embedded in a 6to4 address (`2002::/16`, RFC 3056 §2).
///
/// §2's field table puts V4ADDR in bits 16..=47 — segments 1 and 2. §5.3 is
/// what makes it reachable rather than decorative: a packet to a 6to4 address
/// is encapsulated "with IPv4 destination address = the NLA value V4ADDR
/// extracted from the next hop IPv6 address".
fn six_to_four_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    const PREFIX: u16 = 0x2002;

    let s = ip.segments();
    (s[0] == PREFIX).then(|| ipv4_from_segments(s[1], s[2]))
}

/// The Teredo *server*'s IPv4 address (RFC 4380 §4).
///
/// §4 lays a Teredo address out as prefix | server IPv4 | flags | obfuscated
/// port | obfuscated client IPv4, putting the server at bits 32..=63 —
/// segments 2 and 3, unobfuscated. RFC 5991 randomises bits within the flags
/// field only; the prefix and both embedded addresses keep these positions.
fn teredo_server_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    is_teredo(ip).then(|| ipv4_from_segments(s[2], s[3]))
}

/// The Teredo *client*'s IPv4 address (bits 96..=127), de-obfuscated.
///
/// RFC 4380 §4: "Each bit in the address and port number is reversed; this can
/// be done by an exclusive OR of the ... 32-bit address with the hexadecimal
/// value 0xFFFFFFFF." Per segment, that exclusive OR is a bitwise NOT.
fn teredo_client_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    is_teredo(ip).then(|| ipv4_from_segments(!s[6], !s[7]))
}

/// Whether this is a Teredo address: prefix `2001:0000::/32` (RFC 4380 §2.6).
const fn is_teredo(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    s[0] == 0x2001 && s[1] == 0x0000
}

/// The IPv4 address spelled by two consecutive IPv6 segments, high half first.
///
/// One definition for all four extractors above: assembling the halves by hand
/// at each site is how an octet-order slip gets into one of them and not the
/// others.
fn ipv4_from_segments(high: u16, low: u16) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(high) << 16) | u32::from(low))
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
