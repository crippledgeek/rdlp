// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods`
// permitted for the `std::fs::read` test fixture below per clippy.toml
// policy (c). `missing_docs` exempt because integration tests aren't
// public API.
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

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rdlp_api::RdlpClient;
use rdlp_plugin::test_support::{
    SignedPluginSpec, trusted_identity_for, with_isolated_config_dir, write_signed_plugin,
};
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

    with_isolated_config_dir(|| {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let wasm_bytes = std::fs::read(&wasm_src).expect("read example wasm");

        let signing_key = SigningKey::generate(&mut OsRng);
        write_signed_plugin(
            &tempdir.path().join("example"),
            &signing_key,
            &SignedPluginSpec {
                name: "example",
                version: "0.1.0",
                wit_version: "0.5.0",
                matches: &["https://example.com/video/*"],
                url_regex: Some(r"^https://example\.com/video/(?P<id>\d+)"),
                priority: 150,
                claims_override: &[],
                capabilities: &[],
                supports_extract: true,
                supports_search: false,
                wasm: &wasm_bytes,
            },
        );

        let config = Config {
            plugin_directories: vec![tempdir.path().to_path_buf()],
            plugin_trusted_publishers: vec![trusted_identity_for(&signing_key)],
            ..Default::default()
        };

        let client = RdlpClient::new(config).expect("client");
        let extractors = client.list_extractors();
        assert!(
            extractors.contains(&"example"),
            "expected `example` plugin in list_extractors() output, got: {extractors:?}"
        );
    });
}
