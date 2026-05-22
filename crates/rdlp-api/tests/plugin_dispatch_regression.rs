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

//! Regression test for the plugin dispatch wiring bug.
//!
//! Earlier MVP code only routed the plugin-aware `ExtractorRegistry` to the
//! download path; `extract_info`, `list_extractors`, `search`, and friends fell
//! back to a process-level static built-in-only registry, so a configured
//! plugin's URL would dispatch to the generic extractor instead of the plugin.
//!
//! This test loads a real Ed25519-signed example plugin into a temp dir,
//! constructs an `RdlpClient` with `plugin_directories` set, and asserts that
//! `list_extractors()` includes the plugin's name. Without the fix this
//! assertion fails because `list_extractors()` consults the built-in registry.

use std::path::PathBuf;

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rdlp_api::RdlpClient;
use rdlp_plugin::manifest::{canonical_bytes, parse_manifest_str};
use rdlp_types::Config;

const EXAMPLE_WASM: &str =
    "../../examples/plugins/example-extractor/target/wasm32-wasip1/release/example_extractor.wasm";

fn workspace_relative(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

#[test]
fn list_extractors_includes_loaded_plugin() {
    let wasm_src = workspace_relative(EXAMPLE_WASM);
    if !wasm_src.exists() {
        eprintln!(
            "skipping: example-extractor wasm not built at {}\n\
             run `cd examples/plugins/example-extractor && cargo component build --release` first",
            wasm_src.display()
        );
        return;
    }

    let tempdir = tempfile::tempdir().expect("tempdir");
    let path_str = tempdir.path().to_str().expect("tempdir path is utf-8");

    // Isolate the trust store from any pre-existing global state. The bootstrap
    // resolves the trust store via dirs::config_dir() → XDG_CONFIG_HOME → HOME.
    // Pointing all three at the tempdir guarantees a clean slate.
    // temp-env's internal mutex serialises this against any other test that
    // reads or writes these vars.
    temp_env::with_vars(
        [
            ("XDG_CONFIG_HOME", Some(path_str)),
            ("HOME", Some(path_str)),
        ],
        || {
            let plugin_dir = tempdir.path().join("example");
            std::fs::create_dir_all(&plugin_dir).unwrap();

            let wasm_bytes = std::fs::read(&wasm_src).expect("read example wasm");
            std::fs::write(plugin_dir.join("plugin.wasm"), &wasm_bytes).unwrap();

            let signing_key: SigningKey = SigningKey::generate(&mut OsRng);
            let pubkey_b64 = base64::engine::general_purpose::STANDARD
                .encode(signing_key.verifying_key().as_bytes());

            let template = format!(
                r#"name = "example"
version = "0.1.0"
wit_version = "0.3.0"
matches = ["https://example.com/video/*"]
url_regex = "^https://example\\.com/video/(?P<id>\\d+)"
priority = 150
claims_override = []
supports_search = false
capabilities = []

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#
            );

            let manifest = parse_manifest_str(&template.replace("PLACEHOLDER", "AAAA")).unwrap();
            let mut to_sign = canonical_bytes(&manifest);
            to_sign.extend_from_slice(&wasm_bytes);
            let sig = signing_key.sign(&to_sign);
            let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
            let final_toml = template.replace("PLACEHOLDER", &sig_b64);

            std::fs::write(plugin_dir.join("plugin.toml"), final_toml).unwrap();

            // Pre-trust the publisher so the loader's prompter approves the plugin
            // without interactive input. Identity must match
            // `Signature::identity_string()` exactly, which is `ed25519:<hex of full
            // SHA-256 over the base64-encoded pubkey>` post-MVP-hardening.
            use sha2::{Digest, Sha256};
            let pub_hash_hex = hex::encode(Sha256::digest(pubkey_b64.as_bytes()));
            let identity = format!("ed25519:{pub_hash_hex}");

            let config = Config {
                progress: false,
                plugin_directories: vec![tempdir.path().to_path_buf()],
                plugin_trusted_publishers: vec![identity],
                ..Default::default()
            };

            let client = RdlpClient::new(config).expect("client");
            let extractors = client.list_extractors();
            assert!(
                extractors.contains(&"example"),
                "expected `example` plugin in list_extractors() output, got: {extractors:?}"
            );
        },
    );
}
