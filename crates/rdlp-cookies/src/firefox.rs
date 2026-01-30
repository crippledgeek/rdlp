//! Firefox cookie extraction.
//!
//! Extracts cookies from Firefox's SQLite database.
//! Firefox stores cookies in plaintext (no encryption needed).

use std::path::{Path, PathBuf};

use log::{debug, warn};
use reqwest::cookie::CookieStore;
use reqwest::header::HeaderValue;
use url::Url;

use crate::util;

/// Extract cookies from Firefox and insert them into the jar.
///
/// Returns the number of cookies loaded.
pub fn extract_cookies(jar: &impl CookieStore) -> Result<usize, std::io::Error> {
    let cookie_db = find_cookie_db()?;
    debug!("Firefox cookie DB: {}", cookie_db.display());

    // Copy the DB to a temp file to avoid locking issues
    let temp_dir = std::env::temp_dir();
    let temp_db = temp_dir.join("rdlp_firefox_cookies.db");
    std::fs::copy(&cookie_db, &temp_db)?;

    let result = read_cookies_from_db(&temp_db, jar);

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_db);

    result
}

/// Find Firefox's cookie database.
fn find_cookie_db() -> Result<PathBuf, std::io::Error> {
    let profile_dir = find_default_profile()?;
    let db_path = profile_dir.join("cookies.sqlite");

    if db_path.exists() {
        Ok(db_path)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Firefox cookies.sqlite not found at {}",
                db_path.display()
            ),
        ))
    }
}

/// Find the default Firefox profile directory.
fn find_default_profile() -> Result<PathBuf, std::io::Error> {
    let profiles_dir = firefox_profiles_dir()?;

    if !profiles_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Firefox profiles directory not found: {}",
                profiles_dir.display()
            ),
        ));
    }

    // Try profiles.ini first
    let profiles_ini = profiles_dir.join("profiles.ini");
    if profiles_ini.exists() {
        if let Some(profile) = parse_profiles_ini(&profiles_ini, &profiles_dir) {
            return Ok(profile);
        }
    }

    // Fallback: scan directories for preferred profile suffixes, then any profile
    let suffixes = [".default-release", ".default"];
    if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
        let dirs: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("cookies.sqlite").exists())
            .collect();

        // Try preferred suffixes first
        for suffix in &suffixes {
            if let Some(dir) = dirs.iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(suffix))
            }) {
                return Ok(dir.clone());
            }
        }

        // Any profile with cookies
        if let Some(dir) = dirs.into_iter().next() {
            return Ok(dir);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "No Firefox profile with cookies.sqlite found",
    ))
}

/// Parse profiles.ini to find the default profile path.
fn parse_profiles_ini(ini_path: &Path, profiles_dir: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(ini_path).ok()?;

    let try_commit = |path: &Option<String>, is_relative: bool| -> Option<PathBuf> {
        let path = path.as_ref()?;
        let profile_path = if is_relative {
            profiles_dir.join(path)
        } else {
            PathBuf::from(path)
        };
        profile_path.join("cookies.sqlite").exists().then_some(profile_path)
    };

    let mut current_path: Option<String> = None;
    let mut current_is_relative = true;
    let mut current_is_default = false;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            // Commit previous section if it was the default
            if current_is_default {
                if let Some(ref path) = current_path {
                    let profile_path = if current_is_relative {
                        profiles_dir.join(path)
                    } else {
                        PathBuf::from(path)
                    };
                    if profile_path.join("cookies.sqlite").exists() {
                        return Some(profile_path);
                    }
                }
                }
            }

            // Reset for new section
            current_path = None;
            current_is_relative = true;
            current_is_default = false;
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "Path" => current_path = Some(value.trim().to_string()),
                "IsRelative" => current_is_relative = value.trim() == "1",
                "Default" => current_is_default = value.trim() == "1",
                _ => {}
            }
        }
    }

    // Check last section
    if current_is_default {
        if let Some(ref path) = current_path {
            let profile_path = if current_is_relative {
                profiles_dir.join(path)
            } else {
                PathBuf::from(path)
            };
            if profile_path.join("cookies.sqlite").exists() {
                return Some(profile_path);
            }
        }
    }

    None
}

/// Get Firefox profiles directory.
fn firefox_profiles_dir() -> Result<PathBuf, std::io::Error> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "APPDATA not set")
        })?;
        Ok(PathBuf::from(appdata)
            .join("Mozilla")
            .join("Firefox")
            .join("Profiles"))
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set")
        })?;
        Ok(PathBuf::from(home).join(".mozilla").join("firefox"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set")
        })?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Firefox")
            .join("Profiles"))
            .join("Library")
            .join("Application Support")
            .join("Firefox")
            .join("Profiles"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Firefox cookie extraction not supported on this platform",
        ))
    }
}

/// Read cookies from the SQLite database and insert them into the jar.
fn read_cookies_from_db(
    db_path: &Path,
    jar: &impl CookieStore,
) -> Result<usize, std::io::Error> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT host, name, value, path, isSecure, isHttpOnly \
             FROM moz_cookies",
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut count = 0;

    let rows = stmt
        .query_map([], |row| {
            let host: String = row.get(0)?;
            let name: String = row.get(1)?;
            let value: String = row.get(2)?;
            let path: String = row.get(3)?;
            let is_secure: bool = row.get(4)?;
            let is_httponly: bool = row.get(5)?;
            Ok((host, name, value, path, is_secure, is_httponly))
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    for row in rows {
        let (host, name, value, path, is_secure, is_httponly) = match row {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to read Firefox cookie row: {e}");
                continue;
            }
        };

        if name.is_empty() || value.is_empty() {
            continue;
        }

        let scheme = if is_secure { "https" } else { "http" };
        let clean_host = host.trim_start_matches('.');
        let url_str = format!("{scheme}://{clean_host}{path}");

        let Ok(url) = Url::parse(&url_str) else {
            continue;
        };

        let mut set_cookie = format!("{name}={value}; Domain={host}; Path={path}");
        if is_secure {
            set_cookie.push_str("; Secure");
        }
        if is_httponly {
            set_cookie.push_str("; HttpOnly");
        }

        if let Ok(val) = HeaderValue::from_str(&set_cookie) {
            jar.set_cookies(&mut std::iter::once(&val), &url);
        }
            count += 1;
        }
    }

    debug!("Loaded {count} cookies from Firefox");
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_profiles_ini() {
        let dir = std::env::temp_dir().join("rdlp_test_firefox_profiles");
        let _ = std::fs::create_dir_all(&dir);

        // Create a fake profile directory with cookies.sqlite
        let profile_dir = dir.join("abc123.default-release");
        let _ = std::fs::create_dir_all(&profile_dir);
        std::fs::write(profile_dir.join("cookies.sqlite"), b"fake").unwrap();

        // Write a profiles.ini
        let ini_path = dir.join("profiles.ini");
        let ini_content = "\
[Profile0]
Name=default-release
IsRelative=1
Path=abc123.default-release
Default=1
";
        std::fs::write(&ini_path, ini_content).unwrap();

        let result = parse_profiles_ini(&ini_path, &dir);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), profile_dir);

        // Clean up
        let _ = std::fs::remove_dir_all(&dir);
    }
}
