//! Shared utilities for cookie extraction modules.

use std::path::Path;

use log::warn;
use reqwest::cookie::CookieStore;
use reqwest::header::HeaderValue;
use url::Url;

/// Build a URL and `Set-Cookie` header from cookie fields, then insert into the jar.
///
/// Returns `true` if the cookie was successfully inserted.
pub fn insert_cookie_into_jar(
    jar: &impl CookieStore,
    domain: &str,
    name: &str,
    value: &str,
    path: &str,
    secure: bool,
    httponly: bool,
) -> bool {
    let scheme = if secure { "https" } else { "http" };
    let host = domain.trim_start_matches('.');
    let url_str = format!("{scheme}://{host}{path}");

    let Ok(url) = Url::parse(&url_str) else {
        warn!("Invalid URL from cookie domain: {url_str}");
        return false;
    };

    let mut set_cookie = format!("{name}={value}; Domain={domain}; Path={path}");
    if secure {
        set_cookie.push_str("; Secure");
    }
    if httponly {
        set_cookie.push_str("; HttpOnly");
    }

    match HeaderValue::from_str(&set_cookie) {
        Ok(val) => {
            jar.set_cookies(&mut std::iter::once(&val), &url);
            true
        }
        Err(e) => {
            warn!("Invalid cookie header value for {name}: {e}");
            false
        }
    }
}

/// Copy a database file to a temp location, run a callback, then clean up.
///
/// Browsers lock their SQLite databases while running. Copying to a temp file
/// avoids `SQLITE_BUSY` / `SQLITE_LOCKED` errors.
pub fn with_temp_db_copy<F, T>(
    db_path: &Path,
    temp_name: &str,
    f: F,
) -> Result<T, std::io::Error>
where
    F: FnOnce(&Path) -> Result<T, std::io::Error>,
{
    let temp_db = std::env::temp_dir().join(temp_name);
    std::fs::copy(db_path, &temp_db)?;

    let result = f(&temp_db);

    let _ = std::fs::remove_file(&temp_db);

    result
}

/// Read the `HOME` environment variable, returning an `io::Error` if unset.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn home_dir() -> Result<std::path::PathBuf, std::io::Error> {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set"))
}
