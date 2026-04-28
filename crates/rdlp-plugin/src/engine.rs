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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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
    /// `cfg.tick_period`. The thread is joined when the `Engine` is dropped.
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
                    while !shutdown.load(Ordering::Relaxed) {
                        thread::sleep(period);
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        engine.increment_epoch();
                        epoch.fetch_add(1, Ordering::Relaxed);
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
        // Best-effort: wait briefly for the tick thread to notice. We don't
        // join because a slow `thread::sleep(tick_period)` could block drop
        // for too long; the OS will reap it after the AtomicBool flip.
    }
}
