//! The single SSRF gate for URLs that came out of manifest or page content.
//!
//! HLS and DASH both resolve URLs they did not choose — variant and segment
//! URIs out of a playlist body, `BaseURL`/`SegmentTemplate` targets out of an
//! MPD, a master URL lifted from page JavaScript. They are the same trust
//! class and must be gated the same way, so the gate lives here once rather
//! than once per protocol.

/// Validate a URL that originated in attacker-influenceable content.
///
/// Production behavior: delegates to `rdlp_security::validate_url_security`,
/// which rejects `file://`, `javascript:`, RFC 1918 private hosts, link-local
/// `169.254.0.0/16` (including cloud-metadata IPs), and other SSRF-prone
/// targets.
///
/// Test behavior: allows `http`/`https` on `127.0.0.1` / `localhost` / `[::1]`
/// so mockito-driven unit tests can drive expansion against loopback fixtures.
/// Every other host — including all other private ranges — and every non-HTTP
/// scheme still goes through the real validator. The bypass is `cfg(test)`-
/// gated, so production builds compile with no loopback exemption at all.
///
/// Returns `rdlp_security`'s own error so each protocol can map it into its
/// own error type without this gate having to know about any of them.
pub(crate) fn validate_manifest_sourced_url(url: &str) -> rdlp_security::Result<()> {
    #[cfg(test)]
    {
        if let Ok(parsed) = url::Url::parse(url) {
            let scheme_ok = matches!(parsed.scheme(), "http" | "https");
            let host_loopback = parsed.host_str().is_some_and(|h| {
                h == "127.0.0.1" || h == "localhost" || h == "[::1]" || h == "::1"
            });
            if scheme_ok && host_loopback {
                return Ok(());
            }
        }
    }
    rdlp_security::validate_url_security(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_allowed_for_mockito() {
        assert!(validate_manifest_sourced_url("http://127.0.0.1:1234/v.m3u8").is_ok());
        assert!(validate_manifest_sourced_url("http://localhost:1234/v.m3u8").is_ok());
    }

    /// The exemption is loopback-only: it must not become a general hole for
    /// private address space, which is the whole point of the gate.
    #[test]
    fn other_private_and_link_local_hosts_are_still_rejected() {
        assert!(validate_manifest_sourced_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_manifest_sourced_url("http://10.0.0.1/v.m3u8").is_err());
        assert!(validate_manifest_sourced_url("http://192.168.1.1/v.m3u8").is_err());
    }

    /// Non-HTTP schemes get no exemption even on loopback.
    #[test]
    fn non_http_schemes_are_rejected_on_loopback_too() {
        assert!(validate_manifest_sourced_url("file:///etc/passwd").is_err());
        assert!(validate_manifest_sourced_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn ordinary_public_urls_pass() {
        assert!(validate_manifest_sourced_url("https://cdn.example.com/v.m3u8").is_ok());
    }
}
