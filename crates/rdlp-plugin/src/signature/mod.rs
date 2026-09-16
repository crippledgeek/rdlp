//! Plugin signature verification (Sigstore + Ed25519).

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::missing_errors_doc)]

pub mod ed25519;
pub mod sigstore;

use crate::PluginError;
use crate::manifest::{Manifest, Signature};
use std::io::Read;
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

/// Read a `plugin.wasm` for verification under [`MAX_PLUGIN_WASM_BYTES`].
///
/// The size is checked twice: the open handle's own metadata refuses a
/// declared overrun without reading a byte, and the read itself is cut
/// at the cap, so a source whose metadata lies — a symlink to a device
/// file stats as 0 bytes — or a file swapped after the stat is still
/// bounded by what is actually read.
///
/// # Errors
///
/// [`PluginError::WasmTooLarge`] over the cap; the I/O error otherwise.
fn read_plugin_wasm(path: &Path) -> Result<Vec<u8>, PluginError> {
    #[allow(clippy::disallowed_methods)] // load-time sync I/O, as the loader's
    let file = std::fs::File::open(path)?;
    let declared = file.metadata()?.len();
    read_capped(file, declared, MAX_PLUGIN_WASM_BYTES, path)
}

/// [`read_plugin_wasm`] over any source: `declared` is what the source
/// claims to hold, `max` the cap; the bytes read are the final word.
fn read_capped(
    source: impl Read,
    declared: u64,
    max: u64,
    path: &Path,
) -> Result<Vec<u8>, PluginError> {
    let too_large = |bytes| PluginError::WasmTooLarge {
        path: path.to_path_buf(),
        bytes,
        max,
    };
    if declared > max {
        return Err(too_large(declared));
    }
    let mut wasm = Vec::with_capacity(usize::try_from(declared).unwrap_or(0));
    source.take(max + 1).read_to_end(&mut wasm)?;
    let read = wasm.len() as u64;
    if read > max {
        return Err(too_large(read));
    }
    Ok(wasm)
}

/// Top-level signature verification entry point. Dispatches to the right backend
/// based on the manifest's signature variant.
pub fn verify(manifest: &Manifest, wasm_bytes: &[u8]) -> Result<(), PluginError> {
    match &manifest.signature {
        Signature::Sigstore { .. } => sigstore::verify_sigstore(manifest, wasm_bytes),
        Signature::Ed25519 { .. } => ed25519::verify_ed25519(manifest, wasm_bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    /// A source that must never be asked for bytes.
    struct Unreadable;

    impl io::Read for Unreadable {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("a declared overrun must not be read")
        }
    }

    /// A source whose declared size lies (a symlink to `/dev/zero` stats
    /// as 0 bytes) is still cut off: the bytes actually read are the
    /// guard, not the stat.
    #[test]
    fn read_capped_stops_an_endless_source_at_the_cap() {
        let err = read_capped(io::repeat(0), 0, 16, Path::new("/p/plugin.wasm"))
            .expect_err("an endless source is over any cap");
        assert!(
            matches!(
                err,
                PluginError::WasmTooLarge {
                    bytes: 17,
                    max: 16,
                    ..
                }
            ),
            "{err}"
        );
    }

    /// Exactly the cap is allowed; the declared size short-circuits
    /// without reading when it is already over.
    #[test]
    fn read_capped_allows_the_cap_and_refuses_a_declared_overrun_unread() {
        let ok = read_capped(io::repeat(7).take(16), 16, 16, Path::new("/p")).expect("at the cap");
        assert_eq!(ok.len(), 16);

        let err = read_capped(Unreadable, 17, 16, Path::new("/p")).expect_err("declared over");
        assert!(matches!(
            err,
            PluginError::WasmTooLarge {
                bytes: 17,
                max: 16,
                ..
            }
        ));
    }
}
