//! `host:js-eval` capability — bridges plugin JS evaluation to rdlp's `boa`-backed
//! `rdlp-jsinterp` crate.
//!
//! Each `eval` call creates a fresh Boa `Context` (via `BoaJsEngine`) with the
//! sandbox globals injected as top-level JS variables. Results are serialised to
//! JSON strings.
//!
//! # Timeout / memory caps
//!
//! `JsEvalCtx` carries `timeout` and `memory_cap` fields that document the
//! intended limits, but Boa's interruption API is not yet wired through the
//! `rdlp-jsinterp` public surface. The wasmtime epoch deadline (set per-call in
//! `instance::build_store`) provides a coarser cap: if a plugin spins inside
//! `eval()` the WASM instance will trap when the epoch deadline fires — but only
//! after the boa `spawn_blocking` thread completes or is abandoned. This is a
//! known limitation of the MVP; a future sprint should wire Boa's
//! `Context::set_max_call_stack_size` and the interrupt callback to honour the
//! wall-clock deadline inside the blocking thread.
//!
//! # Capability denial
//!
//! `eval` returns `Err("js-eval capability not granted")` when `PluginStoreData.js_eval`
//! is `None`, i.e. the plugin did not declare the capability in its manifest.

use crate::instance::PluginStoreData;
use rdlp_core::JsEngine as _;
use rdlp_jsinterp::BoaJsEngine;
use std::time::Duration;
use wasmtime::component::Linker;

/// Per-plugin js-eval context. Carries wall-clock + memory caps for documentation
/// and future enforcement. See module-level doc for current limitation.
#[derive(Debug, Clone)]
pub struct JsEvalCtx {
    /// Wall-clock cap on a single eval call. Default 5 seconds.
    /// Not yet enforced inside the boa `spawn_blocking` thread.
    pub timeout: Duration,
    /// Memory cap inside the boa context. Default 32 MB.
    /// Not yet enforced by Boa's public API.
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
    /// Evaluate `source` with the given sandbox globals (key/value string pairs
    /// inserted into the global object before execution). Returns the script's
    /// completion value serialised as a JSON string, or an error message.
    ///
    /// `sandbox_globals` is a slice of `(key, value)` pairs where each `value`
    /// is a JSON-serialisable string. The engine injects them as top-level
    /// variables (string type) before running `source`.
    pub async fn eval(
        &self,
        sandbox_globals: &[(String, String)],
        source: &str,
    ) -> Result<String, String> {
        let engine = BoaJsEngine::new();

        // Build a JSON object from the string key-value pairs so we can use
        // eval_with_context, which injects each top-level key as a global.
        let ctx_json: serde_json::Value = {
            let mut map = serde_json::Map::with_capacity(sandbox_globals.len());
            for (k, v) in sandbox_globals {
                map.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
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
