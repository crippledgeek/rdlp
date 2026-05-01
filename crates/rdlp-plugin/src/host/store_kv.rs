//! `host:store-kv` capability — per-plugin persistent key/value store backed
//! by sled. Each plugin gets its own namespaced sled tree (isolated from
//! every other plugin) with a 10 MB quota.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::too_long_first_doc_paragraph,
    clippy::missing_errors_doc
)]

use crate::PluginError;
use crate::instance::PluginStoreData;
use wasmtime::component::Linker;

/// Default per-plugin quota: 10 MB.
pub const DEFAULT_QUOTA_BYTES: u64 = 10 * 1024 * 1024;

/// Open the host-level sled DB used to namespace each plugin's `host:store-kv`
/// space. The orchestrator opens this once at bootstrap; per-plugin
/// `StoreKvCtx` instances later carve out namespaced trees from it.
pub fn open_host_db(path: &std::path::Path) -> Result<sled::Db, PluginError> {
    sled::open(path).map_err(|e| PluginError::Internal(format!("sled open: {e}")))
}

/// Per-plugin store-kv context.
pub struct StoreKvCtx {
    /// The namespaced sled tree for this plugin.
    pub tree: sled::Tree,
    /// Plugin name used in error messages.
    pub plugin_name: String,
    /// Per-plugin storage quota in bytes.
    pub quota_bytes: u64,
}

impl StoreKvCtx {
    /// Open the namespaced tree for this plugin in the given sled DB.
    pub fn open(db: &sled::Db, plugin_name: &str) -> Result<Self, PluginError> {
        let tree = db
            .open_tree(format!("plugin::{plugin_name}"))
            .map_err(|e| PluginError::Internal(format!("sled open_tree: {e}")))?;
        Ok(Self {
            tree,
            plugin_name: plugin_name.to_string(),
            quota_bytes: DEFAULT_QUOTA_BYTES,
        })
    }

    /// Sum of all (key, value) byte sizes currently in the tree.
    #[must_use]
    pub fn used_bytes(&self) -> u64 {
        self.tree
            .iter()
            .filter_map(std::result::Result::ok)
            .map(|(k, v)| (k.len() + v.len()) as u64)
            .sum()
    }

    /// Return the value for `key`, or `None` if not present.
    #[must_use]
    pub fn get_blocking(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.tree.get(key).ok().flatten().map(|v| v.to_vec())
    }

    /// Insert `key` → `value`, enforcing the quota.
    ///
    /// Returns `Err` when storing the entry would exceed `quota_bytes`.
    pub fn set_blocking(&self, key: &[u8], value: &[u8]) -> Result<(), String> {
        let projected = self.used_bytes() + (key.len() + value.len()) as u64;
        if projected > self.quota_bytes {
            return Err(format!(
                "plugin {} quota exceeded ({}MB)",
                self.plugin_name,
                self.quota_bytes / 1024 / 1024
            ));
        }
        self.tree
            .insert(key, value)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Remove `key` from the store. No-op if the key does not exist.
    pub fn delete_blocking(&self, key: &[u8]) {
        let _ = self.tree.remove(key);
    }
}

/// Wire `host:store-kv` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_store_kv::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_store_kv::Host for PluginStoreData {
    async fn get(&mut self, key: String) -> Option<Vec<u8>> {
        let ctx = self.store_kv.as_ref()?;
        ctx.get_blocking(key.as_bytes())
    }

    async fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        let Some(ctx) = self.store_kv.as_ref() else {
            return Err("store-kv capability not granted".into());
        };
        ctx.set_blocking(key.as_bytes(), &value)
    }

    async fn delete(&mut self, key: String) {
        if let Some(ctx) = self.store_kv.as_ref() {
            ctx.delete_blocking(key.as_bytes());
        }
    }
}
