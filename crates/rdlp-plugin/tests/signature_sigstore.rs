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

use base64::Engine as _;
use rdlp_plugin::manifest::parse_manifest_str;
use rdlp_plugin::signature::sigstore::verify_sigstore;

fn manifest_with_sigstore_bundle(bundle_b64: &str) -> String {
    format!(
        r#"
name = "test"
version = "1.0.0"
wit_version = "0.3.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "sigstore"
identity = "github:user/repo"
oidc_issuer = "https://token.actions.githubusercontent.com"
bundle = "{bundle_b64}"
"#
    )
}

#[test]
fn rejects_non_sigstore_variant() {
    let toml = r#"
name = "test"
version = "1.0.0"
wit_version = "0.3.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
signature = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#;
    let m = parse_manifest_str(toml).unwrap();
    let err = verify_sigstore(&m, b"wasm").unwrap_err();
    let msg = err.to_string();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
    assert!(msg.contains("expected sigstore signature variant"));
}

#[test]
fn rejects_malformed_base64_bundle() {
    let toml = manifest_with_sigstore_bundle("not_base64!!!");
    let m = parse_manifest_str(&toml).unwrap();
    let err = verify_sigstore(&m, b"wasm").unwrap_err();
    let msg = err.to_string();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
    assert!(
        msg.contains("base64 decode bundle"),
        "expected base64 decode error, got: {msg}"
    );
}

#[test]
fn rejects_malformed_json_bundle() {
    let bundle_b64 = base64::engine::general_purpose::STANDARD.encode(b"not json");
    let toml = manifest_with_sigstore_bundle(&bundle_b64);
    let m = parse_manifest_str(&toml).unwrap();
    let err = verify_sigstore(&m, b"wasm").unwrap_err();
    let msg = err.to_string();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
    assert!(
        msg.contains("parse bundle"),
        "expected parse bundle error, got: {msg}"
    );
}

// Bundle v0.3 happy-path is gated behind upstream sigstore-rs support.
// Status as of 2026-04-29:
//   - sigstore-rs latest = 0.13.0 (published 2025-10-16) — no v0.3 parser.
//     Tracking: https://github.com/sigstore/sigstore-rs/issues/432 (open).
//     Reference PR https://github.com/sigstore/sigstore-rs/pull/518 closed
//     un-merged (Nov 2025); the prerequisite PR chain (#513 Merkle/Rekor v2,
//     #514 DSSE) is closed/blocked respectively. Indefinite ETA upstream.
//   - cosign 2.4+ emits Bundle v0.3 by default with --new-bundle-format.
// Verifier-side fallback options if v0.3 becomes a hard requirement before
// upstream lands:
//   1. Switch verifier dep to `sigstore-verify` 0.6 (prefix-dev/sigstore-rust),
//      a workspace shipped by the same author who closed the upstream demo PR.
//      66k+ downloads, has its own conformance suite, Apache-2.0.
//   2. Sign without --new-bundle-format (cosign still supports v0.1/v0.2).
// Sad-path coverage (mismatched signatures, tampered bundles, missing
// fields) is in `signature_sigstore_negative.rs` and runs unconditionally.
#[test]
#[ignore = "sigstore-rs 0.13 only parses Bundle v0.1/v0.2; see comment above for upstream status + migration options"]
fn valid_sigstore_bundle_verifies() {
    let manifest_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sigstore_valid.toml");
    let wasm_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sigstore_valid.wasm");
    let toml = std::fs::read_to_string(&manifest_path).expect("missing sigstore manifest fixture");
    let wasm = std::fs::read(&wasm_path).expect("missing sigstore wasm fixture");
    let m = parse_manifest_str(&toml).expect("parse manifest");
    verify_sigstore(&m, &wasm).expect("valid sigstore bundle should verify");
}
