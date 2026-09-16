//! Plugin signature verification (Sigstore + Ed25519).

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::missing_errors_doc)]

pub mod ed25519;
pub mod sigstore;

use crate::PluginError;
use crate::manifest::{Manifest, Signature};
use std::path::Path;

/// The most `plugin.wasm` bytes the host reads before verifying them.
///
/// The Ed25519 signature covers the raw bytes (`canonical_bytes(manifest)
/// || wasm`), so the whole binary must be in memory to check it; this is
/// the bound on that read, for every path that performs it (the loader,
/// `rdlp plugin list` / `info`). No external authority sets the number:
/// wasmtime bounds only runtime resources, and comparable plugin systems
/// cap for registry storage, not verification memory. 128 MiB is sized
/// from rdlp's own plugins — the Python/yt-dlp shim components are ~36 MB,
/// native ones a few MB — with room to grow (#784).
pub const MAX_PLUGIN_WASM_BYTES: u64 = 128 * 1024 * 1024;

/// Read `wasm_path` under the cap and verify `manifest`'s signature over it.
///
/// Returns the verified bytes. This is the one read-then-verify every
/// consumer of a plugin binary performs: the loader, which goes on to
/// compile them, and `rdlp plugin list` / `info`, which only want the
/// verdict.
///
/// # Errors
///
/// [`PluginError::WasmTooLarge`], the I/O error, or the verification
/// failure, in that order.
pub fn verify_file(manifest: &Manifest, wasm_path: &Path) -> Result<Vec<u8>, PluginError> {
    let wasm = read_plugin_wasm(wasm_path)?;
    verify(manifest, &wasm)?;
    Ok(wasm)
}

/// Read a `plugin.wasm` for verification, refusing one over
/// [`MAX_PLUGIN_WASM_BYTES`] from its size on disk — before any of it is
/// read.
///
/// # Errors
///
/// [`PluginError::WasmTooLarge`] over the cap; the I/O error otherwise.
fn read_plugin_wasm(path: &Path) -> Result<Vec<u8>, PluginError> {
    #[allow(clippy::disallowed_methods)] // load-time sync I/O, as the loader's
    let bytes = std::fs::metadata(path)?.len();
    if bytes > MAX_PLUGIN_WASM_BYTES {
        return Err(PluginError::WasmTooLarge {
            path: path.to_path_buf(),
            bytes,
            max: MAX_PLUGIN_WASM_BYTES,
        });
    }
    #[allow(clippy::disallowed_methods)]
    Ok(std::fs::read(path)?)
}

/// Top-level signature verification entry point. Dispatches to the right backend
/// based on the manifest's signature variant.
pub fn verify(manifest: &Manifest, wasm_bytes: &[u8]) -> Result<(), PluginError> {
    match &manifest.signature {
        Signature::Sigstore { .. } => sigstore::verify_sigstore(manifest, wasm_bytes),
        Signature::Ed25519 { .. } => ed25519::verify_ed25519(manifest, wasm_bytes),
    }
}
