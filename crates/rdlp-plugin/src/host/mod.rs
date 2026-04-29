//! Host capability implementations bridged to rdlp services.

pub mod cookie_jar;
pub mod fetch;
pub mod html_select;
pub mod js_eval;
pub mod log;
pub mod store_kv;

use crate::PluginError;
use crate::instance::PluginStoreData;
use crate::manifest::Manifest;
use std::collections::BTreeSet;
use wasmtime::component::Linker;

/// Wire host capabilities into `linker` based on the plugin manifest's
/// declared `capabilities` list.
///
/// A plugin that declares only `["fetch", "log"]` will have ONLY those host
/// imports linked. If the plugin's `.wasm` imports an interface we did not
/// link, `Linker::instantiate_async` will fail at instantiation time. That
/// is the structural capability-denial enforcement (vector A1 + general
/// principle).
///
/// `claim-all-urls` is a manifest-level gating capability (it controls
/// whether TLD-wildcard match patterns are accepted in [`Manifest::matches`])
/// and corresponds to no host import — so it is silently ignored here.
pub fn add_capability_imports(
    linker: &mut Linker<PluginStoreData>,
    manifest: &Manifest,
) -> Result<(), PluginError> {
    let caps: BTreeSet<&str> = manifest.capabilities.iter().map(String::as_str).collect();

    if caps.contains("log") {
        log::add_to_linker(linker).map_err(|e| PluginError::Internal(format!("link log: {e}")))?;
    }
    if caps.contains("html-select") {
        html_select::add_to_linker(linker)
            .map_err(|e| PluginError::Internal(format!("link html-select: {e}")))?;
    }
    if caps.contains("fetch") {
        fetch::add_to_linker(linker)
            .map_err(|e| PluginError::Internal(format!("link fetch: {e}")))?;
    }
    if caps.contains("cookie-jar") {
        cookie_jar::add_to_linker(linker)
            .map_err(|e| PluginError::Internal(format!("link cookie-jar: {e}")))?;
    }
    if caps.contains("js-eval") {
        js_eval::add_to_linker(linker)
            .map_err(|e| PluginError::Internal(format!("link js-eval: {e}")))?;
    }
    if caps.contains("store-kv") {
        store_kv::add_to_linker(linker)
            .map_err(|e| PluginError::Internal(format!("link store-kv: {e}")))?;
    }
    // `claim-all-urls` is gating-only — no linker action.
    Ok(())
}
