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

use crate::PluginError;
use crate::manifest::{Manifest, Signature};
use base64::Engine as _;
use sigstore::bundle::Bundle;
use sigstore::bundle::verify::blocking::Verifier;
use sigstore::bundle::verify::policy::Identity;

/// Verify a Sigstore-keyless signature on a manifest against the given wasm bytes.
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

    verifier
        .verify(wasm_bytes, bundle, &policy, false)
        .map_err(|e| PluginError::SignatureInvalid {
            plugin: manifest.name.clone(),
            reason: format!("verify: {e}"),
        })
}
