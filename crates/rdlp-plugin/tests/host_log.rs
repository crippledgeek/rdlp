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

use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::instance::{PluginStoreData, build_store};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Smoke tests — verify the linker wiring compiles and links without a full
// component round-trip. The end-to-end "plugin calls log" scenario runs in
// Task 28's reference plugin.
// ---------------------------------------------------------------------------

fn default_engine() -> Engine {
    Engine::new(EngineConfig::default()).expect("engine")
}

#[test]
fn add_to_linker_succeeds() {
    let engine = default_engine();
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    rdlp_plugin::host::log::add_to_linker(&mut linker).expect("add log to linker");
}

#[test]
fn add_to_linker_is_idempotent_across_separate_linkers() {
    // Two separate linkers — both should succeed independently.
    let engine = default_engine();
    let mut l1 = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    let mut l2 = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    rdlp_plugin::host::log::add_to_linker(&mut l1).expect("linker 1");
    rdlp_plugin::host::log::add_to_linker(&mut l2).expect("linker 2");
}

// ---------------------------------------------------------------------------
// Host-trait unit tests — exercise the Host impl on PluginStoreData directly.
//
// The `log` crate facade is used; without a global logger installed the calls
// are silently dropped, but the code path (including the match arms for all 5
// levels) is exercised and any panic / type error would surface here.
// ---------------------------------------------------------------------------

#[test]
fn host_impl_all_levels_do_not_panic() {
    use rdlp_plugin::bindings::rdlp::plugin::host_log::{Host, Level};

    let cancel = CancellationToken::new();
    let mut store = build_store(&default_engine(), "test-plugin", cancel, 10);
    let data = store.data_mut();

    // Call the trait method directly for every level to verify all match arms.
    // `host-log` is a synchronous import (nothing to await — see the bindgen
    // `only_imports` list in lib.rs), so no runtime is needed. No side effect
    // is visible here: the log crate drops calls when no subscriber is installed.
    for level in [
        Level::Trace,
        Level::Debug,
        Level::Info,
        Level::Warn,
        Level::Error,
    ] {
        data.log(level, "test message".to_string());
    }
}

#[test]
fn log_target_format_is_correct() {
    // Verify the log target stored in PluginStoreData matches the expected
    // "plugin::{name}" format that Host::log uses as the log target.
    let cancel = CancellationToken::new();
    let store = build_store(&default_engine(), "my-extractor", cancel, 10);
    assert_eq!(store.data().log_target, "plugin::my-extractor");
}
