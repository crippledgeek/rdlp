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
//! All three libraries whose layouts this crate reads are checked, because
//! their sonames bump independently and a partially-upgraded system can skew
//! one without the others: `libavcodec` (`AVCodec`, `AVCodecContext`),
//! `libavutil` (`AVFrame`, `AVDictionary`, `AVBufferRef`) and `libavformat`
//! (`AVFormatContext`, `AVStream`). They carry genuinely different numbers —
//! at the time of writing 62, 60 and 62 — so checking one does not stand in
//! for the others.

use std::fmt;

use thiserror::Error;

/// Rendered in place of the build-time prefix when there is none.
///
/// `build.rs` emits an empty `RDLP_FFMPEG_PREFIX` when pkg-config yields
/// nothing usable, which is "we could not tell", not "the two disagree".
const UNKNOWN_PREFIX: &str = "<unknown — pkg-config resolved no prefix at build time>";

/// One of the `FFmpeg` shared libraries whose struct layouts this crate reads
/// through the generated bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegLibrary {
    /// `AVCodec`, `AVCodecContext`, `AVPacket`.
    Avcodec,
    /// `AVFrame`, `AVDictionary`, `AVBufferRef`.
    Avutil,
    /// `AVFormatContext`, `AVStream`, `AVOutputFormat`.
    Avformat,
}

impl fmt::Display for FfmpegLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Avcodec => "libavcodec",
            Self::Avutil => "libavutil",
            Self::Avformat => "libavformat",
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
}

/// The generated bindings and a loaded shared library belong to different
/// `FFmpeg` ABI generations.
///
/// The message carries two remedies because it has two audiences: a
/// contributor with a checkout, whose cached binding set needs regenerating,
/// and someone running a prebuilt single-binary rdlp after a distro upgrade
/// moved `FFmpeg` out from under it — the surviving runtime hazard in
/// rdlp#656, and the larger audience of the two.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "FFmpeg ABI mismatch in {library}: these bindings were generated against {library} major \
     {compiled} (build-time FFmpeg prefix {prefix}), but the {library} loaded at run time \
     reports major {linked}. Struct layouts change across a major bump while symbol names \
     survive it, so the link succeeds and every field access through those layouts reads the \
     wrong offset. From a checkout: regenerate the bindings against the FFmpeg you intend to \
     link — `cargo clean -p ffmpeg-sys-the-third`, with PKG_CONFIG_PATH pointing at it. \
     Running a prebuilt rdlp: the FFmpeg on this system has moved to major {linked} since this \
     binary was built, so install a {library} with major {compiled} and point LD_LIBRARY_PATH \
     at it, or obtain an rdlp built against major {linked}."
)]
pub struct AbiMismatch {
    /// Which library disagreed.
    pub library: FfmpegLibrary,
    /// Major the bindings' struct layouts assume.
    pub compiled: AbiMajor,
    /// Major the loaded library reports.
    pub linked: AbiMajor,
    /// Build-time prefix, or [`UNKNOWN_PREFIX`] when none was resolved.
    pub prefix: String,
}

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

/// Compare one library's compile-time and run-time majors.
///
/// Pure over its inputs so the mismatch path is testable with a fabricated
/// pair; `prefix` contributes to the message only.
///
/// # Errors
///
/// Returns [`AbiMismatch`] when the two majors differ.
pub fn check_abi(observed: LibraryAbi, prefix: BuildPrefix<'_>) -> Result<(), AbiMismatch> {
    if observed.compiled == observed.linked {
        return Ok(());
    }
    Err(AbiMismatch {
        library: observed.library,
        compiled: observed.compiled,
        linked: observed.linked,
        prefix: prefix.to_string(),
    })
}

/// What each library's two sides claim in this process.
fn observed_library_abis() -> [LibraryAbi; 3] {
    // SAFETY: each `*_version()` takes no arguments and returns a scalar, so it
    // dereferences no caller-supplied pointer and reads no struct through a
    // possibly-wrong layout. That is what makes them callable even when the ABI
    // they are being used to test turns out to disagree.
    let (avcodec, avutil, avformat) = unsafe {
        (
            ffmpeg_the_third::ffi::avcodec_version(),
            ffmpeg_the_third::ffi::avutil_version(),
            ffmpeg_the_third::ffi::avformat_version(),
        )
    };

    [
        LibraryAbi::new(
            FfmpegLibrary::Avcodec,
            AbiMajor::new(i64::from(ffmpeg_the_third::ffi::LIBAVCODEC_VERSION_MAJOR)),
            AbiMajor::from_packed(avcodec),
        ),
        LibraryAbi::new(
            FfmpegLibrary::Avutil,
            AbiMajor::new(i64::from(ffmpeg_the_third::ffi::LIBAVUTIL_VERSION_MAJOR)),
            AbiMajor::from_packed(avutil),
        ),
        LibraryAbi::new(
            FfmpegLibrary::Avformat,
            AbiMajor::new(i64::from(ffmpeg_the_third::ffi::LIBAVFORMAT_VERSION_MAJOR)),
            AbiMajor::from_packed(avformat),
        ),
    ]
}

/// Compare the bindings this crate was compiled against with every `FFmpeg`
/// library loaded into this process.
///
/// # Errors
///
/// Returns the first [`AbiMismatch`] found. One disagreeing library is already
/// disqualifying, so the remaining checks would add noise, not information.
pub fn check_linked_ffmpeg_abi() -> Result<(), AbiMismatch> {
    let prefix = BuildPrefix::from_env_value(env!("RDLP_FFMPEG_PREFIX"));
    observed_library_abis()
        .into_iter()
        .try_for_each(|observed| check_abi(observed, prefix))
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

    #[test]
    fn agreeing_majors_are_accepted() {
        let result = check_abi(
            observed(
                FfmpegLibrary::Avcodec,
                FFMPEG_8_AVCODEC_MAJOR,
                FFMPEG_8_AVCODEC_MAJOR,
            ),
            BuildPrefix::from_env_value(A_PREFIX),
        );
        assert!(result.is_ok(), "agreeing majors must not be a mismatch");
    }

    #[test]
    fn differing_majors_are_reported_with_both_values() {
        let err = check_abi(
            observed(
                FfmpegLibrary::Avcodec,
                FFMPEG_8_AVCODEC_MAJOR,
                FFMPEG_9_AVCODEC_MAJOR,
            ),
            BuildPrefix::from_env_value(A_PREFIX),
        )
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
        let err = check_abi(
            observed(FfmpegLibrary::Avutil, FFMPEG_8_AVUTIL_MAJOR, 61),
            BuildPrefix::from_env_value(A_PREFIX),
        )
        .expect_err("a skewed libavutil must be reported");

        let message = err.to_string();
        assert!(
            message.contains("libavutil"),
            "message must name the library that disagreed, not a fixed one: {message}"
        );
        assert!(
            !message.contains("libavcodec"),
            "naming libavcodec for a libavutil skew would send the operator to \
             the wrong library: {message}"
        );
    }

    #[test]
    fn each_library_is_checked_independently() {
        // The realistic partial-upgrade case: libavcodec agrees, libavutil does
        // not. A check that only looked at libavcodec would pass this.
        let prefix = BuildPrefix::from_env_value(A_PREFIX);
        let skewed = [
            observed(
                FfmpegLibrary::Avcodec,
                FFMPEG_8_AVCODEC_MAJOR,
                FFMPEG_8_AVCODEC_MAJOR,
            ),
            observed(FfmpegLibrary::Avutil, FFMPEG_8_AVUTIL_MAJOR, 61),
        ];

        let result = skewed
            .into_iter()
            .try_for_each(|observed| check_abi(observed, prefix));

        let err = result.expect_err("a skew in any one library must fail the set");
        assert_eq!(err.library, FfmpegLibrary::Avutil);
    }

    #[test]
    fn every_library_whose_layouts_this_crate_reads_is_observed() {
        // Without this, dropping a library from `observed_library_abis` would
        // pass every other test in this module: the fabricated-pair tests never
        // touch that function, and on an agreeing machine the live check is
        // `Ok` whether it looked at three libraries or one.
        let observed: Vec<FfmpegLibrary> = observed_library_abis()
            .into_iter()
            .map(|abi| abi.library)
            .collect();

        for library in [
            FfmpegLibrary::Avcodec,
            FfmpegLibrary::Avutil,
            FfmpegLibrary::Avformat,
        ] {
            assert!(
                observed.contains(&library),
                "{library} is read through the generated bindings but is not \
                 checked; a skew in it would go unreported. Observed: {observed:?}"
            );
        }
    }

    #[test]
    fn every_checked_library_agrees_in_this_process() {
        // The only assertion here about the machine actually running the tests:
        // whatever it linked must agree with what it compiled, or every other
        // FFmpeg test in this crate is reading wrong offsets.
        assert!(
            check_linked_ffmpeg_abi().is_ok(),
            "the linked FFmpeg disagrees with the generated bindings: {:?}",
            check_linked_ffmpeg_abi()
        );
    }

    #[test]
    fn absent_prefix_is_not_a_mismatch() {
        // build.rs bakes an empty RDLP_FFMPEG_PREFIX when pkg-config yields
        // nothing usable. That is "unknown", and must not be read as drift.
        let result = check_abi(
            observed(
                FfmpegLibrary::Avcodec,
                FFMPEG_8_AVCODEC_MAJOR,
                FFMPEG_8_AVCODEC_MAJOR,
            ),
            BuildPrefix::from_env_value(""),
        );
        assert!(
            result.is_ok(),
            "an unknown prefix alongside agreeing majors is not a mismatch"
        );
    }

    #[test]
    fn absent_prefix_still_reports_a_real_mismatch_as_unknown() {
        let err = check_abi(
            observed(
                FfmpegLibrary::Avcodec,
                FFMPEG_8_AVCODEC_MAJOR,
                FFMPEG_9_AVCODEC_MAJOR,
            ),
            BuildPrefix::from_env_value(""),
        )
        .expect_err("a real major mismatch is reported whether or not the prefix is known");

        let message = err.to_string();
        assert!(
            message.contains(UNKNOWN_PREFIX),
            "an absent prefix must render as unknown rather than as an empty gap: {message}"
        );
    }

    #[test]
    fn a_mismatch_offers_a_remedy_to_both_audiences() {
        let err = check_abi(
            observed(
                FfmpegLibrary::Avcodec,
                FFMPEG_8_AVCODEC_MAJOR,
                FFMPEG_9_AVCODEC_MAJOR,
            ),
            BuildPrefix::from_env_value(A_PREFIX),
        )
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
