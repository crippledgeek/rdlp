//! Branch coverage for the `build.rs` path-intent helper. The helper is the
//! `include!`-shared `first_broken_prefix` from `build_support/`, compiled
//! here as a standalone std-only function (no real `.pc` files touched — the
//! filesystem predicate is injected).
//!
//! Each test would fail against a plausibly-wrong implementation: hardcoding
//! the `:` separator breaks the Windows case; forgetting to skip empty
//! entries breaks the leading-empty case; not trimming breaks the whitespace
//! case; inverting the predicate breaks healthy/broken.

include!("../build_support/pkgconfig_intent.rs");

use std::ffi::OsString;

/// Predicate stand-ins for the injected filesystem check.
fn always_present(_: &Path) -> bool {
    true
}
fn always_missing(_: &Path) -> bool {
    false
}

#[test]
fn broken_first_entry_is_reported() {
    // Core regression scenario: the user's custom prefix is first and broken,
    // a healthy distro fallback follows. We must still flag the broken intent.
    let value = OsString::from("/custom/ffmpeg/lib/pkgconfig:/usr/lib/pkgconfig");
    let got = first_broken_prefix(&value, ':', always_missing);
    assert_eq!(got.as_deref(), Some("/custom/ffmpeg/lib/pkgconfig"));
}

#[test]
fn healthy_first_entry_is_silent() {
    let value = OsString::from("/custom/ffmpeg/lib/pkgconfig:/usr/lib/pkgconfig");
    assert_eq!(first_broken_prefix(&value, ':', always_present), None);
}

#[test]
fn empty_value_is_none() {
    let value = OsString::from("");
    assert_eq!(first_broken_prefix(&value, ':', always_missing), None);
}

#[test]
fn all_empty_entries_is_none() {
    // No usable path → nothing to warn about, even with a "missing" predicate.
    let value = OsString::from(":::");
    assert_eq!(first_broken_prefix(&value, ':', always_missing), None);
}

#[test]
fn leading_empty_entries_are_skipped() {
    let value = OsString::from("::/custom/ffmpeg:/usr/lib/pkgconfig");
    assert_eq!(
        first_broken_prefix(&value, ':', always_missing).as_deref(),
        Some("/custom/ffmpeg")
    );
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    let value = OsString::from("  /custom/ffmpeg  :  /usr/lib/pkgconfig  ");
    assert_eq!(
        first_broken_prefix(&value, ':', always_missing).as_deref(),
        Some("/custom/ffmpeg")
    );
}

#[test]
fn windows_separator_preserves_drive_colon() {
    // With ';' the drive-letter colon in `C:\ffmpeg\lib` must NOT split the
    // entry. A ':'-hardcoded implementation would return "C" here.
    let value = OsString::from(r"C:\ffmpeg\lib\pkgconfig;C:\msys64\usr\lib\pkgconfig");
    assert_eq!(
        first_broken_prefix(&value, ';', always_missing).as_deref(),
        Some(r"C:\ffmpeg\lib\pkgconfig")
    );
}
