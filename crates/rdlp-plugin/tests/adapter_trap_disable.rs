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

//! Three-strike trap rule for `PluginExtractor`.
//!
//! Loads the reference example-extractor component, calls `record_trap()` three
//! times via the test-only hook, asserts the `disabled` flag flips on the
//! third strike. Skips silently when the example wasm hasn't been built so
//! the test isn't a hard dependency on the cargo-component toolchain.

use std::path::PathBuf;
use std::sync::Arc;

use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::LoadedPlugin;
use rdlp_plugin::manifest::parse_manifest_str;

const EXAMPLE_WASM: &str =
    "../../examples/plugins/example-extractor/target/wasm32-wasip1/release/example_extractor.wasm";

fn workspace_relative(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn make_extractor() -> Option<PluginExtractor> {
    let wasm = workspace_relative(EXAMPLE_WASM);
    if !wasm.exists() {
        eprintln!(
            "skipping: example-extractor wasm not built at {}",
            wasm.display()
        );
        return None;
    }
    let bytes = std::fs::read(&wasm).expect("read wasm");
    let engine = Arc::new(Engine::new(EngineConfig::default()).expect("engine"));
    let component =
        wasmtime::component::Component::from_binary(engine.raw(), &bytes).expect("component");

    let toml = r#"
name = "trap-test"
version = "0.0.1"
wit_version = "0.4.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = []

[signature]
type = "ed25519"
pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
signature = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#;
    let manifest = parse_manifest_str(toml).expect("manifest");
    let identity = manifest.signature.identity_string();
    let loaded = LoadedPlugin {
        manifest,
        identity,
        component,
        origin_dir: workspace_relative("tests/fixtures"),
    };
    Some(PluginExtractor::new(loaded, engine, HostResources::default()).expect("adapter"))
}

#[test]
fn third_trap_disables_plugin() {
    let Some(ext) = make_extractor() else {
        return;
    };
    assert!(!ext.test_is_disabled());
    assert_eq!(ext.test_trap_count(), 0);
    ext.test_record_trap();
    assert!(!ext.test_is_disabled(), "1 strike does not disable");
    ext.test_record_trap();
    assert!(!ext.test_is_disabled(), "2 strikes does not disable");
    ext.test_record_trap();
    assert!(ext.test_is_disabled(), "3 strikes MUST disable the adapter");
    assert_eq!(ext.test_trap_count(), 3);
}

#[test]
fn additional_traps_after_disable_are_idempotent() {
    let Some(ext) = make_extractor() else {
        return;
    };
    for _ in 0..5 {
        ext.test_record_trap();
    }
    assert!(ext.test_is_disabled());
    // Counter keeps climbing — no overflow guard needed for AtomicU32 in
    // the lifetime of any sane process — but the disabled flag must stay
    // latched, not flicker.
    assert!(ext.test_is_disabled());
}
