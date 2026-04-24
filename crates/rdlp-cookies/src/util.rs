//! Shared utilities for cookie extraction modules.

use std::path::Path;

use log::debug;
use wreq::cookie::CookieStore;
use wreq::header::HeaderValue;
use url::Url;

/// Build a URL and `Set-Cookie` header from cookie fields, then insert into the jar.
///
/// Returns `true` if the cookie was successfully inserted.
pub(crate) fn insert_cookie_into_jar(
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
        debug!("Invalid URL from cookie domain: {url_str}");
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
            debug!("Invalid cookie header value for {name}: {e}");
            false
        }
    }
}

/// Copy a database file to a temp location, run a callback, then clean up.
///
/// Browsers lock their SQLite databases while running. Copying to a temp file
/// avoids `SQLITE_BUSY` / `SQLITE_LOCKED` errors.
///
/// On Windows, Chrome holds an exclusive lock via `LockFileEx`. If the
/// standard `fs::copy` fails with a permission/sharing error, this falls
/// back to opening the file with `FILE_SHARE_READ | FILE_SHARE_WRITE |
/// FILE_SHARE_DELETE` via Win32 `CreateFileW` to bypass the lock.
pub(crate) fn with_temp_db_copy<F, T>(
    db_path: &Path,
    temp_name: &str,
    f: F,
) -> Result<T, std::io::Error>
where
    F: FnOnce(&Path) -> Result<T, std::io::Error>,
{
    let temp_db = std::env::temp_dir().join(temp_name);

    copy_db_file(db_path, &temp_db)?;

    // Also copy WAL and SHM journal files if they exist. Chrome uses
    // SQLite WAL mode, so recent cookies may live in the journal.
    let wal_src = db_path.with_extension("db-wal");
    let shm_src = db_path.with_extension("db-shm");
    let wal_dst = temp_db.with_extension("db-wal");
    let shm_dst = temp_db.with_extension("db-shm");

    // Chrome's cookie DB doesn't have a .db extension, so use suffix
    // approach: Cookies-wal, Cookies-shm
    let wal_src2 = db_path.with_file_name(format!(
        "{}-wal",
        db_path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let shm_src2 = db_path.with_file_name(format!(
        "{}-shm",
        db_path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let wal_dst2 = temp_db.with_file_name(format!("{}-wal", temp_name));
    let shm_dst2 = temp_db.with_file_name(format!("{}-shm", temp_name));

    // Try both naming patterns; ignore errors (files may not exist)
    for (src, dst) in [
        (&wal_src, &wal_dst),
        (&shm_src, &shm_dst),
        (&wal_src2, &wal_dst2),
        (&shm_src2, &shm_dst2),
    ] {
        if src.exists() {
            let _ = copy_db_file(src, dst);
        }
    }

    let result = f(&temp_db);

    // Clean up all temp files
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(&temp_db);
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(&wal_dst);
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(&shm_dst);
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(&wal_dst2);
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(&shm_dst2);

    result
}

/// Copy a database file, with a Windows-specific fallback for locked files.
fn copy_db_file(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    match std::fs::copy(src, dst) {
        Ok(_) => Ok(()),
        #[cfg(target_os = "windows")]
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.raw_os_error() == Some(32) /* ERROR_SHARING_VIOLATION */ =>
        {
            debug!(
                "Standard copy failed ({}), trying Win32 share-read fallback",
                e
            );
            copy_with_share_read(src, dst)
        }
        Err(e) => Err(e),
    }
}

/// Windows-only: open the source file with full sharing mode and copy
/// its contents manually. This succeeds when Chrome holds a `LockFileEx`
/// byte-range lock but opened the file with `FILE_SHARE_READ`.
#[cfg(target_os = "windows")]
fn copy_with_share_read(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    use std::io::{Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };

    // GENERIC_READ = 0x80000000 — not re-exported by windows-sys FileSystem
    const GENERIC_READ: u32 = 0x80000000;

    // Convert path to wide string with null terminator
    let wide_path: Vec<u16> = src
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    // Safety: handle is valid and we take ownership.
    // On Windows, HANDLE is isize which maps to *mut c_void for
    // FromRawHandle.
    let mut src_file = unsafe { std::fs::File::from_raw_handle(handle) };
    let mut dst_file = std::fs::File::create(dst)?;

    let mut buf = vec![0u8; 256 * 1024]; // 256 KB buffer
    loop {
        let n = src_file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst_file.write_all(&buf[..n])?;
    }
    dst_file.flush()?;

    Ok(())
}

/// Read the `HOME` environment variable, returning an `io::Error` if unset.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn home_dir() -> Result<std::path::PathBuf, std::io::Error> {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set"))
}
