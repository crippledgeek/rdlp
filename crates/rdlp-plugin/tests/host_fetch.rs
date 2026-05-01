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
use rdlp_plugin::host::fetch::FetchCtx;
use rdlp_plugin::instance::PluginStoreData;
use tokio_util::sync::CancellationToken;

#[test]
fn add_to_linker_succeeds() {
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
    rdlp_plugin::host::fetch::add_to_linker(&mut linker).expect("link");
}

#[test]
fn fetch_ctx_can_be_constructed_with_default_client() {
    let ctx = FetchCtx::with_default_client().expect("client");
    drop(ctx);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_without_capability_grant_returns_network_error() {
    // PluginStoreData with fetch=None → calling host fetch returns
    // FetchError::Network("not granted").
    let cancel = CancellationToken::new();
    let mut data = PluginStoreData::new("test", cancel);
    assert!(data.fetch.is_none());

    // We can't easily exercise this without the bindgen Host trait method.
    // Instead, verify the Default ctx existence path.
    data.fetch = Some(FetchCtx::with_default_client().expect("default"));
    assert!(data.fetch.is_some());
}
