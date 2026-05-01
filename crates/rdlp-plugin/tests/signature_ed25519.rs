// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs,
)]

// Lints suppressed for test code — panicking on unexpected errors is intentional here.

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rdlp_plugin::manifest::{Manifest, Signature, canonical_bytes, parse_manifest_str};
use rdlp_plugin::signature::ed25519::verify_ed25519;

fn build_signed_manifest(wasm_bytes: &[u8]) -> (Manifest, SigningKey) {
    let mut csprng = OsRng;
    let key = SigningKey::generate(&mut csprng);

    let pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    let toml = format!(
        r#"
name = "test"
version = "1.0.0"
wit_version = "0.1.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#
    );

    let mut m = parse_manifest_str(&toml).unwrap();
    let mut buf = canonical_bytes(&m);
    buf.extend_from_slice(wasm_bytes);
    let sig = key.sign(&buf);

    if let Signature::Ed25519 { signature, .. } = &mut m.signature {
        *signature = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    }
    (m, key)
}

#[test]
fn valid_signature_verifies() {
    let wasm = b"fake-wasm-bytes";
    let (m, _key) = build_signed_manifest(wasm);
    verify_ed25519(&m, wasm).expect("valid signature should verify");
}

#[test]
fn tampered_wasm_fails() {
    let wasm = b"fake-wasm-bytes";
    let (m, _key) = build_signed_manifest(wasm);
    let tampered = b"tampered-wasm-bytes";
    let err = verify_ed25519(&m, tampered).unwrap_err();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
}

#[test]
fn wrong_key_fails() {
    let wasm = b"fake-wasm-bytes";
    let (mut m, _key) = build_signed_manifest(wasm);
    let other = SigningKey::generate(&mut OsRng);
    if let Signature::Ed25519 { pubkey, .. } = &mut m.signature {
        *pubkey =
            base64::engine::general_purpose::STANDARD.encode(other.verifying_key().as_bytes());
    }
    let err = verify_ed25519(&m, wasm).unwrap_err();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
}

#[test]
fn malformed_pubkey_base64_fails_clearly() {
    let wasm = b"fake-wasm-bytes";
    let (mut m, _key) = build_signed_manifest(wasm);
    if let Signature::Ed25519 { pubkey, .. } = &mut m.signature {
        *pubkey = "not-valid-base64!@#$".into();
    }
    let err = verify_ed25519(&m, wasm).unwrap_err();
    let msg = err.to_string();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
    assert!(msg.contains("pubkey") || msg.contains("base64"));
}

#[test]
fn truncated_signature_fails_clearly() {
    let wasm = b"fake-wasm-bytes";
    let (mut m, _key) = build_signed_manifest(wasm);
    if let Signature::Ed25519 { signature, .. } = &mut m.signature {
        *signature = base64::engine::general_purpose::STANDARD.encode([0u8; 10]);
    }
    let err = verify_ed25519(&m, wasm).unwrap_err();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
}

#[test]
fn rejects_sigstore_signature_variant() {
    let wasm = b"fake-wasm-bytes";
    let toml = r#"
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
bundle = "deadbeef"
"#;
    let m = parse_manifest_str(toml).unwrap();
    let err = verify_ed25519(&m, wasm).unwrap_err();
    let msg = err.to_string();
    assert!(matches!(
        err,
        rdlp_plugin::PluginError::SignatureInvalid { .. }
    ));
    assert!(msg.to_lowercase().contains("ed25519"));
}
