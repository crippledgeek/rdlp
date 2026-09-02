//! Branch coverage for the `build.rs` path helpers — the `include!`-shared
//! functions from `build_support/`, compiled here as standalone std-only code
//! (no `.pc` fixture files are created; the filesystem predicate is injected
//! everywhere except one existence precondition).
//!
//! Each BRANCH test would fail against a plausibly-wrong implementation:
//! hardcoding the `:` separator breaks the Windows case; forgetting to skip
//! empty entries breaks the leading-empty case; not trimming breaks the
//! whitespace case; inverting the predicate breaks healthy/broken; gating the
//! watch path on the file already existing breaks the absent-prefix case,
//! which is the whole premise of #655; and interpolating an entry verbatim
//! breaks the control-character case.
//!
//! Two tests are structural rather than branch coverage, and say so in their
//! own docstrings: `watched_and_probed_entries_cannot_drift` is green by
//! construction today and exists as future-drift insurance, and
//! `avcodec_pc_filename_is_pinned` records a const-to-literal coupling it
//! cannot fail independently of.

include!("../build_support/pkgconfig_intent.rs");

use std::ffi::OsString;
use std::path::PathBuf;

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
    // a later distro fallback follows. We must still flag the broken intent.
    let value = OsString::from("/custom/ffmpeg/lib/pkgconfig:/usr/lib/pkgconfig");
    let got = first_broken_prefix(&value, ':', always_missing);
    assert_eq!(
        got.as_ref().map(DirectiveSafe::as_str),
        Some("/custom/ffmpeg/lib/pkgconfig")
    );
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
        first_broken_prefix(&value, ':', always_missing)
            .as_ref()
            .map(DirectiveSafe::as_str),
        Some("/custom/ffmpeg")
    );
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    let value = OsString::from("  /custom/ffmpeg  :  /usr/lib/pkgconfig  ");
    assert_eq!(
        first_broken_prefix(&value, ':', always_missing)
            .as_ref()
            .map(DirectiveSafe::as_str),
        Some("/custom/ffmpeg")
    );
}

#[test]
fn windows_separator_preserves_drive_colon() {
    // With ';' the drive-letter colon in `C:\ffmpeg\lib` must NOT split the
    // entry. A ':'-hardcoded implementation would return "C" here.
    let value = OsString::from(r"C:\ffmpeg\lib\pkgconfig;C:\msys64\usr\lib\pkgconfig");
    assert_eq!(
        first_broken_prefix(&value, ';', always_missing)
            .as_ref()
            .map(DirectiveSafe::as_str),
        Some(r"C:\ffmpeg\lib\pkgconfig")
    );
}

// --- #655: the verdict must be tied to the file it is read from -------------
//
// `build.rs` decided from a filesystem probe but declared no `rerun-if-changed`
// on the probed path, so Cargo replayed a stale verdict indefinitely.

/// Every expectation below spells `libavcodec.pc` literally rather than using
/// `AVCODEC_PC`, so that the assertions test the const instead of restating
/// it. This pins the const to the spelling they assume.
#[test]
fn avcodec_pc_filename_is_pinned() {
    assert_eq!(AVCODEC_PC, "libavcodec.pc");
}

/// The watch path is the `.pc` inside the FIRST configured entry — the entry
/// expressing custom-FFmpeg intent, not a later distro fallback.
#[test]
fn watch_path_names_the_probed_pc_file() {
    let value = OsString::from("/custom/ffmpeg/lib/pkgconfig:/usr/lib/pkgconfig");
    assert_eq!(
        avcodec_watch_path(&value, ':'),
        Some(PathBuf::from("/custom/ffmpeg/lib/pkgconfig/libavcodec.pc"))
    );
}

/// The regression guard for #655. The bug's premise is a prefix whose `.pc` is
/// ABSENT at build time and appears later; if the path were only produced for
/// files that already exist, Cargo would still have no reason to re-run and the
/// stale verdict would survive. The path below cannot exist.
#[test]
fn watch_path_is_returned_even_when_the_pc_file_is_absent() {
    let missing = "/nonexistent-655/definitely/not/here/pkgconfig";
    assert!(
        !Path::new(missing).join("libavcodec.pc").exists(),
        "precondition: the probe path must genuinely not exist"
    );
    assert_eq!(
        avcodec_watch_path(&OsString::from(missing), ':'),
        Some(PathBuf::from(
            "/nonexistent-655/definitely/not/here/pkgconfig/libavcodec.pc"
        ))
    );
}

#[test]
fn watch_path_none_for_empty_value() {
    assert_eq!(avcodec_watch_path(&OsString::from(""), ':'), None);
}

#[test]
fn watch_path_none_when_all_entries_are_empty() {
    assert_eq!(avcodec_watch_path(&OsString::from(":::"), ':'), None);
}

#[test]
fn watch_path_skips_leading_empty_entries() {
    let value = OsString::from("::/custom/ffmpeg:/usr/lib/pkgconfig");
    assert_eq!(
        avcodec_watch_path(&value, ':'),
        Some(PathBuf::from("/custom/ffmpeg/libavcodec.pc"))
    );
}

#[test]
fn watch_path_trims_surrounding_whitespace() {
    let value = OsString::from("  /custom/ffmpeg  :  /usr/lib/pkgconfig  ");
    assert_eq!(
        avcodec_watch_path(&value, ':'),
        Some(PathBuf::from("/custom/ffmpeg/libavcodec.pc"))
    );
}

/// A ':'-hardcoded split would watch `C/libavcodec.pc` here.
#[test]
fn watch_path_uses_the_windows_separator() {
    let value = OsString::from(r"C:\ffmpeg\lib\pkgconfig;C:\msys64\usr\lib\pkgconfig");
    assert_eq!(
        avcodec_watch_path(&value, ';'),
        Some(PathBuf::from(r"C:\ffmpeg\lib\pkgconfig").join("libavcodec.pc"))
    );
}

/// The defect this pins is DRIFT, not a wrong string: if the watched path and
/// the probed path ever select different entries, Cargo would invalidate on a
/// file the verdict never consults.
///
/// Two honest limits. It is green by construction while both functions share
/// `first_configured_entry`, so it can only fail after a future edit re-inlines
/// the selection into one of them — that future insurance is its whole purpose.
/// And agreement is observable only on the broken branch, because that is the
/// only path on which `first_broken_prefix` yields the entry it selected.
#[test]
fn watched_and_probed_entries_cannot_drift() {
    let cases = [
        "/custom/ffmpeg/lib/pkgconfig:/usr/lib/pkgconfig",
        "::/custom/ffmpeg:/usr/lib/pkgconfig",
        "  /spaced/entry  :/usr/lib/pkgconfig",
        "/only/one/entry",
        "/trailing/sep/entry:",
        ":::/late/entry",
    ];
    for case in cases {
        let value = OsString::from(case);
        let probed = first_broken_prefix(&value, ':', always_missing)
            .expect("always_missing reports every first entry as broken");
        let probed = probed.as_str();
        assert_eq!(
            avcodec_watch_path(&value, ':'),
            Some(Path::new(probed).join("libavcodec.pc")),
            "drift for input {case:?}"
        );
    }
}

/// Cargo reads build-script stdout line by line and treats any line starting
/// with `cargo:` as a directive, with no escaping. An entry carrying a newline
/// would split one `println!` into two lines and let the second be a directive
/// of the entry's choosing — here a `rustc-link-lib`. Both consumers must
/// refuse the entry rather than interpolate it.
#[test]
fn entry_carrying_a_newline_is_refused_by_both_consumers() {
    let value = OsString::from("/custom/ffmpeg\ncargo:rustc-link-lib=evil:/usr/lib/pkgconfig");
    assert_eq!(avcodec_watch_path(&value, ':'), None);
    assert_eq!(first_broken_prefix(&value, ':', always_missing), None);
}

/// Carriage return terminates a line for some readers, and ESC would reach a
/// developer's terminal through `cargo:warning=` (CWE-150). Refusal covers the
/// whole control class, not just `\n`.
#[test]
fn other_control_characters_are_refused_too() {
    for bad in ["/a\rb", "/a\u{1b}[31mb", "/a\u{7}b", "/a\tb"] {
        let value = OsString::from(bad);
        assert_eq!(avcodec_watch_path(&value, ':'), None, "for {bad:?}");
        assert_eq!(
            first_broken_prefix(&value, ':', always_missing),
            None,
            "for {bad:?}"
        );
    }
}

/// A control character in the FIRST entry must not silently promote a later
/// entry: the first entry is the configured intent, and answering with a
/// different one would misreport what was set.
#[test]
fn control_char_first_entry_does_not_fall_through_to_the_next() {
    let value = OsString::from("/bad\nentry:/usr/lib/pkgconfig");
    assert_eq!(avcodec_watch_path(&value, ':'), None);
    // The warning consumer is the one that would interpolate the value, so it
    // must refuse too — and must not report the later entry instead. See
    // `first_configured_entry` for why promoting it would be worse than silence.
    assert_eq!(first_broken_prefix(&value, ':', always_missing), None);
}

/// `None` from the two consumers cannot distinguish "nothing configured" from
/// "configured unusably"; `build.rs` needs the difference to warn about the
/// second without echoing it.
#[test]
fn refusal_is_distinguishable_from_absence() {
    assert!(first_entry_refused(&OsString::from("/bad\nentry"), ':'));
    assert!(!first_entry_refused(&OsString::from("/good/entry"), ':'));
    assert!(!first_entry_refused(&OsString::from(""), ':'));
    assert!(!first_entry_refused(&OsString::from(":::"), ':'));
}

/// The guard lives in one constructor, so it is tested there as well as
/// through the two consumers below. What this reaches that they do not is the
/// entry point: `build.rs` calls `DirectiveSafe::new` directly on pkg-config's
/// `pcfiledir`, without passing through `avcodec_watch_path`.
#[test]
fn directive_safe_admits_plain_paths_and_refuses_control_characters() {
    assert!(DirectiveSafe::new("/usr/lib/pkgconfig").is_some());
    assert!(DirectiveSafe::new("/bad\ncargo:rustc-link-lib=evil").is_none());
    assert!(DirectiveSafe::new("/esc\u{1b}[31m").is_none());
    // Bidi is deliberately admitted — it cannot terminate a directive line.
    assert!(DirectiveSafe::new("/rtl\u{202e}dir").is_some());
}

/// `watch_path_in` is the single constructor for every watched path. It is
/// infallible because its argument already carries the proof.
#[test]
fn watch_path_in_joins_the_pc_filename() {
    let dir = DirectiveSafe::new("/usr/lib/pkgconfig").expect("a plain path is directive-safe");
    assert_eq!(
        watch_path_in(&dir),
        PathBuf::from("/usr/lib/pkgconfig/libavcodec.pc")
    );
}
