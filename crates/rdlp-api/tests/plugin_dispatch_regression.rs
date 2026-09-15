// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `missing_docs` exempt
// because integration tests aren't public API.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

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
//!
//! Uses the committed 0.5.0 example fixture
//! (`rdlp_plugin::test_support::EXAMPLE_0_5_0_WASM`, via
//! `SignedPluginSpec::example`) — a real, WASI-free component loadable by
//! the production loader — rather than a wasm artefact built on demand
//! from `examples/plugins/example-extractor`.
//! The earlier version of this test silently `return`ed (exit 0, no
//! failure) when that on-demand artefact wasn't present, which is the same
//! self-skip defect class as the deleted `adapter_trap_disable.rs`: a test
//! that can pass without ever running its assertion is not a regression
//! guard. The committed fixture is always present, so there is nothing
//! left to skip.

use rdlp_api::RdlpClient;
use rdlp_plugin::test_support::{
    SignedPluginSpec, signed_fixture_config, with_isolated_config_dir,
};

#[test]
fn list_extractors_includes_loaded_plugin() {
    with_isolated_config_dir(|_config_dir| {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config = signed_fixture_config(
            tempdir.path(),
            &SignedPluginSpec {
                matches: &["https://example.com/video/*"],
                url_regex: Some(r"^https://example\.com/video/(?P<id>\d+)"),
                ..SignedPluginSpec::example()
            },
        );

        let client = RdlpClient::new(config).expect("client");
        let extractors = client.list_extractors();
        assert!(
            extractors.contains(&"example"),
            "expected `example` plugin in list_extractors() output, got: {extractors:?}"
        );
    });
}
