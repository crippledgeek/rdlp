//! Shared WASM-component test harness for the by-name-export unit tests in
//! `search_adapter`, `playlist_adapter`, and `metadata_adapter`. Extracted
//! because all three needed a byte-identical `instantiate(wat)`: build an
//! engine, compile the WAT, build a bare-linker store, instantiate — and,
//! for the hand-lifted WIT types, the same `types.wit` body scan
//! ([`wit_body_lines`]) that each drift guard used to carry inline.

#![cfg(test)]

use std::time::Duration;

use crate::instance::PluginStoreData;

/// The WIT source the hand-lifted records/variants are pinned against.
pub const TYPES_WIT: &str = include_str!("../wit/types.wit");

/// A component exporting nothing — what a by-name lookup of any post-0.5.0
/// export finds on a plugin that predates it.
pub const EMPTY_COMPONENT_WAT: &str = "(component)";

/// The field (or case) lines of `kind name { … }` in `wit/types.wit`,
/// trimmed, blank lines dropped, in declaration order — what a
/// hand-lifted type's drift guard compares its expected lines against.
/// wasmtime typechecks a lift by field/case name, type, AND order at
/// `get_typed_func` (`wasmtime::component::func::typed::typecheck_record`
/// / `typecheck_variant`, wasmtime 30.0.2), so a drifted hand-lift would
/// trap, and strike, every plugin at call time; the guards catch it at
/// test time instead. Doc comments inside a body would be returned too,
/// so `types.wit` keeps them above the declaration.
///
/// # Panics
///
/// When `types.wit` declares no `kind name {` or the body is unclosed.
pub fn wit_body_lines(kind: &str, name: &str) -> Vec<&'static str> {
    let opener = format!("{kind} {name} {{");
    let (_, after) = TYPES_WIT
        .split_once(&opener)
        .unwrap_or_else(|| panic!("types.wit declares `{opener}`"));
    let (body, _) = after
        .split_once('}')
        .unwrap_or_else(|| panic!("`{opener}` body is closed"));
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

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
