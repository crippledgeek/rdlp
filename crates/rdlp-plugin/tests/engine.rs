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
use std::time::{Duration, Instant};

#[test]
fn engine_creates_with_default_config() {
    let engine = Engine::new(EngineConfig::default()).expect("engine new");
    drop(engine);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn epoch_ticks_advance_over_time() {
    // Validates the tick thread is actually running by polling for a target
    // tick count rather than sampling at a fixed offset. Under contended CI
    // schedulers (macOS runners in particular) the kernel can delay
    // `park_timeout` wakeups by 100ms+, so a fixed 300ms sample would race
    // even with a 50ms tick period. The deadline window is generous enough
    // that the test can only fail if the tick thread is fundamentally stuck.
    let engine = Engine::new(EngineConfig {
        tick_period: Duration::from_millis(50),
        ..Default::default()
    })
    .expect("engine new");
    let start = engine.current_epoch();

    const TARGET_TICKS: u64 = 4;
    const POLL_INTERVAL: Duration = Duration::from_millis(50);
    const DEADLINE: Duration = Duration::from_secs(5);

    let started = Instant::now();
    let later = loop {
        let now = engine.current_epoch();
        if now >= start + TARGET_TICKS {
            break now;
        }
        if started.elapsed() >= DEADLINE {
            break now;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    };

    assert!(
        later >= start + TARGET_TICKS,
        "expected at least {TARGET_TICKS} ticks within {DEADLINE:?}; got {start}->{later} after {:?}",
        started.elapsed()
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

    // Poll for the tick thread to advance at least once before dropping —
    // again, contention-tolerant rather than relying on a fixed wall-clock
    // sample. After drop the thread is unparked so it observes `shutdown`
    // promptly; we can't observe the dropped engine but the absence of a
    // panic / hang is the assertion.
    let started = Instant::now();
    while engine.current_epoch() == 0 {
        if started.elapsed() >= Duration::from_secs(5) {
            panic!("tick thread never advanced epoch within 5s");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(engine.current_epoch() > 0);
    drop(engine);
}

#[test]
fn engine_default_caps_match_spec() {
    let cfg = EngineConfig::default();
    assert_eq!(cfg.max_memory_bytes, 64 * 1024 * 1024);
    assert_eq!(cfg.max_stack_bytes, 1024 * 1024);
    assert_eq!(cfg.tick_period, Duration::from_millis(100));
}
