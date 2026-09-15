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

use rdlp_plugin::manifest::{ManifestError, Signature, parse_manifest_str};

const VALID_TOML: &str = r#"
name = "youtube"
version = "1.4.2"
wit_version = "0.5.0"
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
    assert_eq!(m.wit_version, "0.5.0");
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

#[test]
fn a_plugin_with_neither_capability_is_rejected() {
    let toml = VALID_TOML
        .replace("supports_search = true", "supports_search = false")
        .replace(
            "capabilities = [\"fetch\", \"log\"]",
            "supports_extract = false\ncapabilities = [\"fetch\", \"log\"]",
        );
    let err = parse_manifest_str(&toml).unwrap_err();
    match err {
        ManifestError::InvalidManifest { reason, .. } => {
            assert!(reason.contains("supports_extract"));
            assert!(reason.contains("supports_search"));
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[test]
fn search_only_plugin_is_accepted() {
    let toml = VALID_TOML.replace(
        "capabilities = [\"fetch\", \"log\"]",
        "supports_extract = false\ncapabilities = [\"fetch\", \"log\"]",
    );
    let m = parse_manifest_str(&toml).expect("supports_search=true covers the composition rule");
    assert!(!m.supports_extract);
}

#[test]
fn search_site_must_be_a_valid_plugin_name_shape() {
    let toml = VALID_TOML.replace(
        "capabilities = [\"fetch\", \"log\"]",
        "search_site = \"../x\"\ncapabilities = [\"fetch\", \"log\"]",
    );
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(matches!(err, ManifestError::InvalidPluginName { .. }));
}

/// Rewrites `VALID_TOML`'s capabilities line to carry `extra` lines above
/// it, so a test states only the fields it is about.
fn valid_with(extra: &str) -> String {
    VALID_TOML.replace(
        "capabilities = [\"fetch\", \"log\"]",
        &format!("{extra}\ncapabilities = [\"fetch\", \"log\"]"),
    )
}

// ── search_claims_override (security M3) ─────────────────────────────────

#[test]
fn search_claims_override_naming_the_plugins_own_search_site_is_accepted() {
    let m = parse_manifest_str(&valid_with(
        "search_site = \"pornhub\"\nsearch_claims_override = [\"pornhub\"]",
    ))
    .expect("the claim names the site this plugin serves");
    assert_eq!(m.search_claims_override, vec!["pornhub"]);
    assert_eq!(m.search_site_name(), "pornhub");
}

#[test]
fn search_claims_override_defaults_to_name_when_search_site_is_unset() {
    // `youtube` is VALID_TOML's `name`, so it is also the search site.
    let m = parse_manifest_str(&valid_with("search_claims_override = [\"youtube\"]"))
        .expect("the claim names the plugin's own name");
    assert_eq!(m.search_claims_override, vec!["youtube"]);
}

#[test]
fn search_claims_override_for_a_site_this_plugin_does_not_serve_is_rejected() {
    // A plugin serves exactly one search site, so an override claim for
    // any other site is a manifest authoring error — and the shape of a
    // shadowing attempt.
    let err = parse_manifest_str(&valid_with(
        "search_site = \"xhamster\"\nsearch_claims_override = [\"pornhub\"]",
    ))
    .unwrap_err();
    assert!(
        matches!(err, ManifestError::InvalidManifest { ref reason, .. } if reason.contains("search_claims_override")),
        "got {err:?}"
    );
}

#[test]
fn search_claims_override_entries_are_held_to_the_plugin_name_shape() {
    let err = parse_manifest_str(&valid_with("search_claims_override = [\"../x\"]")).unwrap_err();
    assert!(
        matches!(err, ManifestError::InvalidPluginName { .. }),
        "got {err:?}"
    );
}

#[test]
fn search_claims_override_requires_supports_search() {
    let toml = valid_with("search_claims_override = [\"youtube\"]")
        .replace("supports_search = true", "supports_search = false");
    let err = parse_manifest_str(&toml).unwrap_err();
    assert!(
        matches!(err, ManifestError::InvalidManifest { ref reason, .. } if reason.contains("supports_search")),
        "got {err:?}"
    );
}
