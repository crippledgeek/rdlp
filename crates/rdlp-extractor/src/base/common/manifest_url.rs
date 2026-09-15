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
/// scheme still goes through the real validator. The bypass is gated by
/// `cfg(test)` OR the `loopback-test-exemption` cargo feature (below), so an
/// ordinary production build — which has neither `cfg(test)` nor that feature
/// enabled — compiles with no loopback exemption at all.
///
/// The `loopback-test-exemption` feature widens the same bypass to non-test
/// builds of THIS crate, so a sibling crate's own test suite (`rdlp-plugin`,
/// `rdlp-api`) can drive this expander against mockito without depending on
/// rdlp-extractor's `#[cfg(test)]` code. It is intended to be enabled ONLY as
/// a dev-dependency feature in those sibling crates, added when their own
/// tests need it — both `rdlp-plugin`'s `[dev-dependencies]` (for its
/// `expand-hls`/`probe-format-sizes` host-import tests) and `rdlp-api`'s
/// `[dev-dependencies]` (for its own mockito-backed extraction tests) enable
/// it today. It must never be enabled by a production binary —
/// `scripts/check-test-only-features-not-in-release.sh` proves that.
///
/// Returns `rdlp_security`'s own error so each protocol can map it into its
/// own error type without this gate having to know about any of them.
pub(crate) fn validate_manifest_sourced_url(url: &str) -> rdlp_security::Result<()> {
    #[cfg(any(test, feature = "loopback-test-exemption"))]
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
/// `cfg(test)`-only (plus the `loopback-test-exemption` feature — see
/// [`validate_manifest_sourced_url`]): production builds carry no loopback
/// concept at all.
#[cfg(any(test, feature = "loopback-test-exemption"))]
pub(crate) fn is_loopback_origin(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed
            .host_str()
            .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]" || h == "::1")
}

/// The numeric id from a loopback URL's PATH, captured by `path_pattern`'s
/// named `id` group.
///
/// The shared shape behind every extractor whose production URL pattern is
/// host-anchored (PornoXO, PornOne, …): a mockito loopback URL never matches
/// that pattern, so `parse_video_id` needs a second, test-only route to drive
/// `extract()` end to end. Extracted from two near-identical copies
/// (`extractors/pornoxo/patterns.rs`, `extractors/pornone/patterns.rs`) so a
/// third site does not become a third copy.
///
/// Shares [`is_loopback_origin`]'s definition of loopback so this and the
/// SSRF gate's seam cannot disagree about which origins qualify. Stays a
/// separate function on purpose: this is an id-parsing convenience, not a
/// security boundary, and the two must be free to change independently.
///
/// Lives in this module for the same reason: it exists purely to be
/// co-located with [`is_loopback_origin`], the one piece of knowledge it
/// actually shares. It is test-only ROUTING support, not a security gate
/// itself — `manifest_url.rs` is where the shared predicate lives, not a
/// statement that this function belongs to the SSRF surface.
///
/// `cfg(test)`-only: production builds carry no loopback concept at all.
#[cfg(test)]
pub(crate) fn loopback_path_id(url: &str, path_pattern: &regex::Regex) -> Option<String> {
    if !is_loopback_origin(url) {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    path_pattern
        .captures(parsed.path())
        .and_then(|c| c.name("id"))
        .map(|m| m.as_str().to_owned())
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

    /// `loopback_path_id` itself, independent of either caller: loopback
    /// scoping and the named-group extraction both pinned here so a caller's
    /// own tests only need to prove it is WIRED, not that it works.
    #[test]
    fn loopback_path_id_extracts_the_named_group_on_loopback_only() {
        let pattern = regex::Regex::new(r"\A/videos/(?P<id>\d+)/[^/?#]+/?\z").expect("valid");
        assert_eq!(
            loopback_path_id("http://127.0.0.1:1234/videos/42/x/", &pattern).as_deref(),
            Some("42")
        );
        assert_eq!(
            loopback_path_id("https://evil.test/videos/42/x/", &pattern),
            None,
            "a non-loopback host must not take the fallback route"
        );
        assert_eq!(
            loopback_path_id("http://127.0.0.1:1234/nope/", &pattern),
            None,
            "loopback but the path doesn't match the caller's pattern"
        );
    }
}
