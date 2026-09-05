//! Fallback Chrome cookie decryptor for targets without a native
//! implementation (currently macOS, which would need Keychain support).
//!
//! Construction fails with `Unsupported`, so `extract_cookies` returns an
//! error before any database work — matching the behaviour before `chrome.rs`
//! was split by platform. The crate still compiles everywhere, and Firefox and
//! Netscape cookie sources continue to work.

use std::path::Path;

use super::CipherVersion;

/// A decryptor that cannot decrypt: it never successfully constructs.
pub(super) struct Decryptor;

impl Decryptor {
    /// Always fails: Chrome cookie decryption is unimplemented on this target.
    pub(super) fn new(_user_data: &Path) -> Result<Self, std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Chrome cookie extraction is not supported on this platform. \
             Export cookies to a Netscape file and use --cookies <file> instead.",
        ))
    }

    /// Unreachable in practice (`new` never returns `Ok`); present so the parent
    /// module's platform-agnostic dispatch type-checks.
    pub(super) fn decrypt_body(&self, _version: CipherVersion, _body: &[u8]) -> Option<Vec<u8>> {
        None
    }
}
