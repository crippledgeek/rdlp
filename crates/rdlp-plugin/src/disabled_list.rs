//! Plugin disabled-list TOML at `~/.config/rdlp/plugin-disabled.toml`.
//!
//! Managed by the `rdlp plugin disable / enable` CLI commands, consumed by
//! the orchestrator's [`crate::loader`] / `plugin_bootstrap` so a disabled
//! plugin is skipped at load time across process restarts. Living here (in
//! `rdlp-plugin`) lets both crates read it without `rdlp-api` depending on
//! `rdlp-cli`.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::missing_errors_doc)]

use crate::PluginError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// On-disk TOML shape: a single `disabled = [...]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisabledList {
    /// Plugin names the operator has explicitly opted out of loading.
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Read the disabled list at `path`. Returns an empty list when the file
/// is absent (operator never disabled anything yet).
///
/// **Does not silently fall back to empty on parse error** — a corrupted
/// file is treated as a hard failure so callers can surface it; silently
/// re-enabling a previously-blocked plugin is a security regression.
pub fn read_disabled_list(path: &Path) -> Result<Vec<String>, PluginError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    #[allow(clippy::disallowed_methods)] // load-time sync I/O is acceptable
    let s = std::fs::read_to_string(path).map_err(PluginError::Io)?;
    let parsed: DisabledList = toml::from_str(&s)?;
    Ok(parsed.disabled)
}
