//! Sigstore signature verification for plugin manifests.
//!
//! This module is a stub during MVP foundation work. The full implementation
//! lands in Task 6b (`Wire sigstore-rs verification`).

use crate::PluginError;
use crate::manifest::{Manifest, Signature};

/// Verify a Sigstore-keyless signature on a manifest against the given wasm bytes.
///
/// **Stub:** returns an explicit error during the foundation phase so callers
/// know the wiring is incomplete. Real implementation lands in Task 6b.
pub fn verify_sigstore(manifest: &Manifest, _wasm_bytes: &[u8]) -> Result<(), PluginError> {
    if !matches!(manifest.signature, Signature::Sigstore { .. }) {
        return Err(PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: "expected sigstore signature variant".into(),
        });
    }
    Err(PluginError::SignatureInvalid {
        plugin: manifest.name.clone(),
        reason: "sigstore verification not yet wired (Task 6b)".into(),
    })
}
