//! `host:log` capability — bridges plugin log calls to the host's `log` crate.
//!
//! When a plugin calls `rdlp:plugin/host-log.log(level, message)`, the call is
//! forwarded to the `log` crate with target `plugin::{plugin_name}`, allowing
//! consumers to filter plugin messages via normal log filters.

use crate::instance::PluginStoreData;
use wasmtime::component::Linker;

/// Wire `rdlp:plugin/host-log` into a component linker.
///
/// After this call, plugins that import `host-log` can call `log(level, message)`.
/// Messages are forwarded to the `log` crate with target `plugin::{plugin_name}`.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_log::add_to_linker(linker, |state| state)
}

impl crate::bindings::rdlp::plugin::host_log::Host for PluginStoreData {
    async fn log(
        &mut self,
        level: crate::bindings::rdlp::plugin::host_log::Level,
        message: String,
    ) {
        use crate::bindings::rdlp::plugin::host_log::Level as L;
        let target = self.log_target.as_str();
        match level {
            L::Trace => log::trace!(target: target, "{message}"),
            L::Debug => log::debug!(target: target, "{message}"),
            L::Info => log::info!(target: target, "{message}"),
            L::Warn => log::warn!(target: target, "{message}"),
            L::Error => log::error!(target: target, "{message}"),
        }
    }
}
