//! Windows Chrome cookie decryption: DPAPI-wrapped AES-256 key + AES-256-GCM
//! values (Chrome v80+).
//!
//! App-Bound Encryption (`v20`, Chrome 127+) is intercepted before dispatch in
//! the parent module and never reaches this decryptor.

use std::path::Path;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use super::CipherVersion;

/// AES-GCM nonce length (96 bits / 12 bytes).
const NONCE_LEN: usize = 12;

/// The `DPAPI` prefix on the base64-decoded `os_crypt.encrypted_key`.
const DPAPI_PREFIX: &[u8] = b"DPAPI";

/// Decrypts Windows Chrome cookie values with the profile's single AES-256 key.
pub(super) struct Decryptor {
    /// The AES-256 key, DPAPI-decrypted from `Local State`.
    key: Vec<u8>,
}

impl Decryptor {
    /// Load and DPAPI-decrypt the AES key from `<user_data>/Local State`.
    pub(super) fn new(user_data: &Path) -> Result<Self, std::io::Error> {
        let local_state = user_data.join("Local State");
        if !local_state.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Chrome Local State file not found",
            ));
        }
        Ok(Self {
            key: load_encryption_key(&local_state)?,
        })
    }

    /// Decrypt one AES-256-GCM cookie body (nonce ‖ ciphertext ‖ tag).
    ///
    /// `version` is ignored: Windows uses one key for both `v10` and `v11`.
    /// Returns the raw plaintext; the caller strips any hash prefix and decodes
    /// UTF-8. All indexing below is guarded by the length check.
    #[allow(clippy::indexing_slicing)]
    pub(super) fn decrypt_body(&self, _version: CipherVersion, body: &[u8]) -> Option<Vec<u8>> {
        if body.len() < NONCE_LEN + 1 {
            return None;
        }
        let nonce_bytes = &body[..NONCE_LEN];
        let ciphertext = &body[NONCE_LEN..];

        let cipher = Aes256Gcm::new_from_slice(&self.key).ok()?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher.decrypt(nonce, ciphertext).ok()
    }
}

/// Load and decrypt the AES encryption key from Local State.
fn load_encryption_key(local_state_path: &Path) -> Result<Vec<u8>, std::io::Error> {
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let content = std::fs::read_to_string(local_state_path)?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let encrypted_key_b64 = json
        .get("os_crypt")
        .and_then(|o| o.get("encrypted_key"))
        .and_then(|k| k.as_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No os_crypt.encrypted_key in Local State",
            )
        })?;

    let encrypted_key = BASE64.decode(encrypted_key_b64).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Base64 decode error: {e}"),
        )
    })?;

    // Strip "DPAPI" prefix (5 bytes). Both slices guarded by the len check.
    #[allow(clippy::indexing_slicing)]
    if encrypted_key.len() < DPAPI_PREFIX.len()
        || &encrypted_key[..DPAPI_PREFIX.len()] != DPAPI_PREFIX
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Encrypted key missing DPAPI prefix",
        ));
    }
    #[allow(clippy::indexing_slicing)]
    let dpapi_encrypted = &encrypted_key[DPAPI_PREFIX.len()..];

    decrypt_dpapi_key(dpapi_encrypted)
}

/// Decrypt a DPAPI-encrypted key.
// Windows DPAPI FFI: the cast/borrow patterns below are dictated by the Win32
// ABI (CRYPT_INTEGER_BLOB requires `*mut u8` even for read-only input,
// CryptUnprotectData takes `*const`/`*mut` raw pointers, LocalFree takes HLOCAL
// = isize). The clippy suggestions either change the ABI shape or are no-ops
// after rewriting; allow the FFI-shape lints in this scoped block.
#[allow(
    clippy::cast_possible_truncation,
    clippy::ptr_as_ptr,
    clippy::ptr_cast_constness,
    clippy::as_ptr_cast_mut,
    clippy::borrow_as_ptr
)]
fn decrypt_dpapi_key(encrypted: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };

    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let result = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut output,
        )
    };

    if result == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "DPAPI CryptUnprotectData failed — run as the same user who owns the Chrome profile",
        ));
    }

    let decrypted = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let vec = slice.to_vec();
        // Free the DPAPI-allocated memory
        windows_sys::Win32::Foundation::LocalFree(output.pbData as _);
        vec
    };

    Ok(decrypted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::Aead;

    /// Build a Decryptor with a known key, bypassing DPAPI, for GCM tests.
    fn decryptor_with_key(key: Vec<u8>) -> Decryptor {
        Decryptor { key }
    }

    /// AES-256-GCM round trip proves `decrypt_body` recovers a value the same
    /// key encrypted, in Chrome's nonce‖ciphertext‖tag layout.
    #[test]
    fn decrypt_body_recovers_gcm_round_trip() {
        let key = vec![0x11u8; 32];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let nonce = Nonce::from_slice(&[0x22u8; NONCE_LEN]);
        let ciphertext = cipher.encrypt(nonce, b"secret_value".as_ref()).unwrap();

        let mut body = nonce.to_vec();
        body.extend_from_slice(&ciphertext);

        let out = decryptor_with_key(key).decrypt_body(CipherVersion::V10, &body);
        assert_eq!(out.as_deref(), Some(b"secret_value".as_ref()));
    }

    #[test]
    fn decrypt_body_rejects_short_body() {
        let out = decryptor_with_key(vec![0u8; 32]).decrypt_body(CipherVersion::V10, b"short");
        assert!(out.is_none());
    }

    /// Version-24 hash-prefix strip on the Windows path: a GCM plaintext of
    /// `sha256-hash ‖ "value"` must yield `"value"` once the parent module
    /// strips the prefix. Exercised end-to-end through `super::decrypt_value`.
    #[test]
    fn windows_v24_hash_prefix_is_stripped() {
        use super::super::{Decrypted, decrypt_value};

        let key = vec![0x33u8; 32];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let nonce = Nonce::from_slice(&[0x44u8; NONCE_LEN]);
        let plaintext = [vec![0x55u8; 32], b"value".to_vec()].concat();
        let ciphertext = cipher.encrypt(nonce, plaintext.as_ref()).unwrap();

        let mut encrypted_value = b"v10".to_vec();
        encrypted_value.extend_from_slice(nonce);
        encrypted_value.extend_from_slice(&ciphertext);

        let decryptor = decryptor_with_key(key);
        match decrypt_value(&decryptor, &encrypted_value, 24) {
            Decrypted::Value(v) => assert_eq!(v, "value"),
            _ => panic!("expected a decrypted value"),
        }
        // Below version 24 the 32-byte prefix is part of the value and the
        // result is not valid UTF-8, so it is reported undecryptable.
        match decrypt_value(&decryptor, &encrypted_value, 23) {
            Decrypted::Undecryptable => {}
            _ => panic!("expected undecryptable below version 24"),
        }
    }
}
