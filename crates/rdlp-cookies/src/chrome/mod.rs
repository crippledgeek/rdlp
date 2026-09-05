//! Chrome/Chromium cookie extraction.
//!
//! Extracts cookies from Chrome's `SQLite` database and decrypts them. The
//! decryption scheme is entirely platform-specific and lives in the sibling
//! modules:
//!
//! - Windows (`windows.rs`): the AES-256 key is DPAPI-wrapped in `Local State`
//!   and cookie values are AES-256-GCM (Chrome v80+).
//! - Linux (`linux.rs`): the AES-128 key is derived from the login keyring
//!   (`v11`) or the constant `"peanuts"` (`v10`) and values are AES-128-CBC.
//!
//! This module owns everything the two platforms share: locating the database,
//! reading rows and the `meta.version`, classifying a value's encryption
//! prefix, stripping the version-24 hash prefix, and dispatching to the
//! platform `Decryptor`.

use std::path::{Path, PathBuf};

use log::{debug, warn};
use wreq::cookie::CookieStore;

use crate::util;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux::Decryptor;
#[cfg(target_os = "windows")]
use windows::Decryptor;

/// The `v10` encryption prefix. On Windows this key comes from DPAPI; on Linux
/// it is derived from the constant password `"peanuts"` (a plaintext key store,
/// or a keyring-less environment).
const V10_PREFIX: &[u8] = b"v10";

/// The `v11` encryption prefix — present only on Linux, where the key is
/// derived from the login keyring's "Chrome Safe Storage" secret.
const V11_PREFIX: &[u8] = b"v11";

/// The `v20` (App-Bound Encryption) prefix — Windows Chrome 127+. The key is
/// bound to the Chrome binary via a SYSTEM service and cannot be recovered by
/// an external same-user process, so these values are recognised and reported,
/// never decrypted. See <https://github.com/yt-dlp/yt-dlp/issues/15401>.
const V20_PREFIX: &[u8] = b"v20";

/// Length of every encryption prefix (`v10`/`v11`/`v20`).
const PREFIX_LEN: usize = 3;

/// Length of the SHA-256 domain-hash prefix prepended to decrypted plaintext.
const HASH_PREFIX_LEN: usize = 32;

/// The cookie-DB `meta.version` from which Chrome prepends [`HASH_PREFIX_LEN`]
/// bytes of SHA-256 domain hash to every decrypted plaintext.
///
/// Cited: yt-dlp `cookies.py` (`hash_prefix=self._meta_version >= 24`) and the
/// Chromium change that introduced it. Below this version no prefix is present.
const HASH_PREFIX_MIN_META_VERSION: i64 = 24;

/// Which per-prefix key a Linux value needs. Windows uses one key regardless,
/// so its `Decryptor` ignores this; it exists to keep illegal states
/// unrepresentable — the `v20` prefix is handled before dispatch and never
/// reaches a `Decryptor`.
#[derive(Clone, Copy)]
enum CipherVersion {
    /// `v10`: `"peanuts"`-derived (Linux) or DPAPI (Windows) key.
    V10,
    /// `v11`: keyring-derived key (Linux only).
    V11,
}

/// How a stored cookie value is protected, decided by its leading bytes.
enum Scheme<'a> {
    /// An encrypted value: the key version plus the ciphertext after the prefix.
    Encrypted {
        /// Which key decrypts it.
        version: CipherVersion,
        /// The bytes after the 3-byte prefix (nonce/IV + ciphertext).
        body: &'a [u8],
    },
    /// A Windows App-Bound (`v20`) value — recognised but not decryptable here.
    AppBound,
    /// No recognised prefix: an old plaintext value, used verbatim.
    Plaintext,
}

/// The result of attempting to recover one cookie value.
enum Decrypted {
    /// A recovered value (may still be empty, which the caller skips).
    Value(String),
    /// A `v20` App-Bound value that cannot be decrypted here.
    AppBound,
    /// A value that carried a known prefix but could not be decrypted.
    Undecryptable,
}

/// Classify a stored value by its encryption prefix.
fn classify(encrypted_value: &[u8]) -> Scheme<'_> {
    // A value shorter than a prefix cannot be prefixed; treat as plaintext.
    if encrypted_value.len() < PREFIX_LEN {
        return Scheme::Plaintext;
    }
    let (prefix, body) = encrypted_value.split_at(PREFIX_LEN);
    match prefix {
        p if p == V10_PREFIX => Scheme::Encrypted {
            version: CipherVersion::V10,
            body,
        },
        p if p == V11_PREFIX => Scheme::Encrypted {
            version: CipherVersion::V11,
            body,
        },
        p if p == V20_PREFIX => Scheme::AppBound,
        // No recognised prefix — an old, unencrypted value stored verbatim.
        _ => Scheme::Plaintext,
    }
}

/// Strip Chrome's SHA-256 domain-hash prefix from a decrypted plaintext.
///
/// Chrome cookie DBs at `meta.version >= 24` prepend 32 bytes of SHA-256 domain
/// hash to every plaintext before encrypting it. A too-short buffer is returned
/// unchanged rather than truncated — the caller then fails the UTF-8 conversion
/// and skips the cookie, which is safer than emitting a fragment.
fn strip_hash_prefix(mut plaintext: Vec<u8>, meta_version: i64) -> Vec<u8> {
    if meta_version >= HASH_PREFIX_MIN_META_VERSION && plaintext.len() >= HASH_PREFIX_LEN {
        plaintext.drain(..HASH_PREFIX_LEN);
    }
    plaintext
}

/// Decode recovered bytes as a UTF-8 cookie value, or report them undecryptable.
fn decode_utf8(bytes: Vec<u8>) -> Decrypted {
    String::from_utf8(bytes).map_or(Decrypted::Undecryptable, Decrypted::Value)
}

/// Recover one cookie value: classify it, dispatch to the platform decryptor,
/// strip the version-24 hash prefix, and decode as UTF-8.
fn decrypt_value(decryptor: &Decryptor, encrypted_value: &[u8], meta_version: i64) -> Decrypted {
    if encrypted_value.is_empty() {
        return Decrypted::Value(String::new());
    }
    match classify(encrypted_value) {
        Scheme::Plaintext => decode_utf8(encrypted_value.to_vec()),
        Scheme::AppBound => Decrypted::AppBound,
        Scheme::Encrypted { version, body } => {
            let Some(raw) = decryptor.decrypt_body(version, body) else {
                return Decrypted::Undecryptable;
            };
            decode_utf8(strip_hash_prefix(raw, meta_version))
        }
    }
}

/// Extract cookies from Chrome and insert them into the jar.
///
/// Returns the number of cookies loaded.
#[allow(clippy::redundant_pub_crate)] // pub(crate) in private mod is more defensive than pub
pub(crate) fn extract_cookies(jar: &impl CookieStore) -> Result<usize, std::io::Error> {
    let cookie_db = find_cookie_db()?;
    let user_data = chrome_user_data_dir()?;

    debug!("Chrome cookie DB: {}", cookie_db.display());

    // Build the platform decryptor up front so a key/keyring failure surfaces
    // before the database is copied.
    let decryptor = Decryptor::new(&user_data)?;

    util::with_temp_db_copy(&cookie_db, "rdlp_chrome_cookies.db", |temp_db| {
        read_cookies_from_db(temp_db, &decryptor, jar)
    })
}

/// Find Chrome's cookie database path.
fn find_cookie_db() -> Result<PathBuf, std::io::Error> {
    let user_data = chrome_user_data_dir()?;
    let path = user_data.join("Default").join("Cookies");
    if path.exists() {
        return Ok(path);
    }
    // Try Network/Cookies (Chrome 96+)
    let network_path = user_data.join("Default").join("Network").join("Cookies");
    if network_path.exists() {
        return Ok(network_path);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "Chrome cookie database not found at {} or {}",
            path.display(),
            network_path.display()
        ),
    ))
}

/// Get Chrome's User Data directory.
fn chrome_user_data_dir() -> Result<PathBuf, std::io::Error> {
    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var("LOCALAPPDATA").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA not set")
        })?;
        Ok(PathBuf::from(local_app_data)
            .join("Google")
            .join("Chrome")
            .join("User Data"))
    }

    #[cfg(target_os = "linux")]
    {
        Ok(util::home_dir()?.join(".config").join("google-chrome"))
    }

    #[cfg(target_os = "macos")]
    {
        Ok(util::home_dir()?
            .join("Library")
            .join("Application Support")
            .join("Google")
            .join("Chrome"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Chrome cookie extraction not supported on this platform",
        ))
    }
}

/// Read the cookie DB's `meta.version`, defaulting to 0 when absent.
///
/// The version decides whether decrypted plaintexts carry the 32-byte hash
/// prefix (see [`strip_hash_prefix`]).
fn read_meta_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
        // `meta.value` is stored as TEXT; parse it, defaulting to 0.
        let raw: String = row.get(0)?;
        Ok(raw.parse::<i64>().unwrap_or(0))
    })
    .unwrap_or(0)
}

/// Read cookies from the `SQLite` database and insert them into the jar.
fn read_cookies_from_db(
    db_path: &Path,
    decryptor: &Decryptor,
    jar: &impl CookieStore,
) -> Result<usize, std::io::Error> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

    let meta_version = read_meta_version(&conn);

    let mut stmt = conn
        .prepare(
            "SELECT host_key, name, encrypted_value, value, path, is_secure, is_httponly \
             FROM cookies",
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut count = 0;
    let mut app_bound = 0usize;

    let rows = stmt
        .query_map([], |row| {
            let host_key: String = row.get(0)?;
            let name: String = row.get(1)?;
            let encrypted_value: Vec<u8> = row.get(2)?;
            let plaintext_value: String = row.get(3)?;
            let path: String = row.get(4)?;
            let is_secure: bool = row.get(5)?;
            let is_httponly: bool = row.get(6)?;
            Ok((
                host_key,
                name,
                encrypted_value,
                plaintext_value,
                path,
                is_secure,
                is_httponly,
            ))
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    for row in rows {
        let (host_key, name, encrypted_value, plaintext_value, path, is_secure, is_httponly) =
            match row {
                Ok(r) => r,
                Err(e) => {
                    debug!("Failed to read cookie row: {e}");
                    continue;
                }
            };

        // The plaintext column wins when populated; otherwise decrypt.
        let value = if plaintext_value.is_empty() {
            match decrypt_value(decryptor, &encrypted_value, meta_version) {
                Decrypted::Value(v) => v,
                Decrypted::AppBound => {
                    app_bound += 1;
                    continue;
                }
                Decrypted::Undecryptable => {
                    debug!("Failed to decrypt cookie: {name} for {host_key}");
                    continue;
                }
            }
        } else {
            plaintext_value
        };

        if value.is_empty() {
            continue;
        }

        if util::insert_cookie_into_jar(
            jar,
            &host_key,
            &name,
            &value,
            &path,
            is_secure,
            is_httponly,
        ) {
            count += 1;
        }
    }

    if app_bound > 0 {
        warn!(
            "Skipped {app_bound} Chrome cookie(s) protected by App-Bound Encryption (v20), \
             which external tools cannot decrypt. Export cookies to a Netscape file and use \
             --cookies <file> instead."
        );
    }

    debug!("Loaded {count} cookies from Chrome");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognizes_v10_v11_v20_and_plaintext() {
        assert!(matches!(
            classify(b"v10ciphertext"),
            Scheme::Encrypted {
                version: CipherVersion::V10,
                ..
            }
        ));
        assert!(matches!(
            classify(b"v11ciphertext"),
            Scheme::Encrypted {
                version: CipherVersion::V11,
                ..
            }
        ));
        assert!(matches!(classify(b"v20ciphertext"), Scheme::AppBound));
        assert!(matches!(classify(b"plain_value"), Scheme::Plaintext));
        // Too short to carry a prefix.
        assert!(matches!(classify(b"ab"), Scheme::Plaintext));
    }

    #[test]
    fn classify_splits_body_after_the_prefix() {
        match classify(b"v10BODYBYTES") {
            Scheme::Encrypted { body, .. } => assert_eq!(body, b"BODYBYTES"),
            _ => panic!("expected encrypted"),
        }
    }

    #[test]
    fn strip_hash_prefix_removes_32_bytes_at_version_24() {
        let plaintext = [vec![0xAA; HASH_PREFIX_LEN], b"value".to_vec()].concat();
        assert_eq!(strip_hash_prefix(plaintext, 24), b"value");
    }

    #[test]
    fn strip_hash_prefix_keeps_bytes_below_version_24() {
        // The load-bearing boundary: version 23 must NOT strip.
        let plaintext = [vec![0xAA; HASH_PREFIX_LEN], b"value".to_vec()].concat();
        let out = strip_hash_prefix(plaintext.clone(), 23);
        assert_eq!(out, plaintext);
    }

    #[test]
    fn strip_hash_prefix_leaves_short_buffer_untouched() {
        // Shorter than the hash prefix: return unchanged rather than truncate.
        let plaintext = b"short".to_vec();
        assert_eq!(strip_hash_prefix(plaintext.clone(), 24), plaintext);
    }
}
