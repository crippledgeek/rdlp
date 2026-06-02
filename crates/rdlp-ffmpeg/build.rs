//! Build script for `rdlp-ffmpeg` — FFmpeg linkage visibility (PR E-1).
//!
//! Detects when the user expressed intent to link against a custom FFmpeg
//! build via `PKG_CONFIG_PATH` / `PKG_CONFIG_LIBDIR` but the configured
//! path no longer contains `libavcodec.pc` (e.g. symlink dir vanished,
//! out-of-tree install was removed). In that case pkg-config silently
//! falls through to the distro FFmpeg, which previously had no signal at
//! all — non-free codecs would silently disappear from rdlp's encoder
//! list. We now emit a yellow `cargo:warning=` so the regression is
//! visible at `cargo build` time.
//!
//! Also bakes the resolved FFmpeg prefix into the binary via
//! `cargo:rustc-env=RDLP_FFMPEG_PREFIX=...` for future runtime
//! diagnostics (PR E-2 / E-3). Empty string is a valid value (means
//! pkg-config couldn't resolve it — we still want consumers to be able
//! to read the env via `env!()` without compile errors).
//!
//! Never fails the build — silent fallback to the distro FFmpeg is a
//! valid scenario for contributors without a custom FFmpeg installed.

use std::env;
use std::path::PathBuf;
use std::process::Command;

// Pure path-intent helper, shared verbatim with `tests/pkgconfig_intent.rs`.
// Brings `std::path::Path` and `std::ffi::OsStr` into scope (used below).
// `cargo test` never runs build scripts, so the branch logic lives in an
// `include!`d std-only file that a real test target can also compile.
include!("build_support/pkgconfig_intent.rs");

fn main() {
    // Re-run when the user's pkg-config configuration changes.
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    println!("cargo:rerun-if-changed=build.rs");

    // Intent: which env did the user (or .cargo/config.toml) set?
    let configured = env::var_os("PKG_CONFIG_PATH").or_else(|| env::var_os("PKG_CONFIG_LIBDIR"));

    if let Some(value) = configured.as_ref() {
        // The FIRST configured path is the user's intent target (typically a
        // custom FFmpeg build). Later paths are conventional fallbacks
        // (`/usr/lib/pkgconfig` etc.). If the first path doesn't contain
        // `libavcodec.pc`, the intent is broken — even when a later fallback
        // path has its own libavcodec.pc that hides the failure at the
        // pkg-config layer. Warn for THIS specific scenario; the silent
        // fall-through to distro FFmpeg still happens (intentional), but the
        // build output makes the regression visible.
        let separator = if cfg!(windows) { ';' } else { ':' };
        let broken = first_broken_prefix(value, separator, |p| p.join("libavcodec.pc").is_file());
        if let Some(first) = broken {
            println!(
                "cargo:warning=PKG_CONFIG_PATH/PKG_CONFIG_LIBDIR's first \
                 entry ({first}) does not contain libavcodec.pc — your \
                 custom FFmpeg build is missing or misconfigured at that \
                 location. Falling back to system FFmpeg (codec coverage \
                 may be reduced)."
            );
        }
    }

    // Resolve the prefix of the FFmpeg actually being linked.
    let prefix = pkg_config_variable("libavcodec", "prefix").unwrap_or_default();
    println!("cargo:rustc-env=RDLP_FFMPEG_PREFIX={prefix}");
}

/// Invoke `pkg-config --variable=<var> <pkg>` and return trimmed stdout.
/// Returns `None` if the command failed, exited non-zero, or output was empty.
fn pkg_config_variable(pkg: &str, var: &str) -> Option<String> {
    let pkg_config = env::var_os("PKG_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("pkg-config"));
    let output = Command::new(pkg_config)
        .arg(format!("--variable={var}"))
        .arg(pkg)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
