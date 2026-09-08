//! ABI agreement between the `FFmpeg` headers the bindings were generated from
//! and the `FFmpeg` shared objects loaded at run time.
//!
//! `ffmpeg-sys-the-third` resolves and links `FFmpeg` in its own build script,
//! and Cargo caches that resolution under rerun triggers this workspace does
//! not control. A months-old binding set can therefore be replayed against an
//! `FFmpeg` that has since had a major bump. Symbol names largely survive such
//! a bump and struct field offsets do not, so the link succeeds and every
//! subsequent field access reads the wrong offset (rdlp#656).
//!
//! No build-time check can see this: `ffmpeg-sys`'s `ffmpeg_X_Y` cfgs apply
//! only to its own compilation, and this crate's `build.rs` can report what
//! pkg-config says *today*, not what `ffmpeg-sys` linked *earlier*. Comparing
//! the major baked into the generated bindings against the major each loaded
//! library reports is the comparison that sees both sides.
//!
//! Every library whose layouts this crate reads is checked — see
//! [`FfmpegLibrary`] for which types each one carries. Their sonames bump
//! independently, so a partially-upgraded system can skew one without the
//! others, and their majors are not even close to each other: at the time of
//! writing 62, 60, 62 and 11. Checking one does not stand in for the rest.
//!
//! `libswscale` and `libswresample` are deliberately absent: this crate calls
//! nothing from either (no `sws_*`/`swr_*` symbol appears in its sources),
//! because rescaling and resampling both go through filters *inside* the
//! filtergraph — that is, through `libavfilter`. `scripts/check-ffmpeg-abi-coverage.sh`
//! fails the build if that stops being true.

use std::fmt;

use thiserror::Error;

/// Rendered in place of the build-time prefix when there is none.
///
/// `build.rs` emits an empty `RDLP_FFMPEG_PREFIX` when pkg-config yields
/// nothing usable, which is "we could not tell", not "the two disagree".
const UNKNOWN_PREFIX: &str = "<unknown — pkg-config resolved no prefix at build time>";

/// One of the `FFmpeg` shared libraries whose struct layouts this crate reads
/// through the generated bindings.
///
/// Each variant names types this crate actually touches, so the list can be
/// audited against the sources rather than taken on trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegLibrary {
    /// `AVCodec`, `AVCodecContext`, `AVPacket`.
    Avcodec,
    /// `AVFrame`, `AVDictionary`.
    Avutil,
    /// `AVFormatContext`, `AVStream`, `AVOutputFormat`.
    Avformat,
    /// `AVFilterInOut`, whose `name`/`filter_ctx`/`pad_idx`/`next` fields are
    /// written directly on the loudnorm and volume graph paths
    /// (`ffi_helpers/filter_graph.rs`).
    Avfilter,
}

impl FfmpegLibrary {
    /// Every variant, in `index_in_all` order.
    pub const ALL: [Self; 4] = [Self::Avcodec, Self::Avutil, Self::Avformat, Self::Avfilter];

    /// Each variant's position in [`ALL`](Self::ALL).
    ///
    /// The `match` is the point: adding a variant without extending `ALL` is a
    /// non-exhaustive-match compile error here, and the const block below
    /// rejects an `ALL` that is merely the wrong length or order. Together they
    /// make "every library is checked" a property the compiler holds, rather
    /// than one a hand-written list in a test claims.
    const fn index_in_all(self) -> usize {
        match self {
            Self::Avcodec => 0,
            Self::Avutil => 1,
            Self::Avformat => 2,
            Self::Avfilter => 3,
        }
    }
}

// Destructuring `ALL` against a fixed-arity pattern is what makes a forgotten
// variant a compile error rather than a silently short list: add one, and
// either `index_in_all`'s match stops being exhaustive or this pattern stops
// matching `ALL`'s new length. The asserts then tie the array's contents and
// order to that match, so the two cannot drift apart.
const _: () = {
    let [avcodec, avutil, avformat, avfilter] = FfmpegLibrary::ALL;
    assert!(avcodec.index_in_all() == 0, "ALL[0] is not Avcodec");
    assert!(avutil.index_in_all() == 1, "ALL[1] is not Avutil");
    assert!(avformat.index_in_all() == 2, "ALL[2] is not Avformat");
    assert!(avfilter.index_in_all() == 3, "ALL[3] is not Avfilter");
};

impl fmt::Display for FfmpegLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Avcodec => "libavcodec",
            Self::Avutil => "libavutil",
            Self::Avformat => "libavformat",
            Self::Avfilter => "libavfilter",
        })
    }
}

/// An `FFmpeg` library major version: the ABI generation a set of struct
/// layouts belongs to.
///
/// Held as `i64` so both C-side representations widen into it infallibly —
/// `LIBAV*_VERSION_MAJOR` is a `c_int` and the `*_version()` functions return
/// a `c_uint`, and a fallible cast here would have to invent a fallback major
/// that could itself fake a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiMajor(i64);

impl AbiMajor {
    /// `FFmpeg` packs versions as `major << 16 | minor << 8 | micro`
    /// (`AV_VERSION_INT`, `libavutil/version.h`), so the major of what
    /// `avcodec_version()` and its siblings return lives in the high bits.
    const MAJOR_SHIFT: u32 = 16;

    /// A major that is already unpacked, e.g. `LIBAVCODEC_VERSION_MAJOR`.
    #[must_use]
    pub const fn new(major: i64) -> Self {
        Self(major)
    }

    /// The major carried by an `AV_VERSION_INT`-packed value such as the
    /// return of `avcodec_version()`.
    #[must_use]
    pub fn from_packed(version: u32) -> Self {
        Self(i64::from(version >> Self::MAJOR_SHIFT))
    }
}

impl fmt::Display for AbiMajor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What one `FFmpeg` library's two sides claim: the major its headers had when
/// the bindings were generated, and the major the loaded object reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryAbi {
    /// Which library these two majors describe.
    pub library: FfmpegLibrary,
    /// Major the bindings' struct layouts assume.
    pub compiled: AbiMajor,
    /// Major the loaded library reports.
    pub linked: AbiMajor,
}

impl LibraryAbi {
    /// Pair a library with the two majors observed for it.
    #[must_use]
    pub const fn new(library: FfmpegLibrary, compiled: AbiMajor, linked: AbiMajor) -> Self {
        Self {
            library,
            compiled,
            linked,
        }
    }

    /// The disagreement this pair represents, if the two majors differ.
    #[must_use]
    pub fn mismatch(self) -> Option<AbiMismatch> {
        (self.compiled != self.linked).then_some(AbiMismatch {
            library: self.library,
            compiled: self.compiled,
            linked: self.linked,
        })
    }
}

/// One library's ABI disagreement. Rendered as a line within [`AbiMismatches`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{library}: bindings assume major {compiled}, loaded library reports major {linked}")]
pub struct AbiMismatch {
    /// Which library disagreed.
    pub library: FfmpegLibrary,
    /// Major the bindings' struct layouts assume.
    pub compiled: AbiMajor,
    /// Major the loaded library reports.
    pub linked: AbiMajor,
}

/// Every `FFmpeg` library whose ABI generation disagrees with the bindings.
///
/// All of them are reported rather than just the first, because *how many*
/// disagree is the signal separating the two situations this error is written
/// for: one skewed library is a partially-upgraded system, while all of them
/// moving together is a wholesale `FFmpeg` bump. Non-empty by construction —
/// [`new`](Self::new) returns `None` when nothing disagreed, so "a mismatch
/// error carrying no mismatches" is unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiMismatches {
    mismatches: Vec<AbiMismatch>,
    prefix: String,
}

impl AbiMismatches {
    /// Gather disagreements, or `None` when every library agreed.
    #[must_use]
    pub fn new(mismatches: Vec<AbiMismatch>, prefix: BuildPrefix<'_>) -> Option<Self> {
        (!mismatches.is_empty()).then(|| Self {
            mismatches,
            prefix: prefix.to_string(),
        })
    }

    /// The disagreeing libraries, in the order they were checked.
    #[must_use]
    pub fn mismatches(&self) -> &[AbiMismatch] {
        &self.mismatches
    }
}

impl fmt::Display for AbiMismatches {
    // Hand-written rather than a `#[error(...)]` template because the body is a
    // variable-length list, and because the two remedies below belong to the
    // set as a whole — repeating them per library would bury them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FFmpeg ABI mismatch: the FFmpeg loaded at run time disagrees with the bindings \
             this build was compiled against, in {} of the {} libraries it reads:",
            self.mismatches().len(),
            FfmpegLibrary::ALL.len()
        )?;
        for mismatch in self.mismatches() {
            write!(f, "\n  - {mismatch}")?;
        }
        write!(
            f,
            "\nBuild-time FFmpeg prefix: {}. Struct layouts change across a major bump while \
             symbol names survive it, so the link succeeds and every field access through those \
             layouts reads the wrong offset. From a checkout: regenerate the bindings against \
             the FFmpeg you intend to link — `cargo clean -p ffmpeg-sys-the-third`, with \
             PKG_CONFIG_PATH pointing at it. Running a prebuilt rdlp: the FFmpeg on this system \
             has moved since this binary was built, so install one providing the majors listed \
             above as \"bindings assume\" and point LD_LIBRARY_PATH at it, or obtain an rdlp \
             built against the majors it reports.",
            self.prefix
        )
    }
}

impl std::error::Error for AbiMismatches {}

/// The `FFmpeg` install prefix this crate's `build.rs` resolved (from
/// `libavcodec.pc`), baked in via `cargo:rustc-env=RDLP_FFMPEG_PREFIX`.
///
/// It is diagnostic only — it tells the operator *which side* drifted, and is
/// never the thing compared. An absent prefix therefore cannot make a
/// mismatch, only make one harder to explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildPrefix<'a>(Option<&'a str>);

impl<'a> BuildPrefix<'a> {
    /// Read the baked env value, treating the empty string as "unknown".
    #[must_use]
    pub fn from_env_value(raw: &'a str) -> Self {
        Self(Some(raw).filter(|prefix| !prefix.is_empty()))
    }
}

impl fmt::Display for BuildPrefix<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.unwrap_or(UNKNOWN_PREFIX))
    }
}

/// Compare every observed library's compile-time and run-time majors.
///
/// Pure over its inputs so the mismatch path is testable with fabricated pairs;
/// `prefix` contributes to the message only.
///
/// # Errors
///
/// Returns [`AbiMismatches`] carrying every library that disagreed.
pub fn check_abis(observed: &[LibraryAbi], prefix: BuildPrefix<'_>) -> Result<(), AbiMismatches> {
    let mismatches = observed.iter().filter_map(|abi| abi.mismatch()).collect();

    AbiMismatches::new(mismatches, prefix).map_or(Ok(()), Err)
}

/// The two majors this process can observe for one library.
fn observe(library: FfmpegLibrary) -> LibraryAbi {
    use ffmpeg_the_third::ffi;

    // SAFETY: each `*_version()` takes no arguments and returns a scalar, so it
    // dereferences no caller-supplied pointer and reads no struct through a
    // possibly-wrong layout. That is what makes them callable even when the ABI
    // they are being used to test turns out to disagree.
    let (compiled, linked) = unsafe {
        match library {
            FfmpegLibrary::Avcodec => (ffi::LIBAVCODEC_VERSION_MAJOR, ffi::avcodec_version()),
            FfmpegLibrary::Avutil => (ffi::LIBAVUTIL_VERSION_MAJOR, ffi::avutil_version()),
            FfmpegLibrary::Avformat => (ffi::LIBAVFORMAT_VERSION_MAJOR, ffi::avformat_version()),
            FfmpegLibrary::Avfilter => (ffi::LIBAVFILTER_VERSION_MAJOR, ffi::avfilter_version()),
        }
    };

    LibraryAbi::new(
        library,
        AbiMajor::new(i64::from(compiled)),
        AbiMajor::from_packed(linked),
    )
}

/// Compare the bindings this crate was compiled against with every `FFmpeg`
/// library loaded into this process.
///
/// # Errors
///
/// Returns [`AbiMismatches`] listing every library that disagreed.
pub fn check_linked_ffmpeg_abi() -> Result<(), AbiMismatches> {
    check_abis(
        &FfmpegLibrary::ALL.map(observe),
        BuildPrefix::from_env_value(env!("RDLP_FFMPEG_PREFIX")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `libavcodec`'s major in `ffmpeg-the-third 4.1`'s bindings (`FFmpeg` 8.0).
    const FFMPEG_8_AVCODEC_MAJOR: i64 = 62;
    /// The next major up (`FFmpeg` 9.0) — the drift rdlp#656 observed.
    const FFMPEG_9_AVCODEC_MAJOR: i64 = 63;
    /// `libavutil`'s major in the same bindings — deliberately not `FFmpeg`'s
    /// own release number, and not equal to `libavcodec`'s.
    const FFMPEG_8_AVUTIL_MAJOR: i64 = 60;

    const A_PREFIX: &str = "/home/user/.local/mediaforge";

    fn observed(library: FfmpegLibrary, compiled: i64, linked: i64) -> LibraryAbi {
        LibraryAbi::new(library, AbiMajor::new(compiled), AbiMajor::new(linked))
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
    fn agreeing_majors_are_accepted() {
        let result = check_abis(&[agreeing_avcodec()], BuildPrefix::from_env_value(A_PREFIX));
        assert!(result.is_ok(), "agreeing majors must not be a mismatch");
    }

    #[test]
    fn differing_majors_are_reported_with_both_values() {
        let err = check_abis(&[skewed_avcodec()], BuildPrefix::from_env_value(A_PREFIX))
            .expect_err("FFmpeg-8 bindings against a FFmpeg-9 library must be reported");

        let message = err.to_string();
        assert!(
            message.contains(&FFMPEG_8_AVCODEC_MAJOR.to_string()),
            "message must name the compile-time major: {message}"
        );
        assert!(
            message.contains(&FFMPEG_9_AVCODEC_MAJOR.to_string()),
            "message must name the run-time major: {message}"
        );
        assert!(
            message.contains(A_PREFIX),
            "message must name the build-time prefix so the operator can see \
             which side drifted: {message}"
        );
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
    fn every_library_this_crate_reads_is_observed() {
        // Derived from ALL rather than a second hand-written list, so this
        // cannot drift from the enum. What it CANNOT see is the crate starting
        // to read a library that has no variant at all — that direction is
        // scripts/check-ffmpeg-abi-coverage.sh's job, which greps the sources
        // for ffi symbols by library prefix.
        for library in FfmpegLibrary::ALL {
            let observed = FfmpegLibrary::ALL.map(observe);
            assert!(
                observed.iter().any(|abi| abi.library == library),
                "{library} has a variant but is never observed; a skew in it \
                 would go unreported"
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
            "an unknown prefix alongside agreeing majors is not a mismatch"
        );
    }

    #[test]
    fn absent_prefix_still_reports_a_real_mismatch_as_unknown() {
        let err = check_abis(&[skewed_avcodec()], BuildPrefix::from_env_value(""))
            .expect_err("a real major mismatch is reported whether or not the prefix is known");

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

    #[test]
    fn major_is_the_high_half_of_a_packed_version() {
        // AV_VERSION_INT(63, 21, 100) — libavcodec 63.21.100. Hex because the
        // components are bit-packed fields, not arithmetic values.
        let packed = (0x3f_u32 << 16) | (0x15_u32 << 8) | 0x64_u32;
        assert_eq!(
            AbiMajor::from_packed(packed),
            AbiMajor::new(FFMPEG_9_AVCODEC_MAJOR)
        );
    }
}
