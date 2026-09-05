//! Shared utilities for cookie extraction modules.

use std::path::Path;

use log::debug;
use url::Url;
use wreq::Uri;
use wreq::cookie::CookieStore;
use wreq::header::HeaderValue;

/// Permission mode for the private temp directory holding a cookie DB copy:
/// owner read/write/execute only, so no other local user can read the copy.
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Attempts to find an unused unique name before giving up. Each attempt uses a
/// fresh counter + timestamp, so a collision is only possible under pathological
/// clock/pid reuse; the bound exists so a persistent failure cannot spin.
const MAX_PRIVATE_DIR_ATTEMPTS: u32 = 100;

/// Build a URL and `Set-Cookie` header from cookie fields, then insert into the jar.
///
/// Returns `true` if the cookie was successfully inserted.
#[allow(clippy::redundant_pub_crate)] // pub(crate) in private mod is more defensive than pub
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

    if Url::parse(&url_str).is_err() {
        debug!("Invalid URL from cookie domain: {url_str}");
        return false;
    }
    let Ok(uri) = url_str.parse::<Uri>() else {
        debug!("Invalid Uri from cookie domain: {url_str}");
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
            jar.set_cookies(&mut std::iter::once(&val), &uri);
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
/// Browsers lock their `SQLite` databases while running. Copying to a temp file
/// avoids `SQLITE_BUSY` / `SQLITE_LOCKED` errors.
///
/// On Windows, Chrome holds an exclusive lock via `LockFileEx`. If the
/// standard `fs::copy` fails with a permission/sharing error, this falls
/// back to opening the file with `FILE_SHARE_READ | FILE_SHARE_WRITE |
/// FILE_SHARE_DELETE` via Win32 `CreateFileW` to bypass the lock.
#[allow(clippy::redundant_pub_crate)] // pub(crate) in private mod is more defensive than pub
pub(crate) fn with_temp_db_copy<F, T>(
    db_path: &Path,
    temp_name: &str,
    f: F,
) -> Result<T, std::io::Error>
where
    F: FnOnce(&Path) -> Result<T, std::io::Error>,
{
    // A cookie DB copy is credential-bearing, so it is placed in a private,
    // uniquely-named directory rather than at a predictable path in the shared
    // temp dir: a fixed name lets another local user pre-create or read the
    // copy. The directory is created 0700 on Unix (see `create_private_temp_dir`).
    let temp_root = create_private_temp_dir()?;
    let temp_db = temp_root.join(temp_name);

    copy_db_file(db_path, &temp_db)?;

    // Browsers use SQLite WAL mode, so recent cookies may live in the -wal/-shm
    // sidecars. The source DB may carry a `.db` extension or none (Chrome's
    // "Cookies" has none), so try both the extension and suffix forms; the
    // destination is always "<temp_name>-wal"/"-shm" beside the copy.
    for suffix in ["wal", "shm"] {
        let dst = temp_root.join(format!("{temp_name}-{suffix}"));
        for src in sidecar_sources(db_path, suffix) {
            if src.exists() {
                let _ = copy_db_file(&src, &dst);
                break;
            }
        }
    }

    let result = f(&temp_db);

    // Remove the whole private directory and everything copied into it at once.
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_dir_all(&temp_root);

    result
}

/// The two candidate source paths for a `SQLite` `-wal`/`-shm` sidecar of
/// `db_path`: the `.db-<suffix>` extension form and the `<name>-<suffix>`
/// suffix form (Chrome's `Cookies` has no extension).
fn sidecar_sources(db_path: &Path, suffix: &str) -> [std::path::PathBuf; 2] {
    [
        db_path.with_extension(format!("db-{suffix}")),
        db_path.with_file_name(format!(
            "{}-{suffix}",
            db_path.file_name().unwrap_or_default().to_string_lossy()
        )),
    ]
}

/// Create a uniquely-named, owner-only directory under the system temp dir.
///
/// The name is unpredictable (pid + counter + timestamp) and creation is
/// exclusive: `create_new_dir` fails with `AlreadyExists` if the path is
/// present, so a successful create proves this process owns a fresh directory —
/// closing the symlink/pre-creation window a fixed temp name would open.
fn create_private_temp_dir() -> Result<std::path::PathBuf, std::io::Error> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let base = std::env::temp_dir();
    let pid = std::process::id();
    for _ in 0..MAX_PRIVATE_DIR_ATTEMPTS {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = base.join(format!("rdlp-cookies-{pid}-{seq}-{nanos}"));
        match create_new_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            // Name taken (extremely unlikely): fall through to the next attempt.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique temp directory for the cookie DB copy",
    ))
}

/// Create a single new directory, owner-only on Unix, failing if it exists.
#[cfg(unix)]
fn create_new_dir(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::DirBuilderExt;
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    std::fs::DirBuilder::new()
        .mode(PRIVATE_DIR_MODE)
        .create(path)
}

/// Create a single new directory, failing if it exists. On Windows the system
/// temp dir is already per-user, so no explicit mode is set.
#[cfg(not(unix))]
fn create_new_dir(path: &Path) -> Result<(), std::io::Error> {
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    std::fs::DirBuilder::new().create(path)
}

/// Copy a database file, with a Windows-specific fallback for locked files.
fn copy_db_file(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    match std::fs::copy(src, dst) {
        Ok(_) => Ok(()),
        #[cfg(target_os = "windows")]
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied
            || e.raw_os_error() == Some(32) /* ERROR_SHARING_VIOLATION */ =>
        {
            debug!("Standard copy failed ({e}), trying Win32 share-read fallback");
            copy_with_share_read(src, dst)
        }
        Err(e) => Err(e),
    }
}

/// Windows-only: open the source file with full sharing mode and copy
/// its contents manually. This succeeds when Chrome holds a `LockFileEx`
/// byte-range lock but opened the file with `FILE_SHARE_READ`.
///
/// The workspace-level `disallowed_methods` lint flags `std::fs::File::create`
/// because it's blocking in async contexts. This function is fully synchronous
/// (sync `Read`/`Write` throughout, called only from sync cookie-extraction
/// paths) so the lint doesn't apply — explicitly allowed here.
#[cfg(target_os = "windows")]
#[allow(clippy::disallowed_methods)]
fn copy_with_share_read(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    use std::io::{Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };

    // GENERIC_READ = 0x8000_0000 — not re-exported by windows-sys FileSystem
    const GENERIC_READ: u32 = 0x8000_0000;

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
        // `Read::read` contract guarantees `n <= buf.len()`, so the slice cannot panic.
        #[allow(clippy::indexing_slicing)]
        dst_file.write_all(&buf[..n])?;
    }
    dst_file.flush()?;

    Ok(())
}

/// Read the `HOME` environment variable, returning an `io::Error` if unset.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn home_dir() -> Result<std::path::PathBuf, std::io::Error> {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME not set"))
}
