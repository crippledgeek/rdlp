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
fn ipv6_v4_mapped_form_is_judged_by_its_inner_address() {
    // ::ffff:0:0/96 — the mapped form must not bypass the v4 gate, and it is
    // judged by the SAME predicate as a bare IPv4, so a range added there
    // reaches this path too.
    assert!(is_private_host("::ffff:127.0.0.1"));
    assert!(is_private_host("::ffff:10.0.0.1"));
    assert!(is_private_host("::ffff:192.168.1.1"));
    assert!(
        is_private_host("::ffff:100.64.0.1"),
        "CGNAT via the mapped form"
    );
    assert!(
        !is_private_host("::ffff:8.8.8.8"),
        "a public v4 stays fetchable"
    );
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
    // "127.0.0.1". This test is the only thing that would notice if that
    // stopped.
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

#[test]
fn globally_reachable_rows_inside_a_blocked_parent_stay_public() {
    // The load-bearing case for longest-prefix match. 192.0.0.0/24 is
    // blocked, but the registry marks these two /32s Globally Reachable, so
    // the more specific row must win. A first-match-wins table gets this
    // wrong in whichever order it happens to be written.
    assert!(!is_private_host("192.0.0.9"), "PCP anycast, RFC 7723");
    assert!(!is_private_host("192.0.0.10"), "TURN anycast, RFC 8155");
    // Their immediate neighbours have no carve-out and stay blocked.
    assert!(is_private_host("192.0.0.8"), "IPv4 dummy address, RFC 7600");
    assert!(is_private_host("192.0.0.11"));
}

#[test]
fn globally_reachable_special_purpose_ranges_stay_public() {
    // Present in the registry but marked Globally Reachable, so blocking
    // them would be a false positive on real routable space.
    assert!(!is_private_host("192.31.196.1"), "AS112-v4, RFC 7535");
    assert!(!is_private_host("192.52.193.1"), "AMT, RFC 7450");
    assert!(
        !is_private_host("192.175.48.1"),
        "AS112 delegation, RFC 7534"
    );
}

// ── 6to4 and Teredo: IPv6 forms that tunnel to an embedded IPv4 ──

#[test]
fn six_to_four_wrapping_a_private_v4_is_private() {
    // RFC 3056 s2 puts V4ADDR in bits 16..=47, and s5.3 encapsulates toward
    // it — so 2002:7f00:1:: IS 127.0.0.1. The commit that added the NAT64
    // unwrap blocked the 6to4 RELAY range (192.88.99.0/24) while leaving
    // this, the prefix that uses the relay, wide open.
    assert!(is_private_host("2002:7f00:1::"), "127.0.0.1");
    assert!(
        is_private_host("2002:a9fe:a9fe::"),
        "169.254.169.254 metadata"
    );
    assert!(is_private_host("2002:c0a8:101::"), "192.168.1.1");
}

#[test]
fn six_to_four_wrapping_a_public_v4_stays_public() {
    // The legitimate use of the prefix; blocking it outright would be wrong.
    assert!(!is_private_host("2002:808:808::"), "8.8.8.8");
}

#[test]
fn a_prefix_resembling_six_to_four_is_not_unwrapped() {
    // Only 2002::/16. A neighbouring prefix carrying the same trailing bits
    // must not inherit the check.
    assert!(!is_private_host("2003:7f00:1::"));
    assert!(!is_private_host("2001:7f00:1::1"));
}

#[test]
fn teredo_wrapping_a_private_server_or_client_is_private() {
    // RFC 4380 s4: prefix | server IPv4 | flags | obf port | obf client IPv4.
    // The server sits at bits 32..=63 unobfuscated; the client at 96..=127
    // XORed with 0xFFFFFFFF, so 127.0.0.1 (0x7f000001) is written 80ff:fffe.
    assert!(
        is_private_host("2001:0:7f00:1:0:0:f7f7:fbfb"),
        "server 127.0.0.1, client 8.8.4.4"
    );
    assert!(
        is_private_host("2001:0:808:808:0:0:80ff:fffe"),
        "server 8.8.8.8, client 127.0.0.1 (de-obfuscated)"
    );
}

#[test]
fn teredo_wrapping_public_addresses_stays_public() {
    assert!(
        !is_private_host("2001:0:808:808:0:0:f7f7:fbfb"),
        "server 8.8.8.8, client 8.8.4.4 — both public"
    );
}

#[test]
fn a_prefix_resembling_teredo_is_not_unwrapped() {
    // The Teredo prefix is 2001:0000::/32 — segment 1 must be zero. Without
    // that, every 2001::/16 address would have its segments misread as an
    // embedded IPv4.
    assert!(!is_private_host("2001:1:7f00:1:0:0:f7f7:fbfb"));
}

#[test]
fn rfc8215_local_use_nat64_prefix_is_not_the_well_known_prefix() {
    // RFC 8215: "64:ff9b:1::/48 ... is distinct from the WKP 64:ff9b::/96.
    // Therefore, the restrictions on the use of the WKP described in Section
    // 3.1 of [RFC6052] do not apply". The middle-segment clause of the WKP
    // match is the only thing keeping it out, and without this test deleting
    // that clause leaves the suite green.
    assert!(!is_private_host("64:ff9b:1:0:0:0:10.0.0.1"));
}

// ── The registry table, checked against an independent transcription ──
//
// `IPV4_SPECIAL_PURPOSE` is hand-typed as `Ipv4Addr::new(a, b, c, d)` plus a
// prefix length. Ranges that used to come from `Ipv4Addr::is_private()` and
// friends are now among those rows, so a `/12` mistyped as `/16` on
// 172.16.0.0 would silently unblock 172.17.0.0-172.31.255.255.
//
// These tests are a differential check, not a restatement: the expectations
// below are transcribed from the IANA registry independently, so a slip in
// either transcription makes the two disagree. Reading the expectation off
// the implementation's own table would prove nothing.
//
// Each row carries its first and last address as literals ALONGSIDE the CIDR,
// and the two are asserted to agree. Deriving both ends from the CIDR alone
// would tie the expectation to the same prefix length being checked, so a red
// test could be "fixed" by editing this table to match the implementation.
// The literals are a second encoding of the range itself, not just of the
// prefix, and they are what the deleted per-range tests contributed.

/// Every row of the registry the predicate is meant to implement.
/// `true` = must be refused.
const EXPECTED_REGISTRY: &[(&str, &str, &str, bool)] = &[
    ("0.0.0.0/8", "0.0.0.0", "0.255.255.255", true),
    ("10.0.0.0/8", "10.0.0.0", "10.255.255.255", true),
    ("100.64.0.0/10", "100.64.0.0", "100.127.255.255", true),
    ("127.0.0.0/8", "127.0.0.0", "127.255.255.255", true),
    ("169.254.0.0/16", "169.254.0.0", "169.254.255.255", true),
    ("172.16.0.0/12", "172.16.0.0", "172.31.255.255", true),
    ("192.0.0.0/24", "192.0.0.0", "192.0.0.255", true),
    ("192.0.0.9/32", "192.0.0.9", "192.0.0.9", false),
    ("192.0.0.10/32", "192.0.0.10", "192.0.0.10", false),
    ("192.0.2.0/24", "192.0.2.0", "192.0.2.255", true),
    ("192.88.99.0/24", "192.88.99.0", "192.88.99.255", true),
    ("192.168.0.0/16", "192.168.0.0", "192.168.255.255", true),
    ("198.18.0.0/15", "198.18.0.0", "198.19.255.255", true),
    ("198.51.100.0/24", "198.51.100.0", "198.51.100.255", true),
    ("203.0.113.0/24", "203.0.113.0", "203.0.113.255", true),
    ("240.0.0.0/4", "240.0.0.0", "255.255.255.255", true),
];

/// Registry rows deliberately left out of `IPV4_SPECIAL_PURPOSE`, and how the
/// omission rule says each must still resolve. The first seven are `Blocked`
/// rows nested inside another `Blocked` row, so the parent decides; the last
/// three are `Globally Reachable` rows with no blocked parent, so matching no
/// row at all leaves them fetchable.
const OMITTED_ROWS: &[(&str, bool)] = &[
    ("0.0.0.0/32", true),         // inside 0.0.0.0/8
    ("192.0.0.0/29", true),       // inside 192.0.0.0/24
    ("192.0.0.8/32", true),       // IPv4 dummy address, RFC 7600
    ("192.0.0.170/32", true),     // NAT64/DNS64 discovery, RFC 8880
    ("192.0.0.171/32", true),     // NAT64/DNS64 discovery, RFC 8880
    ("192.88.99.2/32", true),     // 6a44-relay anycast, inside 192.88.99.0/24
    ("255.255.255.255/32", true), // inside 240.0.0.0/4
    ("192.31.196.0/24", false),   // AS112-v4, RFC 7535
    ("192.52.193.0/24", false),   // AMT, RFC 7450
    ("192.175.48.0/24", false),   // AS112 delegation, RFC 7534
];

/// What the independent transcription says about one address: longest-prefix
/// match over [`EXPECTED_REGISTRY`], then multicast, then public.
fn model_refuses(ip: Ipv4Addr) -> bool {
    let matched = EXPECTED_REGISTRY
        .iter()
        .filter_map(|(cidr, _, _, blocked)| {
            let net: ipnet::Ipv4Net = cidr.parse().expect("test CIDR must parse");
            net.contains(&ip).then_some((net.prefix_len(), *blocked))
        })
        .max_by_key(|(len, _)| *len);
    match matched {
        Some((_, blocked)) => blocked,
        None => ip.is_multicast(),
    }
}

fn assert_agrees(ip: Ipv4Addr, context: &str) {
    let expected = model_refuses(ip);
    assert_eq!(
        is_private_host(&ip.to_string()),
        expected,
        "{ip} ({context}): registry says refuse={expected}"
    );
}

#[test]
fn every_registry_row_spans_the_addresses_it_claims_to() {
    // The literals and the prefix length are two encodings of one range;
    // this is where they are made to agree.
    //
    // Measured by mutating one prefix length and running each test alone —
    // `spans` is this test, `pins` is every_registry_row_is_pinned_at_both_ends,
    // `nbr` is addresses_immediately_outside_every_row_agree_with_the_registry:
    //
    //     mutation                       spans  pins  nbr
    //     implementation row narrowed     pass  FAIL  pass
    //     test CIDR narrowed              FAIL  FAIL  FAIL
    //     implementation row widened      pass  pass  FAIL
    //     test CIDR widened               FAIL  pass  pass
    //     both tables narrowed together   FAIL  pass  pass
    //
    // So this test is the ONLY guard against the last two. A widened test
    // CIDR leaves both literals inside the row, so the pins still agree, and
    // it moves the neighbour probes outward with it, so the one address that
    // would disagree is never tried. A co-ordinated edit moves the model and
    // the expectation together, which is the hole the literals exist to
    // close. The other two rows are covered without it.
    for (cidr, first, last, _) in EXPECTED_REGISTRY {
        let net: ipnet::Ipv4Net = cidr.parse().expect("test CIDR must parse");
        assert_eq!(net.network().to_string(), *first, "first address of {cidr}");
        assert_eq!(net.broadcast().to_string(), *last, "last address of {cidr}");
    }
}

#[test]
fn every_registry_row_is_pinned_at_both_ends() {
    for (cidr, first, last, _) in EXPECTED_REGISTRY {
        for (literal, end) in [(first, "first"), (last, "last")] {
            let ip: Ipv4Addr = literal.parse().expect("test address must parse");
            assert_agrees(ip, &format!("{end} address of {cidr}"));
        }
    }
}

#[test]
fn addresses_immediately_outside_every_row_agree_with_the_registry() {
    // The neighbours are checked against the model rather than asserted
    // public outright, because rows abut: the address below 240.0.0.0 is
    // 239.255.255.255, which is multicast and refused for its own reason.
    for (cidr, _, _, _) in EXPECTED_REGISTRY {
        let net: ipnet::Ipv4Net = cidr.parse().expect("test CIDR must parse");
        if let Some(below) = u32::from(net.network()).checked_sub(1) {
            assert_agrees(Ipv4Addr::from(below), &format!("one below {cidr}"));
        }
        if let Some(above) = u32::from(net.broadcast()).checked_add(1) {
            assert_agrees(Ipv4Addr::from(above), &format!("one above {cidr}"));
        }
    }
}

#[test]
fn omitted_registry_rows_resolve_as_the_rule_claims() {
    // The table's doc comment justifies leaving these out on the grounds that
    // neither kind can change an answer. That justification is only as good as
    // its last check, so it is checked.
    for (cidr, expected) in OMITTED_ROWS {
        let net: ipnet::Ipv4Net = cidr.parse().expect("test CIDR must parse");
        for (ip, end) in [(net.network(), "first"), (net.broadcast(), "last")] {
            assert_eq!(
                is_private_host(&ip.to_string()),
                *expected,
                "{ip} ({end} address of omitted row {cidr})"
            );
        }
    }
}

#[test]
fn the_multicast_lower_edge_is_pinned_from_both_sides() {
    // 224.0.0.0/4 is the one refused range that is NOT a table row — it comes
    // from `Ipv4Addr::is_multicast`. Its upper edge abuts 240.0.0.0/4, so the
    // neighbour test can only reach it from below; without this, the boundary
    // between public space and multicast is unpinned in both directions.
    assert!(!is_private_host("223.255.255.255"), "one below multicast");
    assert!(is_private_host("224.0.0.0"), "first multicast address");
}
