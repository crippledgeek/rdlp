//! `host:js-eval` capability — bridges plugin JS evaluation to rdlp's `boa`-backed
//! `rdlp-jsinterp` crate.
//!
//! Each `eval` call creates a fresh Boa `Context` (via `BoaJsEngine`) with the
//! sandbox globals injected as top-level JS variables. Results are serialised to
//! JSON strings.
//!
//! # Source size cap
//!
//! `JsEvalCtx::eval` rejects sources larger than [`JsEvalCtx::SOURCE_SIZE_LIMIT`]
//! (512 KiB) before the Boa context is even constructed. This caps the initial
//! parse + compile work and prevents a plugin from submitting a multi-megabyte
//! source to degrade the host.
//!
//! # Iteration / recursion / memory caps
//!
//! `BoaJsEngine::make_context` sets `runtime_limits_mut().set_loop_iteration_limit`
//! (10M iterations) and `set_recursion_limit` (256 frames) at every context
//! construction. These are the host-side guard against pure-CPU JS DoS:
//! infinite `while(true)` loops and unbounded recursion both terminate with a
//! `RuntimeLimit` error that **cannot be caught from JS** (boa documents this;
//! `try/catch` does not intercept it). This is the strongest guard available
//! without wall-clock interruption — `tokio::time::timeout` cannot preempt a
//! `spawn_blocking` thread executing a tight loop.
//!
//! `JsEvalCtx.timeout` and `memory_cap` remain as documentation of the
//! intended wall-clock / heap envelope; they are NOT independently enforced.
//! For wall-clock cancellation across the full plugin call (not just JS),
//! the wasmtime epoch deadline set per-call in `instance::build_store` is
//! the authoritative mechanism — a plugin that legitimately yields back from
//! `host:js-eval` will trap on the next instruction once the epoch fires.
//!
//! # Capability denial
//!
//! `eval` returns `Err("js-eval capability not granted")` when `PluginStoreData.js_eval`
//! is `None`, i.e. the plugin did not declare the capability in its manifest.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::missing_errors_doc, clippy::doc_markdown)]

use crate::instance::PluginStoreData;
use rdlp_core::JsEngine as _;
use rdlp_jsinterp::BoaJsEngine;
use std::time::Duration;
use wasmtime::component::Linker;

/// Per-plugin js-eval context.
///
/// The fields are advisory: pure-CPU DoS is bounded by the iteration and
/// recursion limits applied to every Boa context inside `rdlp-jsinterp`
/// (see module-level doc). These fields document the intended wall-clock
/// and heap envelope for plugin authors; the wasmtime epoch deadline
/// enforces wall-clock at the WASM level once `host:js-eval` returns.
#[derive(Debug, Clone)]
pub struct JsEvalCtx {
    /// Documented wall-clock cap on a single eval call. Default 5 seconds.
    /// Not enforced inside the boa `spawn_blocking` thread; pure-CPU loops
    /// are bounded by `runtime_limits_mut().set_loop_iteration_limit` instead.
    pub timeout: Duration,
    /// Documented memory cap inside the boa context. Default 32 MB.
    /// Not enforced by Boa's public API.
    pub memory_cap: usize,
}

impl Default for JsEvalCtx {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            memory_cap: 32 * 1024 * 1024,
        }
    }
}

impl JsEvalCtx {
    /// Maximum byte length of a JS source string accepted by `eval`.
    ///
    /// Sources larger than this are rejected before the Boa context is even
    /// constructed. This caps the initial parse + compile work and prevents a
    /// plugin from submitting a multi-megabyte source to degrade the host.
    pub const SOURCE_SIZE_LIMIT: usize = 512 * 1024;

    /// Evaluate `source` with the given sandbox globals (key/value string pairs
    /// inserted into the global object before execution). Returns the script's
    /// completion value serialised as a JSON string, or an error message.
    ///
    /// `sandbox_globals` is a slice of `(key, value)` pairs where each `value`
    /// is a JSON-serialisable string. The engine injects them as top-level
    /// variables (string type) before running `source`.
    ///
    /// Returns `Err` immediately when `source` exceeds [`Self::SOURCE_SIZE_LIMIT`]
    /// (512 KiB) without performing any JS evaluation.
    pub async fn eval(
        &self,
        sandbox_globals: &[(String, String)],
        source: &str,
    ) -> Result<String, String> {
        if source.len() > Self::SOURCE_SIZE_LIMIT {
            return Err(format!(
                "js-eval source too large: {} bytes exceeds limit of {} bytes",
                source.len(),
                Self::SOURCE_SIZE_LIMIT
            ));
        }

        let engine = BoaJsEngine::new();

        // Build a JSON object from the string key-value pairs so we can use
        // eval_with_context, which injects each top-level key as a global.
        let ctx_json: serde_json::Value = {
            let map: serde_json::Map<String, serde_json::Value> = sandbox_globals
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            serde_json::Value::Object(map)
        };

        let json_result = engine
            .eval_with_context(source, &ctx_json)
            .await
            .map_err(|e| e.to_string())?;

        Ok(json_result.to_string())
    }
}

/// Wire `host:js-eval` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_js_eval::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_js_eval::Host for PluginStoreData {
    async fn eval(
        &mut self,
        ctx: crate::bindings::rdlp::plugin::host_js_eval::Context,
        source: String,
    ) -> Result<String, String> {
        // source_url is unused in MVP — reserved for future stack-trace enrichment.
        let _ = ctx.source_url;

        let Some(js_ctx) = self.js_eval.as_ref() else {
            return Err("js-eval capability not granted".into());
        };

        js_ctx.eval(&ctx.sandbox_globals, &source).await
    }
}
