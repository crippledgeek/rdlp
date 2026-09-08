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
//! the version baked into the generated bindings against the version each
//! loaded library reports is the comparison that sees both sides.
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
//! filtergraph — that is, through `libavfilter`.
//! `scripts/check-ffmpeg-abi-coverage.sh` fails the build if that stops being
//! true.

mod version;

use std::fmt;

use thiserror::Error;

pub use version::{AbiVersion, BuildPrefix, FfmpegLibrary};

/// What one `FFmpeg` library's two sides claim: the version its headers had
/// when the bindings were generated, and the version the loaded object reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryAbi {
    /// Which library these two versions describe.
    pub library: FfmpegLibrary,
    /// Version the bindings' struct layouts assume.
    pub compiled: AbiVersion,
    /// Version the loaded library reports.
    pub linked: AbiVersion,
}

impl LibraryAbi {
    /// Pair a library with the two versions observed for it.
    #[must_use]
    pub const fn new(library: FfmpegLibrary, compiled: AbiVersion, linked: AbiVersion) -> Self {
        Self {
            library,
            compiled,
            linked,
        }
    }

    /// The disagreement this pair represents, if the loaded library cannot
    /// stand in for what the bindings assume.
    #[must_use]
    pub const fn mismatch(self) -> Option<AbiMismatch> {
        if self.linked.can_satisfy(self.compiled) {
            return None;
        }
        let kind = if self.linked.major() == self.compiled.major() {
            MismatchKind::OlderMinor
        } else {
            MismatchKind::DifferentMajor
        };
        Some(AbiMismatch {
            library: self.library,
            kind,
            compiled: self.compiled,
            linked: self.linked,
        })
    }
}

/// How a library's two sides disagree. The two are separated because they fail
/// for different reasons and a reader needs to know which one they have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchKind {
    /// Different layout generations: fields have moved.
    DifferentMajor,
    /// Same generation, but the loaded library predates the bindings. `FFmpeg`
    /// only appends fields within a major, so the bindings know a field the
    /// loaded library never allocated — reading it runs off the end.
    OlderMinor,
}

impl fmt::Display for MismatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DifferentMajor => "different layout generation",
            Self::OlderMinor => "loaded library predates the bindings",
        })
    }
}

/// One library's ABI disagreement. Rendered as a line within [`AbiMismatches`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{library}: bindings assume {compiled}, loaded library reports {linked} ({kind})")]
pub struct AbiMismatch {
    /// Which library disagreed.
    pub library: FfmpegLibrary,
    /// Why the two are incompatible.
    pub kind: MismatchKind,
    /// Version the bindings' struct layouts assume.
    pub compiled: AbiVersion,
    /// Version the loaded library reports.
    pub linked: AbiVersion,
}

/// Every `FFmpeg` library whose ABI disagrees with the bindings.
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
             has moved since this binary was built, so install one providing the versions listed \
             above as \"bindings assume\" and point LD_LIBRARY_PATH at it, or obtain an rdlp \
             built against the versions it reports.",
            self.prefix
        )
    }
}

impl std::error::Error for AbiMismatches {}

/// Compare every observed library's compile-time and run-time versions.
///
/// Pure over its inputs so the mismatch paths are testable with fabricated
/// pairs; `prefix` contributes to the message only.
///
/// # Errors
///
/// Returns [`AbiMismatches`] carrying every library that disagreed.
pub fn check_abis(observed: &[LibraryAbi], prefix: BuildPrefix<'_>) -> Result<(), AbiMismatches> {
    let mismatches = observed.iter().filter_map(|abi| abi.mismatch()).collect();

    AbiMismatches::new(mismatches, prefix).map_or(Ok(()), Err)
}

/// The two versions this process can observe for one library.
fn observe(library: FfmpegLibrary) -> LibraryAbi {
    use ffmpeg_the_third::ffi;

    // SAFETY: each `*_version()` takes no arguments and returns a scalar, so it
    // dereferences no caller-supplied pointer and reads no struct through a
    // possibly-wrong layout. That is what makes them callable even when the ABI
    // they are being used to test turns out to disagree.
    let (major, minor, linked) = unsafe {
        match library {
            FfmpegLibrary::Avcodec => (
                ffi::LIBAVCODEC_VERSION_MAJOR,
                ffi::LIBAVCODEC_VERSION_MINOR,
                ffi::avcodec_version(),
            ),
            FfmpegLibrary::Avutil => (
                ffi::LIBAVUTIL_VERSION_MAJOR,
                ffi::LIBAVUTIL_VERSION_MINOR,
                ffi::avutil_version(),
            ),
            FfmpegLibrary::Avformat => (
                ffi::LIBAVFORMAT_VERSION_MAJOR,
                ffi::LIBAVFORMAT_VERSION_MINOR,
                ffi::avformat_version(),
            ),
            FfmpegLibrary::Avfilter => (
                ffi::LIBAVFILTER_VERSION_MAJOR,
                ffi::LIBAVFILTER_VERSION_MINOR,
                ffi::avfilter_version(),
            ),
        }
    };

    LibraryAbi::new(
        library,
        AbiVersion::new(i64::from(major), i64::from(minor)),
        AbiVersion::from_packed(linked),
    )
}

/// Every library's two versions, as observed in this process.
///
/// The single production source of the observation set: [`check_linked_ffmpeg_abi`]
/// calls it, and the tests exercise it rather than re-deriving `ALL.map(observe)`
/// themselves — a re-derivation would pass even if the production path observed
/// something narrower.
fn observed_library_abis() -> [LibraryAbi; FfmpegLibrary::ALL.len()] {
    FfmpegLibrary::ALL.map(observe)
}

/// Compare the bindings this crate was compiled against with every `FFmpeg`
/// library loaded into this process.
///
/// # Errors
///
/// Returns [`AbiMismatches`] listing every library that disagreed.
pub fn check_linked_ffmpeg_abi() -> Result<(), AbiMismatches> {
    check_abis(
        &observed_library_abis(),
        BuildPrefix::from_env_value(env!("RDLP_FFMPEG_PREFIX")),
    )
}

#[cfg(test)]
mod tests;
