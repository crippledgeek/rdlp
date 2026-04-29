//! Per-call wasmtime instance creation with timeout + cancellation enforcement.
//!
//! Each plugin invocation gets a fresh `Store<PluginStoreData>` so plugin
//! state cannot leak across calls. The store is configured with:
//!
//! - `StoreLimits` — memory + table caps from [`crate::engine::EngineConfig`]
//! - `set_epoch_deadline` — N ticks where N is the desired wall-clock deadline
//!   divided by the engine's tick period (default 100ms)
//! - `epoch_deadline_trap` — on deadline expiry, the plugin traps so the host
//!   regains control deterministically
//!
//! Cancellation is enforced at the call site via `tokio::select!` against a
//! `CancellationToken`.

use crate::PluginError;
use crate::engine::Engine;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};

/// Per-call store data. Each plugin invocation gets a fresh instance.
///
/// Capability host imports are wired by the loader (Task 23) when the linker
/// is constructed; this struct holds the per-plugin context those impls need.
/// Most fields are `Option<...>` because not every plugin requests every
/// capability.
pub struct PluginStoreData {
    /// Resource limiter for memory + table caps.
    pub limits: StoreLimits,
    /// Plugin name (used as `log` target and in error messages).
    pub plugin_name: String,
    /// Cancellation token threaded into all I/O capabilities.
    pub cancel: CancellationToken,
    /// `log` target — `format!("plugin::{plugin_name}")`.
    pub log_target: String,
    // Capability contexts populated by the loader at instance build time.
    // Filled in by Tasks 12-17.
    // pub fetch:        Option<crate::host::fetch::FetchCtx>,
    // pub cookie_jar:   Option<crate::host::cookie_jar::CookieJarCtx>,
    // pub js_eval:      Option<crate::host::js_eval::JsEvalCtx>,
    // pub store_kv:     Option<crate::host::store_kv::StoreKvCtx>,
    /// Granted `host:html-select` capability state, or `None` if not requested.
    pub html_select: Option<crate::host::html_select::HtmlSelectCtx>,
}

impl PluginStoreData {
    /// Build store data with the given plugin name + cancellation token.
    /// Capability fields default to `None` — the loader populates the ones it
    /// granted.
    #[must_use]
    pub fn new(plugin_name: impl Into<String>, cancel: CancellationToken) -> Self {
        let plugin_name = plugin_name.into();
        let log_target = format!("plugin::{plugin_name}");
        let limits = StoreLimitsBuilder::new()
            .memory_size(64 * 1024 * 1024)
            .table_elements(10_000)
            .instances(1)
            .tables(10)
            .memories(1)
            .build();
        Self {
            limits,
            plugin_name,
            cancel,
            log_target,
            html_select: None,
        }
    }
}

/// Build a fresh store with the given deadline (expressed as engine epoch ticks).
///
/// Wires the limiter, sets the epoch deadline, and configures
/// `epoch_deadline_trap` so the plugin traps deterministically when the
/// deadline elapses. This is preferable to `epoch_deadline_async_yield_and_update`
/// for timeout enforcement because the trap is unconditional — the yield variant
/// requires the host to actively stop resuming the future.
pub fn build_store(
    engine: &Engine,
    plugin_name: impl Into<String>,
    cancel: CancellationToken,
    deadline_ticks: u64,
) -> Store<PluginStoreData> {
    let data = PluginStoreData::new(plugin_name, cancel);
    let mut store = Store::new(engine.raw(), data);
    store.limiter(|d| &mut d.limits);
    store.set_epoch_deadline(deadline_ticks);
    store.epoch_deadline_trap();
    store
}

/// Convert a wall-clock duration into engine epoch ticks (rounded up, min 1).
#[must_use]
pub fn deadline_ticks(deadline: Duration, tick_period: Duration) -> u64 {
    let denom = tick_period.as_millis().max(1) as u64;
    let total = deadline.as_millis() as u64;
    (total / denom).max(1)
}

/// Race `fut` against `cancel.cancelled()`. Returns `Err(PluginError::Cancelled)`
/// when cancellation wins.
pub async fn run_with_cancel<F, T>(
    plugin_name: &str,
    cancel: &CancellationToken,
    fut: F,
) -> Result<T, PluginError>
where
    F: std::future::Future<Output = Result<T, PluginError>>,
{
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(PluginError::Cancelled { plugin: plugin_name.to_string() }),
        result = fut => result,
    }
}
