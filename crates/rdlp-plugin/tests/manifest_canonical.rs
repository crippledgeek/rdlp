use rdlp_plugin::manifest::{canonical_bytes, parse_manifest_str};

#[test]
fn canonical_form_is_stable_across_key_order() {
    let a = r#"
name = "x"
version = "1.0.0"
wit_version = "0.1.0"
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
wit_version = "0.1.0"
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
wit_version = "0.1.0"
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
wit_version = "0.1.0"
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
wit_version = "0.1.0"
matches = ["https://x.com/*"]
url_regex = "^https://x\\.com/(?P<id>\\d+)"
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
wit_version = "0.1.0"
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
fn canonical_form_keys_are_sorted() {
    // Keys should appear in lexicographic order. Verify the first three.
    let toml = r#"
name = "x"
version = "1.0.0"
wit_version = "0.1.0"
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
