use rdlp_plugin::engine::{Engine, EngineConfig};
use std::time::Duration;

#[test]
fn engine_creates_with_default_config() {
    let engine = Engine::new(EngineConfig::default()).expect("engine new");
    drop(engine);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn epoch_ticks_advance_over_time() {
    let engine = Engine::new(EngineConfig {
        tick_period: Duration::from_millis(50),
        ..Default::default()
    })
    .expect("engine new");
    let start = engine.current_epoch();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let later = engine.current_epoch();
    assert!(
        later >= start + 4,
        "expected at least 4 ticks in 300ms; got {start}->{later}"
    );
    drop(engine);
}

#[test]
fn engine_drop_stops_tick_thread() {
    let engine = Engine::new(EngineConfig {
        tick_period: Duration::from_millis(20),
        ..Default::default()
    })
    .expect("engine new");
    std::thread::sleep(Duration::from_millis(60));
    let mid = engine.current_epoch();
    assert!(mid > 0);
    drop(engine);
    // After drop, the tick thread should stop quickly. We can't observe the
    // dropped engine, but no panics is the assertion.
}

#[test]
fn engine_default_caps_match_spec() {
    let cfg = EngineConfig::default();
    assert_eq!(cfg.max_memory_bytes, 64 * 1024 * 1024);
    assert_eq!(cfg.max_stack_bytes, 1024 * 1024);
    assert_eq!(cfg.tick_period, Duration::from_millis(100));
}
