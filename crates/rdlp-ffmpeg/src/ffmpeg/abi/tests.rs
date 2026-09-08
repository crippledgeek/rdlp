//! Tests for the ABI check, in their own file so `mod.rs` stays within
//! `CODING_RULES.md`'s 500-line limit.

use super::version::UNKNOWN_PREFIX;
use super::*;

/// `libavcodec`'s major in `ffmpeg-the-third 4.1`'s bindings (`FFmpeg` 8.0).
const FFMPEG_8_AVCODEC_MAJOR: i64 = 62;
/// The next major up (`FFmpeg` 9.0) — the drift rdlp#656 observed.
const FFMPEG_9_AVCODEC_MAJOR: i64 = 63;
/// `libavutil`'s major in the same bindings — deliberately not `FFmpeg`'s
/// own release number, and not equal to `libavcodec`'s.
const FFMPEG_8_AVUTIL_MAJOR: i64 = 60;
/// A minor that is not zero, so a dropped minor is visible.
const A_MINOR: i64 = 11;

const A_PREFIX: &str = "/home/user/.local/mediaforge";

fn observed(library: FfmpegLibrary, compiled: i64, linked: i64) -> LibraryAbi {
    LibraryAbi::new(
        library,
        AbiVersion::new(compiled, A_MINOR),
        AbiVersion::new(linked, A_MINOR),
    )
}

fn agreeing_avcodec() -> LibraryAbi {
    observed(
        FfmpegLibrary::Avcodec,
        FFMPEG_8_AVCODEC_MAJOR,
        FFMPEG_8_AVCODEC_MAJOR,
    )
}

fn skewed_avcodec() -> LibraryAbi {
    observed(
        FfmpegLibrary::Avcodec,
        FFMPEG_8_AVCODEC_MAJOR,
        FFMPEG_9_AVCODEC_MAJOR,
    )
}

fn skewed_avutil() -> LibraryAbi {
    observed(FfmpegLibrary::Avutil, FFMPEG_8_AVUTIL_MAJOR, 61)
}

#[test]
fn agreeing_versions_are_accepted() {
    let result = check_abis(&[agreeing_avcodec()], BuildPrefix::from_env_value(A_PREFIX));
    assert!(result.is_ok(), "agreeing versions must not be a mismatch");
}

#[test]
fn differing_majors_are_reported_with_both_values() {
    let err = check_abis(&[skewed_avcodec()], BuildPrefix::from_env_value(A_PREFIX))
        .expect_err("FFmpeg-8 bindings against a FFmpeg-9 library must be reported");

    let message = err.to_string();
    assert!(
        message.contains(&FFMPEG_8_AVCODEC_MAJOR.to_string()),
        "message must name the compile-time version: {message}"
    );
    assert!(
        message.contains(&FFMPEG_9_AVCODEC_MAJOR.to_string()),
        "message must name the run-time version: {message}"
    );
    assert!(
        message.contains(A_PREFIX),
        "message must name the build-time prefix so the operator can see \
         which side drifted: {message}"
    );
}

#[test]
fn a_backward_minor_skew_is_reported_as_such() {
    // Same major, older loaded minor: a major-only check would pass this,
    // and the bindings would read a field the library never allocated.
    let backward = LibraryAbi::new(
        FfmpegLibrary::Avcodec,
        AbiVersion::new(FFMPEG_8_AVCODEC_MAJOR, 11),
        AbiVersion::new(FFMPEG_8_AVCODEC_MAJOR, 1),
    );

    let mismatch = backward.mismatch().expect("older minor is a mismatch");
    assert_eq!(mismatch.kind, MismatchKind::OlderMinor);
    assert!(
        !mismatch.to_string().contains("different layout generation"),
        "a minor skew must not be described as a major bump: {mismatch}"
    );
}

#[test]
fn a_forward_minor_skew_is_accepted() {
    // Fields are only appended within a major, so a newer loaded library
    // still has every field the bindings know about. Rejecting this would
    // break every routine FFmpeg point release.
    let forward = LibraryAbi::new(
        FfmpegLibrary::Avcodec,
        AbiVersion::new(FFMPEG_8_AVCODEC_MAJOR, 1),
        AbiVersion::new(FFMPEG_8_AVCODEC_MAJOR, 11),
    );
    assert!(forward.mismatch().is_none());
}

#[test]
fn a_mismatch_names_the_library_that_disagreed() {
    let err = check_abis(&[skewed_avutil()], BuildPrefix::from_env_value(A_PREFIX))
        .expect_err("a skewed libavutil must be reported");

    let message = err.to_string();
    assert!(
        message.contains("libavutil"),
        "message must name the library that disagreed, not a fixed one: {message}"
    );
    assert!(
        !message.contains("libavcodec:"),
        "naming libavcodec for a libavutil skew would send the operator to \
         the wrong library: {message}"
    );
}

#[test]
fn one_skewed_library_among_agreeing_ones_still_fails() {
    // The realistic partial-upgrade case: libavcodec agrees, libavutil does
    // not. A check that only looked at libavcodec would pass this.
    let err = check_abis(
        &[agreeing_avcodec(), skewed_avutil()],
        BuildPrefix::from_env_value(A_PREFIX),
    )
    .expect_err("a skew in any one library must fail the set");

    assert_eq!(
        err.mismatches().len(),
        1,
        "only the skewed library belongs in the report: {err}"
    );
    assert_eq!(
        err.mismatches().first().map(|m| m.library),
        Some(FfmpegLibrary::Avutil)
    );
}

#[test]
fn every_disagreeing_library_is_reported_not_just_the_first() {
    // How many disagree is what separates a partial upgrade from a
    // wholesale bump, so stopping at the first would discard the signal.
    let err = check_abis(
        &[skewed_avcodec(), skewed_avutil()],
        BuildPrefix::from_env_value(A_PREFIX),
    )
    .expect_err("two skewed libraries must be reported");

    assert_eq!(
        err.mismatches().len(),
        2,
        "both belong in the report: {err}"
    );
    let message = err.to_string();
    assert!(
        message.contains("libavcodec") && message.contains("libavutil"),
        "the message must list both, not stop at the first: {message}"
    );
}

#[test]
fn an_empty_mismatch_set_cannot_become_an_error() {
    assert!(
        AbiMismatches::new(Vec::new(), BuildPrefix::from_env_value(A_PREFIX)).is_none(),
        "an error carrying no mismatches would claim a failure that did not happen"
    );
}

#[test]
fn each_library_is_observed_through_its_own_version_constants() {
    // Not a tautology over `observe`'s label — that is copied from the
    // argument. This pins the part that can silently be wrong: a swapped
    // arm handing libavutil libavcodec's constants. On an agreeing machine
    // both would still compare Ok, so nothing else would notice.
    use ffmpeg_the_third::ffi;

    for observed in observed_library_abis() {
        let expected = match observed.library {
            FfmpegLibrary::Avcodec => ffi::LIBAVCODEC_VERSION_MAJOR,
            FfmpegLibrary::Avutil => ffi::LIBAVUTIL_VERSION_MAJOR,
            FfmpegLibrary::Avformat => ffi::LIBAVFORMAT_VERSION_MAJOR,
            FfmpegLibrary::Avfilter => ffi::LIBAVFILTER_VERSION_MAJOR,
        };
        assert_eq!(
            observed.compiled.major(),
            i64::from(expected),
            "{} is observed through another library's constants",
            observed.library
        );
    }
}

#[test]
fn the_production_observation_set_covers_every_library() {
    // Binds `observed_library_abis` — the function `check_linked_ffmpeg_abi`
    // actually calls — rather than re-deriving `ALL.map(observe)` here,
    // which would pass however narrow the production path became.
    let observed = observed_library_abis();
    for library in FfmpegLibrary::ALL {
        assert!(
            observed.iter().any(|abi| abi.library == library),
            "{library} is never observed by the production path; a skew in \
             it would go unreported"
        );
    }
}

#[test]
fn every_checked_library_agrees_in_this_process() {
    // The only assertion here about the machine actually running the tests:
    // whatever it linked must agree with what it compiled, or every other
    // FFmpeg test in this crate is reading wrong offsets.
    if let Err(e) = check_linked_ffmpeg_abi() {
        panic!("the linked FFmpeg disagrees with the generated bindings: {e}");
    }
}

#[test]
fn absent_prefix_is_not_a_mismatch() {
    // build.rs bakes an empty RDLP_FFMPEG_PREFIX when pkg-config yields
    // nothing usable. That is "unknown", and must not be read as drift.
    let result = check_abis(&[agreeing_avcodec()], BuildPrefix::from_env_value(""));
    assert!(
        result.is_ok(),
        "an unknown prefix alongside agreeing versions is not a mismatch"
    );
}

#[test]
fn absent_prefix_still_reports_a_real_mismatch_as_unknown() {
    let err = check_abis(&[skewed_avcodec()], BuildPrefix::from_env_value(""))
        .expect_err("a real mismatch is reported whether or not the prefix is known");

    let message = err.to_string();
    assert!(
        message.contains(UNKNOWN_PREFIX),
        "an absent prefix must render as unknown rather than as an empty gap: {message}"
    );
}

#[test]
fn a_mismatch_offers_a_remedy_to_both_audiences() {
    let err = check_abis(&[skewed_avcodec()], BuildPrefix::from_env_value(A_PREFIX))
        .expect_err("mismatch");

    let message = err.to_string();
    assert!(
        message.contains("cargo clean -p ffmpeg-sys-the-third"),
        "a contributor with a checkout needs the rebuild remedy: {message}"
    );
    assert!(
        message.contains("LD_LIBRARY_PATH"),
        "someone running a prebuilt rdlp has no checkout and needs a remedy \
         that does not assume one: {message}"
    );
}
