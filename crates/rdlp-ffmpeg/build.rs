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
//! Both answers are tied to the `libavcodec.pc` they are read from via
//! `cargo:rerun-if-changed` — the configured entry's, and the one pkg-config
//! actually resolved. Without that, Cargo replays a cached verdict when the
//! prefix changes underneath it (#655).
//!
//! Never fails the build — silent fallback to the distro FFmpeg is a
//! valid scenario for contributors without a custom FFmpeg installed.

use std::env;
use std::path::PathBuf;
use std::process::Command;

// Pure path-intent helpers, shared verbatim with `tests/pkgconfig_intent.rs`.
// Brings `std::path::Path` and `std::ffi::OsStr` into scope for the spliced
// helpers below; `build.rs` itself never names either after the include.
// `cargo test` never runs build scripts, so the branch logic lives in an
// `include!`d std-only file that a real test target can also compile.
include!("build_support/pkgconfig_intent.rs");

fn main() {
    // Re-run when the user's pkg-config configuration changes.
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    // `pkg_config_variable` reads PKG_CONFIG to pick the binary. Emitting any
    // `rerun-if-*` turns off Cargo's default "rerun on any package change", so
    // an env var a build script reads but does not declare is untracked.
    // Switching this (pkgconf vs pkg-config, a cross-compile wrapper) changes
    // both the baked prefix and the resolved watch below — #655's own defect
    // class, on #655's own line of output.
    println!("cargo:rerun-if-env-changed=PKG_CONFIG");
    println!("cargo:rerun-if-changed=build.rs");

    // Intent: which env did the user (or .cargo/config.toml) set?
    let configured = env::var_os("PKG_CONFIG_PATH").or_else(|| env::var_os("PKG_CONFIG_LIBDIR"));
    let separator = if cfg!(windows) { ';' } else { ':' };

    // Every `.pc` this script's answers are read from, emitted at one site at
    // the end. Collected rather than printed inline because the two sources
    // below name the SAME file in the common healthy case. The dedup is
    // textual, so it collapses only byte-identical paths — a trailing slash on
    // one side still yields two directives for one file. That is harmless
    // (Cargo tolerates duplicates); what the collection actually buys is one
    // emission site and a deterministic order, not a correctness fix.
    let mut watched: Vec<PathBuf> = Vec::new();

    if let Some(value) = configured.as_ref() {
        // The FIRST configured path is the user's intent target (typically a
        // custom FFmpeg build). Later paths are conventional fallbacks
        // (`/usr/lib/pkgconfig` etc.). If the first path doesn't contain
        // `libavcodec.pc`, the intent is broken — even when a later fallback
        // path has its own libavcodec.pc that hides the failure at the
        // pkg-config layer. Warn for THIS specific scenario; the silent
        // fall-through to distro FFmpeg still happens (intentional), but the
        // build output makes the regression visible.
        // Tie the misconfiguration verdict to the file it is read from — see
        // `avcodec_watch_path` (#655).
        watched.extend(avcodec_watch_path(value, separator));

        // A refused entry would otherwise produce no directive and no warning,
        // reading exactly like "nothing configured". Say so — without echoing
        // the value, which is what made it unusable in the first place.
        if first_entry_refused(value, separator) {
            println!(
                "cargo:warning=PKG_CONFIG_PATH/PKG_CONFIG_LIBDIR's first \
                 entry contains a control character and was ignored. Fix the \
                 variable; the path is not echoed here because printing it \
                 would corrupt this build's output."
            );
        }

        if let Some(first) = first_broken_prefix(value, separator, |p| p.join(AVCODEC_PC).is_file())
        {
            let first = first.as_str();
            println!(
                "cargo:warning=PKG_CONFIG_PATH/PKG_CONFIG_LIBDIR's first \
                 entry ({first}) does not contain libavcodec.pc — your \
                 custom FFmpeg build is missing or misconfigured at that \
                 location. Falling back to system FFmpeg (codec coverage \
                 may be reduced)."
            );
        }
    }

    // Resolve the prefix of the FFmpeg actually being linked. This is a
    // `cargo:` directive built from subprocess output like the watch below, so
    // it takes the same guard; an unusable value degrades to empty rather than
    // dropping the line, because the module contract is that `env!()` always
    // has something to read.
    let prefix = pkg_config_variable("libavcodec", "prefix")
        .as_deref()
        .and_then(DirectiveSafe::new);
    let prefix = prefix.as_ref().map_or("", DirectiveSafe::as_str);
    println!("cargo:rustc-env=RDLP_FFMPEG_PREFIX={prefix}");

    // The watch above covers the first CONFIGURED entry — the one whose
    // breakage this script reports. It does not cover where pkg-config
    // actually RESOLVED, and the two differ whenever no `PKG_CONFIG_*` is set
    // (the contributor-on-distro-FFmpeg case), the first entry was refused, or
    // the answer came from a later entry. In all three the baked prefix above
    // had no declared input and replayed indefinitely — #655's defect on
    // #655's own line of output. `pcfiledir` names the directory holding the
    // `.pc` pkg-config really used, so watching the file inside it ties the
    // baked prefix to its own source.
    //
    // Two branches here emit nothing. pkg-config not answering at all (FFmpeg
    // absent, or no usable `pkg-config` binary) leaves the empty prefix
    // undeclared — the link fails anyway, so it is left rather than guessed at.
    // A `pcfiledir` refused by the guard also emits nothing, and unlike the
    // configured-entry path it gets no warning: that value is pkg-config's own
    // report of a directory it just read a file from, not operator input, so a
    // control character in it is not a misconfiguration a warning could help
    // with.
    watched.extend(
        pkg_config_variable("libavcodec", "pcfiledir")
            .as_deref()
            .and_then(DirectiveSafe::new)
            .map(|dir| watch_path_in(&dir)),
    );

    watched.sort();
    watched.dedup();
    for pc in watched {
        println!("cargo:rerun-if-changed={}", pc.display());
    }
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
