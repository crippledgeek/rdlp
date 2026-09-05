//! Linux Chrome cookie decryption.
//!
//! Chrome on Linux (`os_crypt_linux.cc`) derives a 128-bit key with
//! PBKDF2-HMAC-SHA1 over the password, salt `"saltysalt"`, 1 iteration, and
//! encrypts cookie values with AES-128-CBC under a 16-space IV. The password is
//! either the constant `"peanuts"` (`v10`: plaintext key store, or no keyring)
//! or the login keyring's "Chrome Safe Storage" secret (`v11`).
//!
//! Values are dispatched by prefix, matching Chromium and yt-dlp: a `v10` value
//! is tried against the `"peanuts"` key, a `v11` value against the keyring key,
//! each with an empty-password fallback (which covers auto-unlocked login
//! keyrings that return an empty secret).

use std::collections::HashMap;
use std::path::Path;

use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use log::debug;
use sha1::Sha1;

use super::CipherVersion;

/// PBKDF2 salt (`os_crypt_linux.cc` `kSalt`).
const SALT: &[u8] = b"saltysalt";
/// PBKDF2 iteration count (`kEncryptionIterations`).
const PBKDF2_ITERATIONS: u32 = 1;
/// Derived AES-128 key length in bytes (`kDerivedKeySizeInBits` / 8).
const KEY_LEN: usize = 16;
/// AES-CBC IV: 16 space bytes (`os_crypt_linux.cc` `iv(kIVBlockSizeAES128, ' ')`).
const CBC_IV: [u8; 16] = [b' '; 16];
/// The `v10` password (`GetPasswordV10` returns a cached `"peanuts"`).
const V10_PASSWORD: &[u8] = b"peanuts";
/// Keyring item label Chrome stores its key under (`key_storage_libsecret.cc`
/// `kKeyStorageEntryName` for the Google Chrome build).
const KEYRING_LABEL: &str = "Chrome Safe Storage";
/// AES block size; a CBC ciphertext length must be a positive multiple of it.
const AES_BLOCK_LEN: usize = 16;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Decrypts Linux Chrome cookie values, holding the candidate keys per version.
pub(super) struct Decryptor {
    /// Keys tried for a `v10` value: `["peanuts"-derived, empty-derived]`.
    v10_keys: Vec<[u8; KEY_LEN]>,
    /// Keys tried for a `v11` value: `[keyring-derived?, empty-derived]`.
    v11_keys: Vec<[u8; KEY_LEN]>,
}

impl Decryptor {
    /// Derive the candidate keys, reading the login keyring for the `v11` key.
    ///
    /// `user_data` is unused on Linux (the key does not live under the profile).
    /// A missing/locked keyring is not fatal: `v10` and empty-key data still
    /// decrypt, so construction always succeeds and only `v11` values are lost.
    // Returns `Result` (always `Ok`) to match the Windows `Decryptor::new`,
    // which genuinely fails; the parent module treats both uniformly.
    #[allow(clippy::unnecessary_wraps)]
    pub(super) fn new(_user_data: &Path) -> Result<Self, std::io::Error> {
        let peanuts = derive_key(V10_PASSWORD);
        let empty = derive_key(b"");

        let mut v11_keys = Vec::with_capacity(2);
        match keyring_password() {
            Some(password) => v11_keys.push(derive_key(&password)),
            None => debug!(
                "Chrome 'Safe Storage' keyring secret unavailable; v11 cookies will not decrypt"
            ),
        }
        v11_keys.push(empty);

        Ok(Self {
            v10_keys: vec![peanuts, empty],
            v11_keys,
        })
    }

    /// Decrypt one AES-128-CBC cookie body, trying each candidate key for the
    /// value's version in order. Returns the raw plaintext; the caller strips
    /// any hash prefix and decodes UTF-8.
    pub(super) fn decrypt_body(&self, version: CipherVersion, body: &[u8]) -> Option<Vec<u8>> {
        let keys = match version {
            CipherVersion::V10 => &self.v10_keys,
            CipherVersion::V11 => &self.v11_keys,
        };
        keys.iter().find_map(|key| cbc_decrypt(body, key))
    }
}

/// Derive a 128-bit key from `password` per Chromium's Linux parameters.
fn derive_key(password: &[u8]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<Sha1>(password, SALT, PBKDF2_ITERATIONS, &mut key);
    key
}

/// Decrypt an AES-128-CBC body (IV = 16 spaces, PKCS#7). Returns `None` on any
/// length or padding error — the wrong key almost always fails unpadding, which
/// is what lets the caller try keys in turn.
fn cbc_decrypt(body: &[u8], key: &[u8; KEY_LEN]) -> Option<Vec<u8>> {
    if body.is_empty() || !body.len().is_multiple_of(AES_BLOCK_LEN) {
        return None;
    }
    let mut buf = body.to_vec();
    let plaintext = Aes128CbcDec::new_from_slices(key, &CBC_IV)
        .ok()?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .ok()?;
    Some(plaintext.to_vec())
}

/// Read Chrome's "Safe Storage" secret from the login keyring over the
/// org.freedesktop.secrets D-Bus API.
///
/// Runs the async Secret Service client on a private current-thread runtime:
/// `extract_cookies` is invoked inside `spawn_blocking` (see
/// `rdlp-cookies/src/lib.rs`), so this thread is not a runtime worker and may
/// own a nested runtime. Any failure (no D-Bus, no keyring, locked, absent
/// item) returns `None` — v11 decryption is then simply skipped.
fn keyring_password() -> Option<Vec<u8>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime.block_on(keyring_password_async())
}

/// Async body of [`keyring_password`]: connect, scan the default collection for
/// the "Chrome Safe Storage" item (matching by label, as Chromium and yt-dlp
/// do — GNOME Keyring does not organise Chrome's key by searchable attributes),
/// unlock it if needed, and return its secret.
async fn keyring_password_async() -> Option<Vec<u8>> {
    use secret_service::{EncryptionType, SecretService};

    let service = SecretService::connect(EncryptionType::Dh).await.ok()?;

    // Prefer the schema-scoped search; fall back to scanning all items by label
    // when the keyring did not index Chrome's item by attribute.
    let mut attributes = HashMap::new();
    attributes.insert("xdg:schema", "chrome_libsecret_os_crypt_password");
    if let Ok(result) = service.search_items(attributes).await
        && let Some(secret) = first_secret(&result.unlocked, &result.locked).await
    {
        return Some(secret);
    }

    let collection = service.get_default_collection().await.ok()?;
    if collection.is_locked().await.unwrap_or(false) {
        collection.unlock().await.ok()?;
    }
    let items = collection.get_all_items().await.ok()?;
    for item in items {
        if item.get_label().await.ok().as_deref() == Some(KEYRING_LABEL) {
            if item.is_locked().await.unwrap_or(false) {
                item.unlock().await.ok()?;
            }
            return item.get_secret().await.ok();
        }
    }
    None
}

/// Return the first readable secret from a search result, unlocking a locked
/// item if that is the only match.
async fn first_secret(
    unlocked: &[secret_service::Item<'_>],
    locked: &[secret_service::Item<'_>],
) -> Option<Vec<u8>> {
    if let Some(item) = unlocked.first() {
        return item.get_secret().await.ok();
    }
    let item = locked.first()?;
    item.unlock().await.ok()?;
    item.get_secret().await.ok()
}

#[cfg(test)]
// indexing_slicing: test-only prefix stripping (`[3..]`) on fixed-length
// constants defined in this module; the lengths are known at the call site.
// disallowed_methods: the DB-fixture test uses synchronous std::fs directly,
// which is fine in a #[test] (no async runtime to block).
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods
)]
mod tests {
    use super::*;
    use crate::chrome::{Decrypted, decrypt_value};

    impl Decryptor {
        /// Build a decryptor with explicit keys, so decryption logic tests need
        /// no keyring/D-Bus.
        fn from_keys(v10_keys: Vec<[u8; KEY_LEN]>, v11_keys: Vec<[u8; KEY_LEN]>) -> Self {
            Self { v10_keys, v11_keys }
        }
    }

    // Known-answer keys, computed independently with Python's hashlib:
    //   pbkdf2_hmac('sha1', pw, b'saltysalt', 1, 16)
    const PEANUTS_KEY: [u8; KEY_LEN] = [
        253, 98, 31, 229, 162, 180, 2, 83, 157, 250, 20, 124, 169, 39, 39, 120,
    ];
    const EMPTY_KEY: [u8; KEY_LEN] = [
        208, 208, 236, 156, 125, 119, 212, 58, 197, 65, 135, 250, 72, 24, 209, 127,
    ];

    #[test]
    fn derive_key_matches_known_answers() {
        assert_eq!(derive_key(V10_PASSWORD), PEANUTS_KEY);
        assert_eq!(derive_key(b""), EMPTY_KEY);
    }

    // A real Chromium-format v10 blob: "v10" ‖ AES-128-CBC(peanuts, "test_value"),
    // generated independently with yt-dlp's pure-Python AES.
    const V10_TEST_VALUE: &[u8] = &[
        118, 49, 48, 185, 131, 10, 213, 97, 68, 14, 220, 112, 165, 230, 22, 234, 177, 215, 188,
    ];

    #[test]
    fn cbc_decrypt_recovers_chromium_v10_blob() {
        // Strip the "v10" prefix, decrypt with the peanuts key.
        let body = &V10_TEST_VALUE[3..];
        assert_eq!(
            cbc_decrypt(body, &PEANUTS_KEY).as_deref(),
            Some(&b"test_value"[..])
        );
    }

    #[test]
    fn cbc_decrypt_rejects_non_block_length() {
        assert!(cbc_decrypt(b"not-a-block", &PEANUTS_KEY).is_none());
        assert!(cbc_decrypt(b"", &PEANUTS_KEY).is_none());
    }

    #[test]
    fn decrypt_body_v10_uses_peanuts_key() {
        let decryptor = Decryptor::from_keys(vec![PEANUTS_KEY, EMPTY_KEY], vec![EMPTY_KEY]);
        let body = &V10_TEST_VALUE[3..];
        assert_eq!(
            decryptor.decrypt_body(CipherVersion::V10, body).as_deref(),
            Some(&b"test_value"[..])
        );
    }

    #[test]
    fn decrypt_body_v11_without_keyring_key_fails() {
        // v11 keys are [empty] only (no keyring). A peanuts-encrypted body must
        // not decrypt as v11 — the empty key fails unpadding.
        let decryptor = Decryptor::from_keys(vec![PEANUTS_KEY, EMPTY_KEY], vec![EMPTY_KEY]);
        let body = &V10_TEST_VALUE[3..];
        assert!(decryptor.decrypt_body(CipherVersion::V11, body).is_none());
    }

    // "v10" ‖ CBC(peanuts, sha256("example.com") ‖ "hello") — a version-24 blob.
    const V10_HASHED_HELLO: &[u8] = &[
        118, 49, 48, 163, 201, 153, 4, 94, 130, 107, 150, 117, 196, 246, 239, 211, 103, 24, 172,
        96, 119, 22, 72, 27, 64, 195, 82, 246, 116, 113, 218, 190, 101, 149, 234, 214, 138, 37,
        135, 190, 107, 59, 10, 139, 186, 206, 121, 221, 73, 165, 210,
    ];

    #[test]
    fn decrypt_value_strips_v24_hash_prefix_on_linux() {
        let decryptor = Decryptor::from_keys(vec![PEANUTS_KEY, EMPTY_KEY], vec![EMPTY_KEY]);
        // Version 24: the 32-byte hash is stripped, leaving "hello".
        match decrypt_value(&decryptor, V10_HASHED_HELLO, 24) {
            Decrypted::Value(v) => assert_eq!(v, "hello"),
            _ => panic!("expected a decrypted value"),
        }
        // Version 23: the hash bytes remain, so the result is not valid UTF-8.
        match decrypt_value(&decryptor, V10_HASHED_HELLO, 23) {
            Decrypted::Undecryptable => {}
            _ => panic!("expected undecryptable below version 24"),
        }
    }

    #[test]
    fn decrypt_value_skips_a_malformed_short_v10_value() {
        // A "v10"-prefixed value too short to be valid ciphertext is skipped as
        // undecryptable, not returned verbatim as a bogus plaintext cookie (the
        // pre-split code returned "v10xxxxx" here). Deliberate behaviour change.
        let decryptor = Decryptor::from_keys(vec![PEANUTS_KEY, EMPTY_KEY], vec![EMPTY_KEY]);
        match decrypt_value(&decryptor, b"v10xxxxx", 0) {
            Decrypted::Undecryptable => {}
            _ => panic!("a short v10 value must be skipped, not returned as plaintext"),
        }
    }

    /// End-to-end over a hand-built cookie DB: proves `read_cookies_from_db`
    /// reads `meta.version`, applies the v24 strip, counts each inserted cookie,
    /// and decrypts a real v10 value. Kills the mutants on `read_meta_version`
    /// and the `read_cookies_from_db` count path (a wrong meta version drops the
    /// hashed cookie, breaking the count of 2).
    #[test]
    fn read_cookies_from_db_reads_meta_version_and_counts_inserts() {
        use crate::chrome::{read_cookies_from_db, read_meta_version};
        use rusqlite::params;

        let dir = std::env::temp_dir().join(format!("rdlp-cookie-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("Cookies");

        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute("CREATE TABLE meta (key TEXT, value TEXT)", [])
                .unwrap();
            conn.execute("INSERT INTO meta VALUES ('version', '24')", [])
                .unwrap();
            conn.execute(
                "CREATE TABLE cookies (host_key TEXT, name TEXT, encrypted_value BLOB, \
                 value TEXT, path TEXT, is_secure INTEGER, is_httponly INTEGER)",
                [],
            )
            .unwrap();
            // A v10 value whose plaintext is sha256-hash ‖ "hello"; at version 24
            // the hash is stripped and the cookie value is "hello".
            conn.execute(
                "INSERT INTO cookies VALUES ('.example.com', 'sess', ?1, '', '/', 1, 1)",
                params![V10_HASHED_HELLO],
            )
            .unwrap();
            // A plaintext-column cookie: taken verbatim, no decryption.
            conn.execute(
                "INSERT INTO cookies VALUES ('.example.com', 'plain', x'', 'plainval', '/', 0, 0)",
                [],
            )
            .unwrap();

            assert_eq!(read_meta_version(&conn), 24);
        }

        let decryptor = Decryptor::from_keys(vec![PEANUTS_KEY, EMPTY_KEY], vec![EMPTY_KEY]);
        // extract_cookies passes the inner wreq Jar (the CookieStore impl).
        let jar = wreq::cookie::Jar::default();
        let count = read_cookies_from_db(&db_path, &decryptor, &jar).unwrap();

        // Both cookies inserted: the decrypted "hello" and the plaintext one.
        assert_eq!(count, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
