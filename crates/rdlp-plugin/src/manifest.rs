//! Plugin manifest (`plugin.toml`) schema and validation.
//!
//! Implementation lives in the leaf [`rdlp_plugin_manifest`] crate so author
//! tooling (`tools/sign-plugin`) can pull only the manifest types without
//! the full host's wasmtime + sigstore + sled transitive dependency graph.
//! This module re-exports the canonical surface and bridges the leaf's
//! [`ManifestError`] into [`PluginError`] for in-host call sites.

use crate::error::PluginError;

pub use rdlp_plugin_manifest::{
    KNOWN_CAPABILITIES, Manifest, ManifestError, Signature, canonical_bytes, parse_manifest_file,
    parse_manifest_str, validate_plugin_name,
};

impl From<ManifestError> for PluginError {
    fn from(e: ManifestError) -> Self {
        match e {
            ManifestError::InvalidManifest { path, reason } => {
                PluginError::InvalidManifest { path, reason }
            }
            ManifestError::InvalidPluginName { name, reason } => {
                PluginError::InvalidPluginName { name, reason }
            }
            ManifestError::ClaimsOverrideOutsideMatches { host } => PluginError::InvalidManifest {
                path: std::path::PathBuf::new(),
                reason: format!(
                    "claims_override entry '{host}' does not correspond \
                         to any host in the matches patterns"
                ),
            },
            ManifestError::Toml(e) => PluginError::Toml(e),
            ManifestError::Io(e) => PluginError::Io(e),
        }
    }
}
