// Shared, self-contained (std-only) helpers `include!`d by BOTH `build.rs`
// (at build time) and `tests/pkgconfig_intent.rs` (at test time). Build
// scripts are a separate compilation unit that `cargo test` never executes,
// so the only way to unit-test build-script logic is to factor the pure part
// into a file like this and `include!` it from a real test target.
//
// Keep this file dependency-free beyond `std` — both includers compile it
// independently — and free of Cargo's build-script output EMISSION: these
// helpers answer questions about paths, including whether one can be named in
// a directive, and `build.rs` does all the printing.
//
// NOTE for anyone editing: `rustfmt` does not descend into `include!`d files,
// so `cargo fmt` and `cargo fmt --check` are both blind to this one.
// `scripts/check-build-support-fmt.sh` covers it instead.

use std::ffi::OsStr;
use std::path::Path;

/// Wrapped in a real `mod` so the tuple field is genuinely unreachable from
/// the includer. `include!` splices tokens into the includer's own module, so
/// a bare `struct DirectiveSafe(String)` here would leave the field
/// constructible from `build.rs` as `DirectiveSafe("evil\n".into())` — the
/// invariant would be convention rather than enforcement. This boundary is
/// what makes "cannot be built without passing the guard" true.
mod directive_safe {
    /// Whether `s` can be interpolated into a Cargo build-script directive.
    ///
    /// Exposed alongside [`DirectiveSafe`] so a caller that only wants the
    /// question answered does not have to allocate a value to throw away.
    pub(crate) fn is_directive_safe(s: &str) -> bool {
        !s.contains(char::is_control)
    }

    /// A string proven safe to interpolate into a Cargo build-script directive.
    ///
    /// The guard runs in [`DirectiveSafe::new`] and a value cannot be built any
    /// other way, so a value that exists carries its own proof and no call site
    /// has to remember to check. That is the point: such strings reach `cargo:`
    /// lines from two unrelated sources — a `PKG_CONFIG_PATH` entry and
    /// pkg-config's own subprocess output — and re-checking at each use is a
    /// discipline the compiler cannot enforce.
    ///
    /// What the guard rejects: control characters (Unicode `Cc` — C0, DEL, C1).
    /// Cargo reads a build script's stdout line by line and treats every line
    /// beginning with `cargo:` as a directive, with no escaping mechanism, so an
    /// embedded newline splits one `println!` into two lines and lets the second
    /// be a directive of the value's choosing. The same value also reaches a
    /// `cargo:warning=` rendered in the developer's terminal, where an escape
    /// sequence is CWE-150.
    ///
    /// What it does NOT reject: bidi and format overrides such as U+202E, which
    /// cannot terminate a line and so do not enable injection. This is
    /// deliberately weaker than `rdlp_cli::sanitize`, which strips bidi too —
    /// that function guards remote-sourced, attacker-controlled text, whereas
    /// the values here are set by the operator in their own environment or
    /// reported by their own pkg-config.
    // `Debug`/`PartialEq` are load-bearing for `assert_eq!` in the tests;
    // `Eq` rides along. No ordering is derived: byte order over a path
    // string is not path order, and nothing sorts these (`watched.sort()`
    // sorts `PathBuf`s).
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) struct DirectiveSafe(String);

    impl DirectiveSafe {
        /// `None` when `s` cannot be named in a directive.
        pub(crate) fn new(s: &str) -> Option<Self> {
            is_directive_safe(s).then(|| Self(s.to_string()))
        }

        pub(crate) fn as_str(&self) -> &str {
            &self.0
        }
    }
}

use directive_safe::{DirectiveSafe, is_directive_safe};

/// The pkg-config file whose presence decides whether a custom FFmpeg build is
/// usable at a location.
///
/// Named once for both path-forming sites — the existence probe in `build.rs`
/// and [`watch_path_in`] — so they cannot come to name different files. It is
/// still spelled literally in `build.rs`'s module doc and warning text, where
/// it is prose rather than a path. No test can observe a divergence between
/// the probe and the watch path: `build.rs` is a separate compilation unit the
/// test target never compiles, so `avcodec_pc_filename_is_pinned` pins only the
/// spelling the tests' own literal expectations assume.
const AVCODEC_PC: &str = "libavcodec.pc";

/// The first non-empty, whitespace-trimmed entry of a `PKG_CONFIG_PATH`-style
/// value, whether or not it can be named in a directive.
///
/// That entry is the user's custom-FFmpeg intent; later entries are
/// conventional distro fallbacks (`/usr/lib/pkgconfig` etc.). Every consumer
/// below resolves through here so none can select a different entry.
///
/// `separator` is `:` on Unix and `;` on Windows. Splitting on the wrong
/// separator would corrupt a Windows path like `C:\ffmpeg\lib` (the drive
/// colon would be treated as an entry boundary).
///
/// Entries are decoded with `to_string_lossy`. A non-UTF-8 entry therefore
/// yields a string that matches nothing on disk, with two consequences, both
/// fail-open: the watch path never matches, so the script re-runs every build
/// rather than caching a stale verdict; and the existence probe also fails, so
/// a *healthy* non-UTF-8 prefix produces a false "missing or misconfigured"
/// warning naming a U+FFFD-bearing path (U+FFFD is not `Cc`, so the guard
/// admits it).
fn first_nonempty_entry(value: &OsStr, separator: char) -> Option<String> {
    value
        .to_string_lossy()
        .split(separator)
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

/// The first configured entry, or `None` when it cannot be named in a
/// directive.
///
/// Refusal does NOT fall through to the next entry. Promoting a later one
/// would not merely misreport what was configured — the distro fallback has
/// its own `libavcodec.pc`, so it would produce a *healthy* verdict and
/// silently suppress the warning this script exists to emit.
fn first_configured_entry(value: &OsStr, separator: char) -> Option<DirectiveSafe> {
    first_nonempty_entry(value, separator)
        .as_deref()
        .and_then(DirectiveSafe::new)
}

/// Whether the first configured entry exists but cannot be named in a
/// directive.
///
/// Distinguishes "nothing configured" from "configured unusably" so `build.rs`
/// can say so instead of falling silent — without interpolating the offending
/// value, which is the thing that made it unusable. Asks
/// [`is_directive_safe`] directly rather than building a value to discard.
fn first_entry_refused(value: &OsStr, separator: char) -> bool {
    first_nonempty_entry(value, separator).is_some_and(|s| !is_directive_safe(&s))
}

/// Return the first non-empty `PKG_CONFIG_PATH`/`PKG_CONFIG_LIBDIR` entry that
/// does NOT contain `libavcodec.pc`, signalling a broken custom-FFmpeg intent.
///
/// `None` means one of three things: no usable entry, a first entry refused by
/// [`first_configured_entry`], or a healthy first entry.
///
/// The filesystem check is injected as `has_avcodec` so the branch logic is
/// unit-testable without real `.pc` files; `build.rs` passes the real
/// `|p| p.join(AVCODEC_PC).is_file()`.
fn first_broken_prefix(
    value: &OsStr,
    separator: char,
    has_avcodec: impl Fn(&Path) -> bool,
) -> Option<DirectiveSafe> {
    let first = first_configured_entry(value, separator)?;
    if has_avcodec(Path::new(first.as_str())) {
        None
    } else {
        Some(first)
    }
}

/// The `libavcodec.pc` inside `dir` — the single constructor for every watched
/// path, so every watch names its file the same way.
///
/// Infallible: `dir` already carries the proof that it can be named in a
/// directive, so there is nothing left to check here.
// `PathBuf` is spelled in full: `build.rs` already imports it, and a second
// `use` in this `include!`d file would collide there (E0252).
fn watch_path_in(dir: &DirectiveSafe) -> std::path::PathBuf {
    Path::new(dir.as_str()).join(AVCODEC_PC)
}

/// The `libavcodec.pc` whose existence decides the verdict — the path
/// `build.rs` declares to Cargo so the cached answer is invalidated when the
/// custom prefix is installed, rebuilt, or removed.
///
/// Without that declaration Cargo has no reason to re-run the script when the
/// prefix changes, and replays the previous verdict indefinitely (#655).
///
/// A declared path that does not exist is treated by Cargo as dirty, so the
/// script re-runs every build while the prefix is missing and resumes normal
/// caching once the file appears. The Cargo book documents only the mtime
/// comparison and is silent on that missing-path case; the measured scenario
/// table is in #655.
///
/// This covers the FIRST configured entry only — the one whose breakage this
/// script reports. Where pkg-config actually resolved is a separate question
/// that `build.rs` watches separately, because the two can differ.
fn avcodec_watch_path(value: &OsStr, separator: char) -> Option<std::path::PathBuf> {
    Some(watch_path_in(&first_configured_entry(value, separator)?))
}
