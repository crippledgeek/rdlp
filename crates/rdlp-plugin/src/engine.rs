//! Wasmtime engine wrapper with epoch-tick thread for timeout enforcement.
//!
//! The host's `Engine` owns a `wasmtime::Engine` (with epoch interruption,
//! component model, and async support enabled) plus a single background
//! thread that increments the engine's epoch counter at a fixed period
//! (default 100ms). Per-call deadlines are expressed in epoch ticks via
//! `Store::set_epoch_deadline()` — when the deadline is reached, the plugin
//! traps deterministically (via `epoch_deadline_trap`), returning control to
//! the host.
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
    ///
    /// Smaller values give more responsive cancellation at the cost of higher
    /// CPU overhead from the tick thread. Default 100ms.
    pub tick_period: Duration,

    /// Maximum WASM linear-memory bytes per plugin instance. Default 64 MB.
    ///
    /// This is a hard ceiling enforced by `StoreLimits`. Plugins requesting
    /// more memory trap at the `memory.grow` instruction rather than being
    /// silently denied (see `trap_on_grow_failure`).
    pub max_memory_bytes: usize,

    /// Maximum WASM call-stack bytes per plugin instance. Default 1 MB.
    ///
    /// Enforced by `wasmtime::Config::max_wasm_stack`. Exceeding this limit
    /// produces a stack-overflow trap, not a Rust stack overflow.
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

/// Host-side engine handle.
///
/// The wrapped [`wasmtime::Engine`] is reference-counted internally so cloning
/// it is cheap. However this outer handle is NOT `Clone` because it owns the
/// background tick thread and the shutdown flag; only one instance should exist
/// per process (or per isolated test).
///
/// Drop signals the tick thread via the `shutdown` flag and unparks it so it
/// exits promptly rather than sleeping out the remainder of its current tick.
pub struct Engine {
    inner: wasmtime::Engine,
    epoch: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    /// Background thread handle. Retained so `Drop` can unpark the thread;
    /// we intentionally do NOT join it (a panicked tick thread must not
    /// propagate through `Drop`). The `Option` is only `None` before
    /// the first `new()` call completes — always `Some` in live use.
    tick_thread: Option<thread::JoinHandle<()>>,
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
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Internal`] if:
    /// - Wasmtime rejects the compiled `Config` (e.g. invalid `max_wasm_stack`
    ///   alignment or incompatible feature combination on this platform).
    /// - The OS refuses to spawn the background tick thread.
    // clippy::needless_pass_by_value: EngineConfig is not Copy; passing by value
    // matches the builder-pattern idiom (Engine::new(EngineConfig { ... })) and
    // avoids forcing callers to take a borrow reference at the call site.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(cfg: EngineConfig) -> Result<Self, PluginError> {
        let mut config = wasmtime::Config::new();
        config
            .async_support(true)
            // Why: epoch interruption is the mechanism for deadline-based plugin
            // timeouts. The tick thread calls engine.increment_epoch(); the store
            // traps when its deadline tick count is reached.
            .epoch_interruption(true)
            .wasm_component_model(true)
            .max_wasm_stack(cfg.max_stack_bytes)
            // Why: explicit opt-in so a future wasmtime default flip (e.g. to
            // SpeedAndSize) cannot silently change JIT behaviour on upgrade.
            // Source: wasmtime fast-execution docs.
            .cranelift_opt_level(wasmtime::OptLevel::Speed)
            // Why: Cranelift performance recommendation — allows bounds-check
            // elision via signal handlers instead of explicit branch instructions,
            // significantly reducing JIT output size and hot-path latency.
            // Source: https://docs.wasmtime.dev/api/wasmtime/struct.Config.html#method.signals_based_traps
            .signals_based_traps(true)
            // Why: explicit security hardening — disallow shared memory between
            // WASM instances. Our threat model requires per-call isolation;
            // wasm-threads would allow cross-instance state via SharedArrayBuffer.
            // Source: wasmtime security doc.
            .wasm_threads(false)
            // Why: deterministic execution across hosts. relaxed-SIMD allows
            // implementation-defined (non-deterministic) results for some SIMD
            // ops; disabling it ensures identical output on all platforms rdlp
            // runs on (Linux/macOS/Windows with varying CPU feature sets).
            .wasm_relaxed_simd(false)
            // Why: wasm-reference-types is required by the Component Model's
            // GC type system (anyref, externref). Explicitly enabling it prevents
            // a future wasmtime default-off flip from breaking component loading.
            .wasm_reference_types(true)
            // Why: speeds up first-load compilation on multi-core hosts by
            // parallelising Cranelift's module compilation. Safe: compilation is
            // deterministic regardless of thread count.
            .parallel_compilation(true);

        // Why: a 4 GiB virtual-address reservation on 64-bit hosts allows
        // Cranelift to elide all explicit bounds checks for 32-bit WASM linear
        // memories — accesses that would be OOB trap via SIGSEGV/SIGBUS on the
        // guard page instead of a conditional branch on every load/store.
        // Source: https://github.com/bytecodealliance/wasmtime/blob/main/docs/examples-fast-execution.md
        // Gated on 64-bit: 32-bit hosts cannot map 4 GiB of virtual address space.
        if cfg!(target_pointer_width = "64") {
            config.memory_reservation(1u64 << 32);
        }

        // Why: 32 MiB guard pages catch accesses that slip just past the end of
        // the reservation (e.g. wasm loads with a large constant offset). A
        // smaller guard risks missing those accesses if Cranelift's offset-folding
        // puts them beyond the page boundary.
        // Source: wasmtime fast-execution doc, same as above.
        config.memory_guard_size(32 * 1024 * 1024);

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
            tick_thread: Some(tick_thread),
        })
    }

    /// Borrow the underlying [`wasmtime::Engine`] for `Store` / `Component` construction.
    ///
    /// The returned reference is valid for the lifetime of this `Engine` handle.
    /// Do not hold it across an `Engine::drop`.
    #[must_use]
    // clippy::missing_const_for_fn: wasmtime::Engine is not a const-constructible
    // type; the lint suggestion is misleading here.
    #[allow(clippy::missing_const_for_fn)]
    pub fn raw(&self) -> &wasmtime::Engine {
        &self.inner
    }

    /// Current observed epoch tick count (host-side counter).
    ///
    /// Useful for instrumentation and tests. This counter increments in
    /// lock-step with the wasmtime engine's internal epoch, but is maintained
    /// independently via an `AtomicU64` so callers don't need access to the
    /// internal wasmtime epoch representation.
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
        if let Some(handle) = self.tick_thread.as_ref() {
            handle.thread().unpark();
        }
    }
}
