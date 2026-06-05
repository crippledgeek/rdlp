// Shared, self-contained (std-only) helper `include!`d by BOTH `build.rs`
// (at build time) and `tests/pkgconfig_intent.rs` (at test time). Build
// scripts are a separate compilation unit that `cargo test` never executes,
// so the only way to unit-test build-script logic is to factor the pure part
// into a file like this and `include!` it from a real test target.
//
// Keep this file dependency-free beyond `std` — both includers compile it
// independently.

use std::ffi::OsStr;
use std::path::Path;

/// Return the first non-empty `PKG_CONFIG_PATH`/`PKG_CONFIG_LIBDIR` entry that
/// does NOT contain `libavcodec.pc`, signalling a broken custom-FFmpeg intent.
/// Return `None` when there is no usable entry, or when the first non-empty
/// entry is healthy.
///
/// The filesystem check is injected as `has_avcodec` so the branch logic
/// (entry split, trim, empty-skip, OS-specific `separator`) is unit-testable
/// without real `.pc` files; `build.rs` passes the real
/// `|p| p.join("libavcodec.pc").is_file()`.
///
/// `separator` is `:` on Unix and `;` on Windows. Splitting on the wrong
/// separator would corrupt a Windows path like `C:\ffmpeg\lib` (the drive
/// colon would be treated as an entry boundary), which is exactly what the
/// accompanying test pins down.
fn first_broken_prefix(
    value: &OsStr,
    separator: char,
    has_avcodec: impl Fn(&Path) -> bool,
) -> Option<String> {
    let value_str = value.to_string_lossy();
    let first = value_str
        .split(separator)
        .map(str::trim)
        .find(|s| !s.is_empty())?;
    if has_avcodec(Path::new(first)) {
        None
    } else {
        Some(first.to_string())
    }
}
