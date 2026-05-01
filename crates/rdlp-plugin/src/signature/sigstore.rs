//! Sigstore keyless signature verification for plugin manifests.
//!
//! Uses sigstore-rs's blocking `Verifier::production()` against the public-good
//! trust root. The manifest's `Signature::Sigstore { identity, oidc_issuer, .. }`
//! fields are mapped to a sigstore `Identity` policy: `identity` matches the SAN
//! on the Fulcio-issued cert, `oidc_issuer` matches the OIDC issuer extension.
//!
//! **Network on first call.** `Verifier::production()` fetches the sigstore TUF
//! trust root from the public sigstore endpoint the first time it is constructed.
//! That blocking I/O happens inside `verify_sigstore`. The plugin loader runs at
//! startup where network access is already a hard dependency, so this is
//! acceptable for the MVP — but it surprises anyone calling it from a no-network
//! context.
//!
//! **Signed payload** matches the Ed25519 path: `canonical_bytes(manifest) || wasm_bytes`.
//! The combined byte sequence is hashed with SHA-256 and the resulting digest is
//! fed to `Verifier::verify_digest` so the bundle's hashedrekord/DSSE statement
//! covers both the manifest contents and the WASM binary. Manifests with tampered
//! capabilities, match patterns, or `claims_override` will therefore fail
//! verification even when the WASM binary is unchanged.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::manual_let_else,
    clippy::match_wildcard_for_single_variants
)]

use crate::PluginError;
use crate::manifest::{Manifest, Signature, canonical_bytes};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use sigstore::bundle::Bundle;
use sigstore::bundle::verify::blocking::Verifier;
use sigstore::bundle::verify::policy::Identity;

/// Verify a Sigstore-keyless signature on a manifest against the given wasm bytes.
///
/// The signed artifact is `canonical_bytes(manifest) || wasm_bytes`, matching the
/// Ed25519 signing convention. Both the manifest content (capabilities, matches,
/// claims_override, etc.) and the WASM binary must match what was signed.
pub fn verify_sigstore(manifest: &Manifest, wasm_bytes: &[u8]) -> Result<(), PluginError> {
    let (identity, oidc_issuer, bundle_b64) = match &manifest.signature {
        Signature::Sigstore {
            identity,
            oidc_issuer,
            bundle,
        } => (identity, oidc_issuer, bundle),
        _ => {
            return Err(PluginError::SignatureInvalid {
                plugin: manifest.name.clone(),
                reason: "expected sigstore signature variant".into(),
            });
        }
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(bundle_b64)
        .map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("base64 decode bundle: {e}"),
        })?;

    let bundle: Bundle =
        serde_json::from_slice(&decoded).map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("parse bundle: {e}"),
        })?;

    let policy = Identity::new(identity, oidc_issuer);

    let verifier = Verifier::production().map_err(|e| PluginError::SignatureInvalid {
        plugin: manifest.name.clone(),
        reason: format!("init verifier: {e}"),
    })?;

    // Build the combined payload: canonical_bytes(manifest) || wasm_bytes.
    // This matches the Ed25519 signing convention so both signature variants
    // cover the same byte sequence. Manifests with tampered capabilities,
    // match patterns, or claims_override will therefore fail verification even
    // when the WASM binary bytes are unchanged.
    let mut hasher = Sha256::new();
    hasher.update(canonical_bytes(manifest));
    hasher.update(wasm_bytes);

    verifier
        .verify_digest(hasher, bundle, &policy, false)
        .map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("verify: {e}"),
        })
}
