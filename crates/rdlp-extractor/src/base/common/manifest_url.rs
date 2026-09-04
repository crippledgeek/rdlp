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
    if is_loopback_origin(url) {
        return Ok(());
    }
    rdlp_security::validate_url_security(url)
}

/// Whether `url` is an HTTP(S) URL on a loopback host.
///
/// The single definition of "loopback origin" for every `cfg(test)` seam that
/// needs one — this gate's mockito exemption and the PornoXO id-parsing seam in
/// `extractors/pornoxo/patterns.rs`. Those two remain separate FUNCTIONS
/// deliberately (a security gate and an id parser must be free to change
/// independently), but they must not hold separate OPINIONS about which
/// origins are loopback: the day someone adds `0.0.0.0` or a
/// bracket-normalisation fix to one copy, the test seam and the security gate
/// start disagreeing.
///
/// The scheme is part of the judgement on purpose. A loopback host reached
/// over `file://` is not a loopback *origin*, so no caller can inherit the
/// exemption by forgetting its own scheme check.
///
/// `cfg(test)`-only: production builds carry no loopback concept at all.
#[cfg(test)]
pub(crate) fn is_loopback_origin(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed
            .host_str()
            .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]" || h == "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Which origins are loopback" is ONE piece of knowledge with two real
    /// uses (this gate and the PornoXO id-parsing test seam). The two gates
    /// stay separate — a security gate and an id parser should change
    /// independently — but the definition is shared, so they cannot disagree
    /// about what loopback means.
    #[test]
    fn loopback_origins_are_recognised() {
        assert!(is_loopback_origin("http://127.0.0.1:1234/v.m3u8"));
        assert!(is_loopback_origin("https://127.0.0.1/v.m3u8"));
        assert!(is_loopback_origin("http://localhost:1234/v.m3u8"));
        // `Url::host_str` returns IPv6 hosts in bracketed form.
        assert!(is_loopback_origin("http://[::1]:1234/v.m3u8"));
    }

    #[test]
    fn non_loopback_origins_are_rejected() {
        assert!(!is_loopback_origin("https://cdn.example.com/v.m3u8"));
        assert!(!is_loopback_origin(
            "http://169.254.169.254/latest/meta-data/"
        ));
        assert!(!is_loopback_origin("http://10.0.0.1/v.m3u8"));
        // Lookalikes that merely contain a loopback token.
        assert!(!is_loopback_origin("http://127.0.0.1.evil.test/v.m3u8"));
        assert!(!is_loopback_origin("http://localhost.evil.test/v.m3u8"));
    }

    /// The predicate is about the ORIGIN, so a loopback host reached over a
    /// non-HTTP scheme is not one — otherwise `file://` would inherit the
    /// exemption the moment a caller forgot its own scheme check.
    #[test]
    fn non_http_schemes_are_not_loopback_origins() {
        assert!(!is_loopback_origin("file://localhost/etc/passwd"));
        assert!(!is_loopback_origin("ftp://127.0.0.1/x"));
        assert!(!is_loopback_origin("not a url at all"));
    }

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
