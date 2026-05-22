// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs
)]

// Lints suppressed for test code — panicking on unexpected errors is intentional here.

use rdlp_plugin::manifest::{Signature, parse_manifest_str};

const VALID_TOML: &str = r#"
name = "youtube"
version = "1.4.2"
wit_version = "0.4.0"
matches = ["https://*.youtube.com/*"]
url_regex = '^https?://(?:www\.)?youtube\.com/watch\?v=(?P<id>[A-Za-z0-9_-]{11})'
priority = 150
claims_override = []
supports_search = true
capabilities = ["fetch", "log"]

[signature]
type = "ed25519"
pubkey = "MCowBQYDK2VwAyEA8R4dJ8U5N7l4M7g7Q3PqQ7Q3PqQ7Q3PqQ7Q3PqQ7Q="
signature = "dGVzdC1zaWctYmFzZTY0LWVuY29kZWQtcGFkZGVkLXRvLTY0LWNoYXJzPT09PT09PQ"
"#;

#[test]
fn parse_valid_manifest() {
    let m = parse_manifest_str(VALID_TOML).expect("parse should succeed");
    assert_eq!(m.name, "youtube");
    assert_eq!(m.version, "1.4.2");
    assert_eq!(m.wit_version, "0.4.0");
    assert_eq!(m.matches, vec!["https://*.youtube.com/*"]);
    assert!(m.url_regex.is_some());
    assert_eq!(m.priority, 150);
    assert!(m.supports_search);
    assert_eq!(m.capabilities, vec!["fetch", "log"]);
    assert!(matches!(m.signature, Signature::Ed25519 { .. }));
}

#[test]
fn priority_below_range_rejected() {
    let toml = VALID_TOML.replace("priority = 150", "priority = 99");
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().contains("priority"));
}

#[test]
fn priority_above_range_rejected() {
    let toml = VALID_TOML.replace("priority = 150", "priority = 200");
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().contains("priority"));
}

#[test]
fn empty_matches_rejected() {
    let toml = VALID_TOML.replace(r#"matches = ["https://*.youtube.com/*"]"#, "matches = []");
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("matches"));
}

#[test]
fn unknown_capability_rejected() {
    let toml = VALID_TOML.replace(r#"["fetch", "log"]"#, r#"["fetch", "log", "fs"]"#);
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("capability"));
}

#[test]
fn url_regex_too_long_rejected() {
    let huge = "a".repeat(3000);
    let toml = VALID_TOML.replace(
        r"url_regex = '^https?://(?:www\.)?youtube\.com/watch\?v=(?P<id>[A-Za-z0-9_-]{11})'",
        &format!("url_regex = '{huge}'"),
    );
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("regex"));
}

#[test]
fn tld_wildcard_requires_claim_all_urls_capability() {
    // matches = ["https://*/*"] without "claim-all-urls" should fail
    let toml = VALID_TOML.replace(
        r#"matches = ["https://*.youtube.com/*"]"#,
        r#"matches = ["https://*/*"]"#,
    );
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("claim-all-urls"));
}

#[test]
fn tld_wildcard_accepted_with_claim_all_urls_capability() {
    let toml = VALID_TOML
        .replace(
            r#"matches = ["https://*.youtube.com/*"]"#,
            r#"matches = ["https://*/*"]"#,
        )
        .replace(
            r#"["fetch", "log"]"#,
            r#"["fetch", "log", "claim-all-urls"]"#,
        );
    let m = parse_manifest_str(&toml).expect("should accept");
    assert!(m.capabilities.contains(&"claim-all-urls".to_string()));
}

#[test]
fn bare_tld_wildcard_https_requires_claim_all_urls() {
    let toml = VALID_TOML.replace(
        r#"matches = ["https://*.youtube.com/*"]"#,
        r#"matches = ["https://*"]"#,
    );
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("claim-all-urls"));
}

#[test]
fn bare_tld_wildcard_http_requires_claim_all_urls() {
    let toml = VALID_TOML.replace(
        r#"matches = ["https://*.youtube.com/*"]"#,
        r#"matches = ["http://*"]"#,
    );
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("claim-all-urls"));
}

#[test]
fn bare_any_scheme_wildcard_requires_claim_all_urls() {
    let toml = VALID_TOML.replace(
        r#"matches = ["https://*.youtube.com/*"]"#,
        r#"matches = ["*://*"]"#,
    );
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("claim-all-urls"));
}
