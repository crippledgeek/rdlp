#![allow(clippy::disallowed_methods)]

use base64::Engine as _;
use rdlp_plugin::manifest::parse_manifest_str;
use rdlp_plugin::signature::sigstore::verify_sigstore;

fn manifest_with_sigstore_bundle(bundle_b64: &str) -> String {
    format!(
        r#"
name = "test"
version = "1.0.0"
wit_version = "0.1.0"
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
wit_version = "0.1.0"
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

#[test]
#[ignore = "sigstore-rs 0.13 only parses Bundle v0.1/v0.2; cosign 2.4 --new-bundle-format emits v0.3. Un-ignore once sigstore-rs adds v0.3 support upstream."]
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
