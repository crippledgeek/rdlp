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

// ========================================================================
// Proxy Validation Tests
// ========================================================================

#[test]
fn test_validate_proxy_url_http_ok() {
    assert!(validate_proxy_url("http://proxy.example.com:3128").is_ok());
}

#[test]
fn test_validate_proxy_url_socks5_ok() {
    assert!(validate_proxy_url("socks5://proxy.example.com:1080").is_ok());
    assert!(validate_proxy_url("socks5h://proxy.example.com:1080").is_ok());
}

#[test]
fn test_validate_proxy_url_rejects_ftp() {
    assert!(validate_proxy_url("ftp://proxy.example.com").is_err());
}

#[test]
fn test_validate_proxy_url_rejects_private_host() {
    assert!(validate_proxy_url("http://192.168.1.1:3128").is_err());
    assert!(validate_proxy_url("socks5://localhost:1080").is_err());
}

#[test]
fn test_sanitize_strips_user_pass_from_proxy() {
    let input = "http://user:password@proxy.example.com:3128";
    let safe = sanitize_for_logging(input);
    assert!(safe.contains("*:*@"), "should redact credentials");
    assert!(!safe.contains("password"), "should not leak password");
}

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

// ── Hardened IPv6 private-host coverage ─────────────────────────

#[test]
fn ipv6_link_local_is_private() {
    for ip in ["fe80::1", "fe80::abcd", "FE80::1"] {
        assert!(is_private_host(ip), "{ip} must be flagged private");
    }
}

#[test]
fn ipv6_unique_local_is_private() {
    for ip in ["fc00::1", "fd00::1", "fdee:abcd::1"] {
        assert!(is_private_host(ip), "{ip} must be flagged private");
    }
}

#[test]
fn ipv6_multicast_is_private() {
    for ip in ["ff00::1", "ff02::1"] {
        assert!(is_private_host(ip), "{ip} must be flagged private");
    }
}

#[test]
fn ipv6_v4_mapped_loopback_is_private() {
    // ::ffff:127.0.0.1 — IPv4-mapped form must not bypass the v4 gate.
    assert!(is_private_host("::ffff:127.0.0.1"));
    assert!(is_private_host("::ffff:10.0.0.1"));
    assert!(is_private_host("::ffff:192.168.1.1"));
}

#[test]
fn ipv6_global_unicast_is_public() {
    for ip in ["2001:4860:4860::8888", "2606:4700:4700::1111"] {
        assert!(
            !is_private_host(ip),
            "{ip} must NOT be flagged private (public)"
        );
    }
}

#[test]
fn localhost_variants_are_private() {
    for h in [
        "localhost",
        "LocalHost",
        "localhost.",
        "localhost.localdomain",
        "ip6-localhost",
        "ip6-loopback",
    ] {
        assert!(is_private_host(h), "{h} must be private");
    }
}

#[test]
fn ipv4_broadcast_and_multicast_are_private() {
    assert!(is_private_host("255.255.255.255"));
    assert!(is_private_host("239.255.255.250")); // SSDP multicast
}

#[test]
fn validate_url_security_blocks_ipv6_link_local() {
    // Bracketed form, both upper- and lower-cased.
    assert!(validate_url_security("http://[fe80::1]/x").is_err());
    assert!(validate_url_security("https://[FE80::abcd]/").is_err());
    assert!(validate_url_security("https://[fc00::1]/x").is_err());
    assert!(validate_url_security("http://[::ffff:127.0.0.1]/").is_err());
}

#[test]
fn sanitize_extends_to_cdn_signing() {
    for input in [
        "url?sig=abcd1234",
        "url?HMAC=abcd",
        "url?access_token=tok",
        "url?bearer=tok",
        "url?X-Amz-Signature=abc&X-Amz-Credential=def",
    ] {
        let out = sanitize_for_logging(input);
        assert!(
            !out.contains("abcd1234")
                && !out.contains("=abcd")
                && !out.contains("=tok")
                && !out.contains("=abc")
                && !out.contains("=def"),
            "expected redaction but got {out:?}"
        );
    }
}

#[test]
fn sanitize_case_insensitive_param_names() {
    // Parameter name case-insensitive (CDN URLs sometimes use TitleCase).
    let out = sanitize_for_logging("url?TOKEN=hideme");
    assert!(!out.contains("hideme"), "expected redaction, got: {out}");
}

// ── L1 regression guard: auth=, authorization=, session= must be redacted ─

/// Before L1 was fixed, `auth=`, `authorization=`, and `session=` query
/// parameters were not covered by `sanitize_for_logging`, so their values
/// leaked verbatim into log output.
#[test]
fn sanitize_redacts_auth_param() {
    let input = "https://api.example.com/resource?auth=secret_auth_token&other=value";
    let output = sanitize_for_logging(input);
    assert!(
        output.contains("auth=***"),
        "auth= was not redacted; got: {output}"
    );
    assert!(
        !output.contains("secret_auth_token"),
        "auth value leaked into output: {output}"
    );
    assert!(
        output.contains("other=value"),
        "unrelated param was incorrectly altered: {output}"
    );
}

#[test]
fn sanitize_redacts_authorization_param() {
    let input = "https://api.example.com/resource?authorization=Bearer%20xyz789&q=1";
    let output = sanitize_for_logging(input);
    assert!(
        output.contains("authorization=***"),
        "authorization= was not redacted; got: {output}"
    );
    assert!(
        !output.contains("xyz789"),
        "authorization value leaked into output: {output}"
    );
}

#[test]
fn sanitize_redacts_session_param() {
    let input = "https://cdn.example.com/video.m3u8?session=sess_abc123&quality=720p";
    let output = sanitize_for_logging(input);
    assert!(
        output.contains("session=***"),
        "session= was not redacted; got: {output}"
    );
    assert!(
        !output.contains("sess_abc123"),
        "session value leaked into output: {output}"
    );
    assert!(
        output.contains("quality=720p"),
        "unrelated param was incorrectly altered: {output}"
    );
}

#[test]
fn sanitize_redacts_new_patterns_case_insensitive() {
    // Verify case-insensitive matching for all three new patterns.
    let auth_upper = sanitize_for_logging("url?AUTH=val1");
    assert!(
        !auth_upper.contains("val1"),
        "AUTH= (uppercase) not redacted"
    );

    let authz_mixed = sanitize_for_logging("url?Authorization=val2");
    assert!(
        !authz_mixed.contains("val2"),
        "Authorization= (mixed case) not redacted"
    );

    let sess_upper = sanitize_for_logging("url?SESSION=val3");
    assert!(
        !sess_upper.contains("val3"),
        "SESSION= (uppercase) not redacted"
    );
}

#[test]
fn sanitize_for_logging_redacts_new_328_patterns_via_delegate() {
    assert_eq!(
        sanitize_for_logging("https://cdn/s?X-Amz-Security-Token=STS&code=AUTH"),
        "https://cdn/s?X-Amz-Security-Token=***&code=***"
    );
}

// ── Reserved IPv4 ranges the predicate used to miss (#663) ──────
//
// Each range is pinned from BOTH sides: the first and last address inside
// it must be private, and the addresses immediately below and above must
// stay public. A test placed only in the middle of a range passes against
// an off-by-one in the bound and so guards nothing.

#[test]
fn cgnat_shared_address_space_is_private() {
    // RFC 6598: 100.64.0.0/10 spans 100.64.0.0 ..= 100.127.255.255.
    assert!(is_private_host("100.64.0.0"), "first address in the range");
    assert!(is_private_host("100.100.50.1"), "interior");
    assert!(
        is_private_host("100.127.255.255"),
        "last address in the range"
    );
}

#[test]
fn addresses_bracketing_the_cgnat_range_stay_public() {
    assert!(!is_private_host("100.63.255.255"), "one below the range");
    assert!(!is_private_host("100.128.0.0"), "one above the range");
}

#[test]
fn ietf_protocol_assignment_range_is_private() {
    // RFC 6890: 192.0.0.0/24 spans 192.0.0.0 ..= 192.0.0.255.
    assert!(is_private_host("192.0.0.0"));
    assert!(is_private_host("192.0.0.255"));
}

#[test]
fn addresses_bracketing_the_ietf_range_stay_public() {
    assert!(!is_private_host("191.255.255.255"), "one below the range");
    assert!(!is_private_host("192.0.1.0"), "one above the range");
}

#[test]
fn benchmarking_range_is_private() {
    // RFC 2544: 198.18.0.0/15 spans 198.18.0.0 ..= 198.19.255.255.
    assert!(is_private_host("198.18.0.0"));
    assert!(is_private_host("198.19.255.255"));
}

#[test]
fn addresses_bracketing_the_benchmarking_range_stay_public() {
    assert!(!is_private_host("198.17.255.255"), "one below the range");
    assert!(!is_private_host("198.20.0.0"), "one above the range");
}

#[test]
fn six_to_four_relay_anycast_is_private() {
    // RFC 7526 deprecated 192.88.99.0/24; it still routes on some networks.
    assert!(is_private_host("192.88.99.0"));
    assert!(is_private_host("192.88.99.255"));
}

#[test]
fn addresses_bracketing_the_six_to_four_range_stay_public() {
    assert!(!is_private_host("192.88.98.255"), "one below the range");
    assert!(!is_private_host("192.88.100.0"), "one above the range");
}

// ── IPv6 forms that wrap an IPv4 address (#663) ─────────────────

#[test]
fn ipv4_compatible_ipv6_wrapping_a_private_v4_is_private() {
    // The deprecated IPv4-COMPATIBLE form (RFC 4291 s2.5.5.1), distinct from
    // the IPv4-mapped `::ffff:` form already covered. Measured on Rust
    // 1.97.0: `"::127.0.0.1".parse::<Ipv6Addr>()` yields `to_ipv4_mapped()
    // == None` and `is_loopback() == false`, so neither existing check fires.
    assert!(is_private_host("::127.0.0.1"));
    assert!(is_private_host("::10.0.0.1"));
    assert!(is_private_host("::192.168.1.1"));
}

#[test]
fn ipv4_compatible_ipv6_wrapping_a_public_v4_stays_public() {
    assert!(!is_private_host("::8.8.8.8"));
}

#[test]
fn nat64_well_known_prefix_wrapping_a_private_v4_is_private() {
    // RFC 6052 s3.1: "The Well-Known Prefix MUST NOT be used to represent
    // non-global IPv4 addresses, such as those defined in [RFC1918]" and
    // translators "MUST drop these packets". Such an address is therefore
    // both non-conformant and an SSRF attempt.
    assert!(is_private_host("64:ff9b::127.0.0.1"));
    assert!(is_private_host("64:ff9b::10.0.0.1"));
    assert!(is_private_host("64:ff9b::169.254.169.254"));
}

#[test]
fn nat64_well_known_prefix_wrapping_a_global_v4_stays_public() {
    // The legitimate use of the prefix. Blocking this would break NAT64
    // networks outright, so the check must read the embedded address.
    assert!(!is_private_host("64:ff9b::8.8.8.8"));
}

#[test]
fn a_prefix_resembling_nat64_is_not_treated_as_nat64() {
    // Only 64:ff9b::/96 is the Well-Known Prefix. A neighbouring prefix
    // carrying the same trailing bits must not inherit the check.
    assert!(!is_private_host("64:ff9c::10.0.0.1"));
    assert!(!is_private_host("65:ff9b::10.0.0.1"));
}

// ── The decimal/octal/hex normalisation this gate relies on (#663) ──

#[test]
fn alternate_ipv4_encodings_are_rejected_through_validate_url_security() {
    // This protection is currently a behaviour of the `url` crate rather
    // than of this crate: special-scheme hosts ending in a number are run
    // through `parse_ipv4addr`, which accepts decimal, octal and hex forms
    // and re-serialises them as a dotted quad, so `is_private_host` sees
    // "127.0.0.1". Nothing in this repo would notice if that stopped.
    //
    // 2130706433 == 0x7f000001 == 127.0.0.1
    for url in [
        "http://2130706433/x", // decimal
        "http://0x7f000001/x", // hex
        "http://0177.0.0.1/x", // octal first octet
        "http://127.1/x",      // short form
    ] {
        assert!(
            validate_url_security(url).is_err(),
            "{url} must be rejected as loopback"
        );
    }
}
