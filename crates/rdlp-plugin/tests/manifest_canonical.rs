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

use rdlp_plugin::manifest::{canonical_bytes, parse_manifest_str};

#[test]
fn canonical_form_is_stable_across_key_order() {
    let a = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;

    let b = r#"
priority = 150
version = "1.0.0"
wit_version = "0.5.0"
name = "x"
capabilities = ["log"]
matches = ["https://x.com/*"]

[signature]
signature = "ZA"
pubkey = "ZA"
type = "ed25519"
"#;

    let ma = parse_manifest_str(a).unwrap();
    let mb = parse_manifest_str(b).unwrap();

    assert_eq!(
        canonical_bytes(&ma),
        canonical_bytes(&mb),
        "canonical form must be order-independent"
    );
}

#[test]
fn canonical_form_excludes_signature_field() {
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let bytes = canonical_bytes(&m);
    let s = std::str::from_utf8(&bytes).unwrap();
    assert!(
        !s.contains("[signature]"),
        "must exclude signature block header"
    );
    assert!(!s.contains("pubkey"), "must not include signature fields");
    assert!(
        !s.contains("signature ="),
        "must not include signature value"
    );
}

#[test]
fn canonical_form_sorts_list_contents() {
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://b.com/*", "https://a.com/*"]
priority = 150
capabilities = ["log", "fetch"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    // The "https://a.com/*" must appear before "https://b.com/*" in the canonical form
    let a_pos = s.find("a.com").unwrap();
    let b_pos = s.find("b.com").unwrap();
    assert!(a_pos < b_pos, "list contents must be sorted");
    let fetch_pos = s.find("fetch").unwrap();
    let log_pos = s.find("log").unwrap();
    assert!(fetch_pos < log_pos, "capability list must be sorted");
}

#[test]
fn canonical_form_includes_optional_url_regex_when_present() {
    let with_regex = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
url_regex = '^https://x\.com/(?P<id>\d+)'
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(with_regex).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert!(s.contains("url_regex"));
}

#[test]
fn canonical_form_omits_url_regex_when_absent() {
    let without_regex = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(without_regex).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert!(!s.contains("url_regex"));
}

#[test]
fn canonical_bytes_of_a_pre_0_5_1_manifest_are_unchanged() {
    // Same fixture as `canonical_form_excludes_signature_field` above, with no
    // `supports_extract`/`search_site` set — a manifest written before those
    // fields existed. Pinned by running this exact parse+encode against the
    // pre-0.5.1 code (before Task 10's fields were added) and copying its
    // real output, so a regression here means an existing plugin's signature
    // silently stops verifying.
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert_eq!(
        s,
        "capabilities = [\"log\"]\nclaims_override = []\nmatches = [\"https://x.com/*\"]\nname = \"x\"\npriority = 150\nsupports_search = false\nversion = \"1.0.0\"\nwit_version = \"0.5.0\"\n"
    );
}

#[test]
fn supports_extract_defaults_true_and_is_absent_from_canonical_bytes() {
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
supports_search = true
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    assert!(m.supports_extract);
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert!(!s.contains("supports_extract"));
}

#[test]
fn supports_extract_false_is_in_canonical_bytes_only_when_false() {
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
supports_search = true
supports_extract = false
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert!(s.contains("supports_extract = false\n"));
}

#[test]
fn search_site_defaults_to_name_and_is_canonical_only_when_present() {
    let without = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let with = r#"
name = "x-search"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
search_site = "xhamster"
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m_without = parse_manifest_str(without).unwrap();
    let m_with = parse_manifest_str(with).unwrap();
    assert_eq!(m_without.search_site_name(), "x");
    assert_eq!(m_with.search_site_name(), "xhamster");

    let s_without = String::from_utf8(canonical_bytes(&m_without)).unwrap();
    let s_with = String::from_utf8(canonical_bytes(&m_with)).unwrap();
    assert!(!s_without.contains("search_site"));
    assert!(s_with.contains("search_site = \"xhamster\""));
}

#[test]
fn canonical_form_keys_are_sorted() {
    // Keys should appear in lexicographic order. Verify the first three.
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
supports_search = true
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    let lines: Vec<&str> = s.lines().filter(|l| !l.is_empty()).collect();
    let first_keys: Vec<&str> = lines
        .iter()
        .take(3)
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    let mut sorted = first_keys.clone();
    sorted.sort();
    assert_eq!(first_keys, sorted, "keys must be in lexicographic order");
}

#[test]
fn search_claims_override_is_canonical_only_when_non_empty() {
    let without = r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
supports_search = true
search_claims_override = []
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(without).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert!(
        !s.contains("search_claims_override"),
        "an empty (default) claim must leave pre-0.5.1 bytes untouched: {s}"
    );
}

/// Exact-bytes golden for a manifest carrying every 0.5.1 field at a
/// non-default value — the shape a third-party signer must reproduce
/// byte-for-byte. Keys sorted, new keys interleaved in that order,
/// `supports_extract` present only because it is `false`.
#[test]
fn canonical_bytes_of_a_full_0_5_1_manifest_are_pinned() {
    let toml = r#"
name = "ph-search"
version = "1.0.0"
wit_version = "0.5.1"
matches = ["https://x.com/*"]
priority = 150
supports_search = true
supports_extract = false
search_site = "pornhub"
search_claims_override = ["pornhub"]
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let s = String::from_utf8(canonical_bytes(&m)).unwrap();
    assert_eq!(
        s,
        "capabilities = [\"log\"]\nclaims_override = []\nmatches = [\"https://x.com/*\"]\nname = \"ph-search\"\npriority = 150\nsearch_claims_override = [\"pornhub\"]\nsearch_site = \"pornhub\"\nsupports_extract = false\nsupports_search = true\nversion = \"1.0.0\"\nwit_version = \"0.5.1\"\n"
    );
}
