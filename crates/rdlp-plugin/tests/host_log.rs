use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::instance::{build_store, PluginStoreData};
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
    // The futures are driven to completion synchronously because the Host impl
    // is synchronous in its side-effects (log macro calls); the async wrapper
    // is imposed by bindgen.
    for level in [Level::Trace, Level::Debug, Level::Info, Level::Warn, Level::Error] {
        let fut = data.log(level, "test message".to_string());
        // Drive the future — it should complete instantly with no side effects
        // visible here (the log crate drops calls when no subscriber is installed).
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(fut);
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
