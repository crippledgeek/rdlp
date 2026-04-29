//! Plugin signature verification (Sigstore + Ed25519).

pub mod ed25519;
pub mod sigstore;

use crate::PluginError;
use crate::manifest::{Manifest, Signature};

/// Top-level signature verification entry point. Dispatches to the right backend
/// based on the manifest's signature variant.
pub fn verify(manifest: &Manifest, wasm_bytes: &[u8]) -> Result<(), PluginError> {
    match &manifest.signature {
        Signature::Sigstore { .. } => sigstore::verify_sigstore(manifest, wasm_bytes),
        Signature::Ed25519 { .. } => ed25519::verify_ed25519(manifest, wasm_bytes),
    }
}
