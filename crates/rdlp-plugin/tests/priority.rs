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

use rdlp_plugin::manifest::{Manifest, parse_manifest_str};
use rdlp_plugin::priority::{BUILT_IN_MAX, PLUGIN_MAX, PLUGIN_MIN, USER_MAX, effective_priority};
use url::Url;

fn manifest_for_test(priority: u32, claims_override: &[&str]) -> Manifest {
    let claims_str = claims_override
        .iter()
        .map(|h| format!("\"{h}\""))
        .collect::<Vec<_>>()
        .join(", ");

    // Include each claims_override host in the matches list so the new M4
    // validation (claims_override must correspond to a matches host) passes.
    // The tests verify priority logic, not manifest validation, so this is the
    // correct fix — a real plugin declaring claims_override["youtube.com"] would
    // also match on youtube.com URLs.
    let mut match_patterns: Vec<String> = vec!["\"https://example.com/*\"".to_string()];
    for h in claims_override {
        match_patterns.push(format!("\"https://{h}/*\""));
    }
    let matches_str = match_patterns.join(", ");

    let toml = format!(
        r#"
name = "test"
version = "1.0.0"
wit_version = "0.1.0"
matches = [{matches_str}]
priority = {priority}
claims_override = [{claims_str}]
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#,
    );
    parse_manifest_str(&toml).expect("parse")
}

#[test]
fn constants_are_correct() {
    assert_eq!(BUILT_IN_MAX, 99);
    assert_eq!(PLUGIN_MIN, 100);
    assert_eq!(PLUGIN_MAX, 199);
    assert_eq!(USER_MAX, 255);
}

#[test]
fn no_override_no_builtin_claim_keeps_declared_priority() {
    let m = manifest_for_test(150, &[]);
    let url = Url::parse("https://obscure-site.example/page").unwrap();
    assert_eq!(effective_priority(&m, &url, false, None), 150);
}

#[test]
fn no_override_with_builtin_claim_clamps_to_99() {
    let m = manifest_for_test(199, &[]);
    let url = Url::parse("https://youtube.com/watch?v=1").unwrap();
    assert_eq!(effective_priority(&m, &url, true, None), 99);
}

#[test]
fn explicit_override_for_host_keeps_full_priority() {
    let m = manifest_for_test(199, &["youtube.com"]);
    let url = Url::parse("https://youtube.com/watch?v=1").unwrap();
    assert_eq!(effective_priority(&m, &url, true, None), 199);
}

#[test]
fn override_for_subdomain_via_suffix_match() {
    let m = manifest_for_test(150, &["youtube.com"]);
    let url = Url::parse("https://www.youtube.com/").unwrap();
    assert_eq!(effective_priority(&m, &url, true, None), 150);
}

#[test]
fn override_does_not_apply_to_unrelated_host() {
    let m = manifest_for_test(199, &["youtube.com"]);
    let url = Url::parse("https://pornhub.com/").unwrap();
    // built-in claims this URL, plugin has override only for youtube.com,
    // so effective priority for pornhub.com is clamped.
    assert_eq!(effective_priority(&m, &url, true, None), 99);
}

#[test]
fn user_override_supersedes_everything() {
    let m = manifest_for_test(150, &[]);
    let url = Url::parse("https://anywhere.com/").unwrap();
    assert_eq!(effective_priority(&m, &url, true, Some(220)), 220);
    assert_eq!(effective_priority(&m, &url, false, Some(220)), 220);
}

#[test]
fn user_override_clamped_to_user_max() {
    let m = manifest_for_test(150, &[]);
    let url = Url::parse("https://anywhere.com/").unwrap();
    assert_eq!(effective_priority(&m, &url, false, Some(999)), USER_MAX);
}
