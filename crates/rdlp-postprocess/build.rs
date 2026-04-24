//! Build script for rdlp-postprocess.
//!
//! Auto-detects FFmpeg shared builds on Windows for the `ffmpeg-the-third`
//! dev-dependency. Sets `FFMPEG_DETECTED_DIR` so the crate can locate FFmpeg
//! at compile time.
//!
//! # FFmpeg resolution order
//!
//! 1. `FFMPEG_DIR` environment variable (explicit override)
//! 2. WinGet package paths (`Gyan.FFmpeg.Shared`)
//! 3. Chocolatey FFmpeg installation
//! 4. Derive from `ffmpeg.exe` found in `PATH`

// Safe: build.rs runs synchronously at compile time — no async runtime exists.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");

    if let Some(dir) = detect_ffmpeg() {
        let dir_str = dir.to_string_lossy();
        println!("cargo:rustc-env=FFMPEG_DETECTED_DIR={dir_str}");
        // Also set FFMPEG_DIR so ffmpeg-sys-the-third picks it up
        // (only works if this build script runs before ffmpeg-sys-the-third,
        //  which it doesn't - use .cargo/config.toml for that)
        println!("cargo:ffmpeg_dir={dir_str}");
    }
}

/// Detect FFmpeg shared build directory.
///
/// Returns the path to a directory containing `include/` and `lib/` subdirs.
fn detect_ffmpeg() -> Option<PathBuf> {
    // 1. Explicit FFMPEG_DIR takes priority
    if let Ok(dir) = std::env::var("FFMPEG_DIR") {
        let path = PathBuf::from(&dir);
        if is_valid_ffmpeg_dir(&path) {
            return Some(path);
        }
        println!("cargo:warning=FFMPEG_DIR={dir} is set but missing include/ or lib/");
    }

    // 2. Derive from PKG_CONFIG_PATH (finds custom FFmpeg builds on Linux)
    if let Some(dir) = derive_from_pkg_config_path() {
        eprintln!("FFmpeg detected via PKG_CONFIG_PATH: {}", dir.display());
        return Some(dir);
    }

    // 3. Search common installation paths (Windows)
    if cfg!(target_os = "windows")
        && let Some(dir) = search_windows_paths()
    {
        eprintln!("FFmpeg auto-detected at: {}", dir.display());
        return Some(dir);
    }

    // 4. Derive from ffmpeg in PATH
    if let Some(dir) = derive_from_path() {
        eprintln!("FFmpeg derived from PATH: {}", dir.display());
        return Some(dir);
    }

    println!(
        "cargo:warning=FFmpeg shared build not found. Set FFMPEG_DIR or install via: winget install Gyan.FFmpeg.Shared"
    );
    None
}

/// Check if a directory has the expected FFmpeg structure (include/ + lib/).
fn is_valid_ffmpeg_dir(path: &Path) -> bool {
    path.join("include").join("libavcodec").is_dir() && path.join("lib").is_dir()
}

/// Search common Windows installation paths for FFmpeg shared builds.
fn search_windows_paths() -> Option<PathBuf> {
    let local_app_data = std::env::var("LOCALAPPDATA").ok()?;

    // WinGet: Gyan.FFmpeg.Shared
    let winget_base = PathBuf::from(&local_app_data)
        .join("Microsoft")
        .join("WinGet")
        .join("Packages");

    if winget_base.is_dir()
        && let Ok(entries) = std::fs::read_dir(&winget_base)
    {
        // Look for Gyan.FFmpeg.Shared_* directories
        let mut ffmpeg_dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("Gyan.FFmpeg.Shared"))
            })
            .collect();

        // Sort to get newest version last
        ffmpeg_dirs.sort();

        for winget_dir in ffmpeg_dirs.iter().rev() {
            // Inside WinGet dir, find the ffmpeg-*-shared subdirectory
            if let Ok(inner) = std::fs::read_dir(winget_dir) {
                for entry in inner.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() && is_valid_ffmpeg_dir(&path) {
                        return Some(path);
                    }
                }
            }
        }
    }

    // Chocolatey: C:\ProgramData\chocolatey\lib\ffmpeg-shared
    let choco_paths = [
        PathBuf::from(r"C:\ProgramData\chocolatey\lib\ffmpeg-shared\tools\ffmpeg"),
        PathBuf::from(r"C:\ProgramData\chocolatey\lib\ffmpeg\tools\ffmpeg"),
    ];
    for path in &choco_paths {
        if is_valid_ffmpeg_dir(path) {
            return Some(path.clone());
        }
    }

    // Program Files
    let program_files = [
        std::env::var("ProgramFiles").unwrap_or_default(),
        std::env::var("ProgramFiles(x86)").unwrap_or_default(),
    ];
    for pf in &program_files {
        if pf.is_empty() {
            continue;
        }
        let ffmpeg_dir = PathBuf::from(pf).join("FFmpeg");
        if is_valid_ffmpeg_dir(&ffmpeg_dir) {
            return Some(ffmpeg_dir);
        }
    }

    None
}

/// Try to derive the FFmpeg directory from `PKG_CONFIG_PATH`.
///
/// Searches each directory in `PKG_CONFIG_PATH` for `libavcodec.pc` and derives
/// the parent directory (e.g., `.../lib/pkgconfig/libavcodec.pc` → `...`).
fn derive_from_pkg_config_path() -> Option<PathBuf> {
    let pkg_path = std::env::var("PKG_CONFIG_PATH").ok()?;

    for dir in pkg_path.split(':') {
        let pc_file = PathBuf::from(dir).join("libavcodec.pc");
        if pc_file.is_file() {
            // .../lib/pkgconfig -> .../lib -> ...
            if let Some(pkgconfig_dir) = pc_file.parent()
                && let Some(lib_dir) = pkgconfig_dir.parent()
                && let Some(parent) = lib_dir.parent()
                && is_valid_ffmpeg_dir(parent)
            {
                return Some(parent.to_path_buf());
            }
        }
    }

    None
}

/// Try to find ffmpeg.exe in PATH and derive the shared build directory.
///
/// If ffmpeg.exe is at `.../bin/ffmpeg.exe`, check if `../` has include/ and lib/.
fn derive_from_path() -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let separator = if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    };

    for dir in path_var.split(separator) {
        let ffmpeg_exe = PathBuf::from(dir).join(if cfg!(target_os = "windows") {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });

        if ffmpeg_exe.is_file() {
            // Go from .../bin/ffmpeg.exe -> .../
            if let Some(bin_dir) = ffmpeg_exe.parent()
                && let Some(parent) = bin_dir.parent()
                && is_valid_ffmpeg_dir(parent)
            {
                return Some(parent.to_path_buf());
            }
        }
    }

    None
}
