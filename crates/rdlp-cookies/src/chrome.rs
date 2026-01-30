//! Chrome/Chromium cookie extraction.
//!
//! Extracts cookies from Chrome's SQLite database and decrypts them.
//! On Windows, uses DPAPI + AES-256-GCM (Chrome v80+).

use std::path::{Path, PathBuf};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use log::{debug, warn};
use reqwest::cookie::CookieStore;
use reqwest::header::HeaderValue;
use url::Url;

/// AES-GCM nonce length (96 bits / 12 bytes).
const NONCE_LEN: usize = 12;

/// Chrome v10+ encrypted value prefix.
const V10_PREFIX: &[u8] = b"v10";

/// Extract cookies from Chrome and insert them into the jar.
///
/// Returns the number of cookies loaded.
pub fn extract_cookies(jar: &impl CookieStore) -> Result<usize, std::io::Error> {
    let cookie_db = find_cookie_db()?;
    let local_state = find_local_state()?;

    debug!("Chrome cookie DB: {}", cookie_db.display());
    debug!("Chrome Local State: {}", local_state.display());

    // Get the AES key from Local State (DPAPI-encrypted)
    let key = load_encryption_key(&local_state)?;

    // Copy the DB to a temp file to avoid locking issues
    let temp_dir = std::env::temp_dir();
    let temp_db = temp_dir.join("rdlp_chrome_cookies.db");
    std::fs::copy(&cookie_db, &temp_db)?;

    let result = read_cookies_from_db(&temp_db, &key, jar);

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_db);

    result
}

/// Find Chrome's cookie database path.
fn find_cookie_db() -> Result<PathBuf, std::io::Error> {
    let path = chrome_user_data_dir()?.join("Default").join("Cookies");
    if path.exists() {
        Ok(path)
    } else {
        // Try Network/Cookies (Chrome 96+)
        let network_path = chrome_user_data_dir()?.join("Default").join("Network").join("Cookies");
        if network_path.exists() {
            Ok(network_path)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Chrome cookie database not found at {} or {}", path.display(), network_path.display()),
            ))
        }
    }
}

/// Find Chrome's Local State file (contains the encryption key).
fn find_local_state() -> Result<PathBuf, std::io::Error> {
    let path = chrome_user_data_dir()?.join("Local State");
    if path.exists() {
        Ok(path)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Chrome Local State file not found",
        ))
    }
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
        let home = std::env::var("HOME").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set")
        })?;
        Ok(PathBuf::from(home)
            .join(".config")
            .join("google-chrome"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set")
        })?;
        Ok(PathBuf::from(home)
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

/// Load and decrypt the AES encryption key from Local State.
fn load_encryption_key(local_state_path: &Path) -> Result<Vec<u8>, std::io::Error> {
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

    // Base64 decode
    let encrypted_key = BASE64.decode(encrypted_key_b64).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Base64 decode error: {e}"))
    })?;

    // Strip "DPAPI" prefix (5 bytes)
    if encrypted_key.len() < 5 || &encrypted_key[..5] != b"DPAPI" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Encrypted key missing DPAPI prefix",
        ));
    }
    let dpapi_encrypted = &encrypted_key[5..];

    // Decrypt with platform-specific method
    decrypt_dpapi_key(dpapi_encrypted)
}

/// Decrypt a DPAPI-encrypted key.
#[cfg(target_os = "windows")]
fn decrypt_dpapi_key(encrypted: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

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

#[cfg(not(target_os = "windows"))]
fn decrypt_dpapi_key(_encrypted: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Chrome DPAPI decryption is only supported on Windows. Use --cookies with a Netscape cookie file instead.",
    ))
}

/// Decrypt a Chrome AES-256-GCM encrypted cookie value.
fn decrypt_cookie_value(encrypted_value: &[u8], key: &[u8]) -> Option<String> {
    if encrypted_value.is_empty() {
        return None;
    }

    // Check for v10/v11 prefix (Chrome 80+)
    if encrypted_value.len() < V10_PREFIX.len() + NONCE_LEN + 1 {
        // Might be plaintext or old format
        return String::from_utf8(encrypted_value.to_vec()).ok();
    }

    let prefix = &encrypted_value[..V10_PREFIX.len()];
    if prefix != V10_PREFIX && prefix != b"v11" {
        // Not AES-encrypted, try as plaintext
        return String::from_utf8(encrypted_value.to_vec()).ok();
    }

    let encrypted_data = &encrypted_value[V10_PREFIX.len()..];
    if encrypted_data.len() < NONCE_LEN + 1 {
        return None;
    }

    let nonce_bytes = &encrypted_data[..NONCE_LEN];
    let ciphertext = &encrypted_data[NONCE_LEN..];

    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

/// Read cookies from the SQLite database and insert them into the jar.
fn read_cookies_from_db(
    db_path: &Path,
    key: &[u8],
    jar: &impl CookieStore,
) -> Result<usize, std::io::Error> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT host_key, name, encrypted_value, value, path, is_secure, is_httponly \
             FROM cookies",
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut count = 0;

    let rows = stmt
        .query_map([], |row| {
            let host_key: String = row.get(0)?;
            let name: String = row.get(1)?;
            let encrypted_value: Vec<u8> = row.get(2)?;
            let plaintext_value: String = row.get(3)?;
            let path: String = row.get(4)?;
            let is_secure: bool = row.get(5)?;
            let is_httponly: bool = row.get(6)?;
            Ok((host_key, name, encrypted_value, plaintext_value, path, is_secure, is_httponly))
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    for row in rows {
        let (host_key, name, encrypted_value, plaintext_value, path, is_secure, is_httponly) =
            match row {
                Ok(r) => r,
                Err(e) => {
                    warn!("Failed to read cookie row: {e}");
                    continue;
                }
            };

        // Decrypt the value
        let value = if !plaintext_value.is_empty() {
            plaintext_value
        } else {
            match decrypt_cookie_value(&encrypted_value, key) {
                Some(v) => v,
                None => {
                    debug!("Failed to decrypt cookie: {name} for {host_key}");
                    continue;
                }
            }
        };

        if value.is_empty() {
            continue;
        }

        // Build URL and Set-Cookie header
        let scheme = if is_secure { "https" } else { "http" };
        let host = host_key.trim_start_matches('.');
        let url_str = format!("{scheme}://{host}{path}");

        let Ok(url) = Url::parse(&url_str) else {
            continue;
        };

        let mut set_cookie = format!("{name}={value}; Domain={host_key}; Path={path}");
        if is_secure {
            set_cookie.push_str("; Secure");
        }
        if is_httponly {
            set_cookie.push_str("; HttpOnly");
        }

        if let Ok(val) = HeaderValue::from_str(&set_cookie) {
            jar.set_cookies(&mut std::iter::once(&val), &url);
            count += 1;
        }
    }

    debug!("Loaded {count} cookies from Chrome");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decrypt_plaintext_value() {
        let value = b"plain_value";
        let result = decrypt_cookie_value(value, &[0u8; 32]);
        assert_eq!(result, Some("plain_value".to_string()));
    }

    #[test]
    fn test_decrypt_empty_value() {
        let result = decrypt_cookie_value(&[], &[0u8; 32]);
        assert!(result.is_none());
    }

    #[test]
    fn test_decrypt_too_short_v10_falls_back_to_plaintext() {
        // v10 prefix but too short for nonce + ciphertext — falls back to plaintext attempt
        let mut data = Vec::from(V10_PREFIX);
        data.extend_from_slice(&[b'x'; 5]);
        let result = decrypt_cookie_value(&data, &[0u8; 32]);
        // Falls through to plaintext path since too short for AES-GCM
        assert_eq!(result, Some("v10xxxxx".to_string()));
    }
}
