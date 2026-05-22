//! Ed25519 signature verification for plugin manifests.
//!
//! The signing payload is `canonical_bytes(manifest) || wasm_bytes`. Reference
//! implementations in other languages must reproduce this byte vector exactly
//! to interop with rdlp.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::missing_errors_doc,
    clippy::manual_let_else,
    clippy::match_wildcard_for_single_variants
)]

use crate::PluginError;
use crate::manifest::{Manifest, Signature, canonical_bytes};
use base64::Engine as _;
use ed25519_dalek::{Signature as DalekSig, VerifyingKey};

/// Verify the Ed25519 signature on a manifest against the given wasm bytes.
///
/// Errors with `PluginError::SignatureInvalid` if the manifest's signature is
/// not the Ed25519 variant, the encoded values are malformed, or the signature
/// does not match.
pub fn verify_ed25519(manifest: &Manifest, wasm_bytes: &[u8]) -> Result<(), PluginError> {
    let (pubkey_b64, sig_b64) = match &manifest.signature {
        Signature::Ed25519 { pubkey, signature } => (pubkey, signature),
        _ => {
            return Err(PluginError::SignatureInvalid {
                plugin: manifest.name.clone(),
                reason: "expected ed25519 signature variant".into(),
            });
        }
    };

    let pubkey_bytes = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64)
        .map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("pubkey base64 decode failed: {e}"),
        })?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("signature base64 decode failed: {e}"),
        })?;

    let pubkey_array: [u8; 32] =
        pubkey_bytes
            .as_slice()
            .try_into()
            .map_err(|_| PluginError::SignatureInvalid {
                plugin: manifest.name.clone(),
                reason: format!(
                    "pubkey wrong length: got {} bytes, expected 32",
                    pubkey_bytes.len()
                ),
            })?;
    let sig_array: [u8; 64] =
        sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| PluginError::SignatureInvalid {
                plugin: manifest.name.clone(),
                reason: format!(
                    "signature wrong length: got {} bytes, expected 64",
                    sig_bytes.len()
                ),
            })?;

    let verifying_key =
        VerifyingKey::from_bytes(&pubkey_array).map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("invalid ed25519 pubkey: {e}"),
        })?;
    let sig = DalekSig::from_bytes(&sig_array);

    let mut buf = canonical_bytes(manifest);
    buf.extend_from_slice(wasm_bytes);

    // `verify_strict` (vs `verify`) rejects non-cofactor-reduced signatures,
    // closing a malleability vector where two distinct byte sequences could
    // verify against the same key+message pair. See ed25519_dalek docs.
    verifying_key
        .verify_strict(&buf, &sig)
        .map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("ed25519 signature verification failed: {e}"),
        })
}
