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

use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::host::add_capability_imports;
use rdlp_plugin::instance::PluginStoreData;
use rdlp_plugin::manifest::Manifest;
use rdlp_plugin::manifest::parse_manifest_str;

fn manifest_with_caps(caps: &[&str]) -> Manifest {
    let cap_list = caps
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        r#"
name = "test"
version = "1.0.0"
wit_version = "0.1.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = [{cap_list}]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#,
    );
    parse_manifest_str(&toml).expect("manifest parse")
}

#[test]
fn empty_capabilities_links_nothing() {
    // Note: can't have truly-empty capabilities because TLD-wildcard matches
    // need claim-all-urls. Use a single innocuous cap to keep manifest valid.
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let m = manifest_with_caps(&["log"]);
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    add_capability_imports(&mut linker, &m).expect("add caps");
}

#[test]
fn all_capabilities_link_successfully() {
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let m = manifest_with_caps(&[
        "fetch",
        "cookie-jar",
        "js-eval",
        "html-select",
        "log",
        "store-kv",
    ]);
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    add_capability_imports(&mut linker, &m).expect("add all caps");
}

#[test]
fn claim_all_urls_capability_is_gating_only_no_linker_op() {
    // claim-all-urls is a manifest-level flag (Task 3 validation), not a
    // host import. Linker wiring must succeed even when only claim-all-urls
    // is declared (alongside another cap to keep the manifest valid).
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let m = manifest_with_caps(&["log", "claim-all-urls"]);
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    add_capability_imports(&mut linker, &m).expect("add caps");
}

#[test]
fn fetch_capability_alone_links() {
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let m = manifest_with_caps(&["fetch"]);
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    add_capability_imports(&mut linker, &m).expect("add fetch only");
}
