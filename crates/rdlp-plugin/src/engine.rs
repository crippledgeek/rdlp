//! Wasmtime engine wrapper with epoch-tick thread for timeout enforcement.
//!
//! The host's `Engine` owns a `wasmtime::Engine` (with epoch interruption,
//! component model, and async support enabled) plus a single background
//! thread that increments the engine's epoch counter at a fixed period
//! (default 100ms). Per-call deadlines are expressed in epoch ticks via
//! `Store::set_epoch_deadline()` — when the deadline is reached, the plugin
//! yields to the host (via `epoch_deadline_async_yield_and_update`).
//!
//! See design spec §10–11 and tracking issue #213.

use crate::PluginError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Engine configuration. All fields are tunable from the rdlp `Config`.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Wall-clock period at which the engine's epoch counter increments.
    /// Smaller = more responsive cancellation; larger = lower CPU overhead.
    /// Default 100ms.
    pub tick_period: Duration,

    /// Maximum WASM linear-memory bytes per plugin instance. Default 64 MB.
    pub max_memory_bytes: usize,

    /// Maximum WASM call-stack bytes per plugin instance. Default 1 MB.
    pub max_stack_bytes: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            tick_period: Duration::from_millis(100),
            max_memory_bytes: 64 * 1024 * 1024,
            max_stack_bytes: 1024 * 1024,
        }
    }
}

/// Host-side engine handle. The wrapped `wasmtime::Engine` is `Clone` cheaply
/// (it's an `Arc` internally), so callers can clone it freely.
pub struct Engine {
    inner: wasmtime::Engine,
    epoch: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    _tick_thread: Option<thread::JoinHandle<()>>,
}

impl Engine {
    /// Construct a new engine with the given configuration.
    ///
    /// Spawns a background thread that ticks the engine's epoch every
    /// `cfg.tick_period` using absolute-deadline scheduling
    /// (`thread::park_timeout` against successive `Instant` targets) so
    /// individual sleep overshoots under CI/scheduler contention don't
    /// accumulate as drift across iterations. On drop, the shutdown flag is
    /// set and the tick thread is unparked for prompt termination.
    pub fn new(cfg: EngineConfig) -> Result<Self, PluginError> {
        let mut config = wasmtime::Config::new();
        config
            .async_support(true)
            .epoch_interruption(true)
            .wasm_component_model(true)
            .max_wasm_stack(cfg.max_stack_bytes);

        let inner = wasmtime::Engine::new(&config)
            .map_err(|e| PluginError::Internal(format!("wasmtime engine init: {e}")))?;

        let epoch = Arc::new(AtomicU64::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        let tick_thread = {
            let engine = inner.clone();
            let epoch = epoch.clone();
            let shutdown = shutdown.clone();
            let period = cfg.tick_period;
            thread::Builder::new()
                .name("rdlp-plugin-epoch".into())
                .spawn(move || {
                    // Anchor cadence to absolute Instants so a slow scheduler
                    // wakeup in one cycle doesn't push subsequent targets back.
                    let mut next = Instant::now() + period;
                    while !shutdown.load(Ordering::Relaxed) {
                        let now = Instant::now();
                        if let Some(remaining) = next.checked_duration_since(now) {
                            // park_timeout returns early if Drop unparks us.
                            thread::park_timeout(remaining);
                            if shutdown.load(Ordering::Relaxed) {
                                break;
                            }
                            // Spurious wakeup: if we haven't reached the
                            // deadline yet, loop and re-park. checked_duration_since
                            // returns None once `now >= next`, falling through
                            // to the tick.
                            if Instant::now() < next {
                                continue;
                            }
                        }
                        engine.increment_epoch();
                        epoch.fetch_add(1, Ordering::Relaxed);
                        // Compute the next deadline relative to the previous
                        // target rather than `now`, so a single late wakeup
                        // doesn't permanently shift the cadence. If we're
                        // far enough behind that the next deadline is already
                        // in the past (CI under heavy load), skip ahead to
                        // avoid a tight catch-up loop.
                        next += period;
                        let now = Instant::now();
                        if next < now {
                            next = now + period;
                        }
                    }
                })
                .map_err(|e| PluginError::Internal(format!("spawn epoch tick thread: {e}")))?
        };

        Ok(Self {
            inner,
            epoch,
            shutdown,
            _tick_thread: Some(tick_thread),
        })
    }

    /// Borrow the underlying `wasmtime::Engine` for `Store` / `Component` ops.
    #[must_use]
    pub fn raw(&self) -> &wasmtime::Engine {
        &self.inner
    }

    /// Current observed epoch tick count (host-side counter). Useful for
    /// instrumentation / tests; not the same as `wasmtime::Engine`'s internal
    /// counter, but increments in lock-step with it.
    #[must_use]
    pub fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Unpark the tick thread so it observes `shutdown` immediately
        // instead of sleeping out the rest of its current `park_timeout`.
        // We don't join — a panicked tick thread shouldn't take Drop down,
        // and the OS will reap it after the loop exits.
        if let Some(handle) = self._tick_thread.as_ref() {
            handle.thread().unpark();
        }
    }
}
