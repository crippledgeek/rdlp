//! Shared WASM-component test harness for the by-name-export unit tests in
//! `search_adapter`, `playlist_adapter`, and `metadata_adapter`. Extracted
//! because all three needed a byte-identical `instantiate(wat)`: build an
//! engine, compile the WAT, build a bare-linker store, instantiate.

#![cfg(test)]

use std::time::Duration;

use crate::instance::PluginStoreData;

/// Wall-clock deadline for a by-name-export unit test's store.
///
/// Deliberately its own constant rather than borrowing a real call kind's
/// timeout (`SEARCH_TIMEOUT`/`EXTRACT_TIMEOUT`): these tests exercise the
/// lookup/typecheck/trap machinery, not pacing, and only need enough ticks
/// that the epoch deadline can't fire before the in-process call completes.
pub const TEST_DEADLINE: Duration = Duration::from_secs(60);

/// A minimal component exporting only what one test's WAT declares,
/// instantiated on a bare linker (no host world, no capabilities) into a
/// store carrying the unit-test plugin name — the smallest thing a by-name
/// call can be pointed at.
pub async fn instantiate(
    wat: &str,
) -> (
    wasmtime::Store<PluginStoreData>,
    wasmtime::component::Instance,
) {
    use crate::engine::{Engine, EngineConfig};
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let component =
        wasmtime::component::Component::new(engine.raw(), wat::parse_str(wat).expect("wat"))
            .expect("component");
    // The production deadline: an unbounded tick count overflows
    // wasmtime's epoch arithmetic (`store.rs` adds unchecked) once the
    // engine's ticker has advanced.
    let mut store = crate::instance::build_store(
        &engine,
        "test",
        tokio_util::sync::CancellationToken::new(),
        crate::instance::deadline_ticks(TEST_DEADLINE, engine.tick_period()),
    );
    let instance = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw())
        .instantiate_async(&mut store, &component)
        .await
        .expect("instantiate");
    (store, instance)
}
