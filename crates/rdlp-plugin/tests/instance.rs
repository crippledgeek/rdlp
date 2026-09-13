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

use rdlp_plugin::PluginError;
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::instance::{build_store, deadline_ticks, run_with_cancel};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const SPIN_FOREVER_WAT: &str = include_str!("fixtures/spin_forever.wat");

fn engine_for_tests() -> Engine {
    Engine::new(EngineConfig {
        tick_period: Duration::from_millis(50),
        ..Default::default()
    })
    .expect("engine new")
}

#[test]
fn deadline_ticks_round_up() {
    assert_eq!(
        deadline_ticks(Duration::from_millis(100), Duration::from_millis(100)),
        1
    );
    assert_eq!(
        deadline_ticks(Duration::from_millis(250), Duration::from_millis(100)),
        2
    );
    assert_eq!(
        deadline_ticks(Duration::from_millis(0), Duration::from_millis(100)),
        1
    );
    assert_eq!(
        deadline_ticks(Duration::from_secs(30), Duration::from_millis(100)),
        300
    );
}

#[test]
fn build_store_sets_plugin_name() {
    let engine = engine_for_tests();
    let cancel = CancellationToken::new();
    let store = build_store(&engine, "youtube", cancel, 10);
    assert_eq!(store.data().plugin_name, "youtube");
    assert_eq!(store.data().log_target, "plugin::youtube");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_with_cancel_returns_cancelled_when_token_fires() {
    let cancel = CancellationToken::new();
    let cancel2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel2.cancel();
    });
    let result: Result<(), PluginError> = run_with_cancel("test", &cancel, async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok(())
    })
    .await;
    assert!(matches!(result, Err(PluginError::Cancelled { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_with_cancel_returns_inner_result_when_no_cancel() {
    let cancel = CancellationToken::new();
    let result: Result<i32, PluginError> = run_with_cancel("test", &cancel, async { Ok(42) }).await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spinning_component_traps_on_epoch_deadline() {
    let engine = engine_for_tests();
    let component_bytes = wat::parse_str(SPIN_FOREVER_WAT).expect("wat parse");
    let component = wasmtime::component::Component::new(engine.raw(), &component_bytes)
        .expect("component compile");

    let cancel = CancellationToken::new();
    // 4 ticks * 50ms = 200ms deadline
    let mut store = build_store(&engine, "spin", cancel.clone(), 4);

    let linker =
        wasmtime::component::Linker::<rdlp_plugin::instance::PluginStoreData>::new(engine.raw());

    // Instantiate and call the exported `infinite` function via the raw
    // component API. We expect a trap (epoch deadline reached) within ~200ms.
    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("instantiate");

    let func = instance
        .get_func(&mut store, "infinite")
        .expect("export found");
    let typed = func.typed::<(), ()>(&store).expect("typed");

    let start = std::time::Instant::now();
    let result = typed.call_async(&mut store, ()).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "expected trap from epoch deadline");
    assert!(
        elapsed < Duration::from_secs(2),
        "should have trapped quickly; took {elapsed:?}"
    );
}
