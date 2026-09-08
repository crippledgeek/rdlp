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
//! the major baked into the generated bindings against the major the loaded
//! library reports is the one comparison that sees both sides.

use std::fmt;

use thiserror::Error;

/// Rendered in place of the build-time prefix when there is none.
///
/// `build.rs` emits an empty `RDLP_FFMPEG_PREFIX` when pkg-config yields
/// nothing usable, which is "we could not tell", not "the two disagree".
const UNKNOWN_PREFIX: &str = "<unknown — pkg-config resolved no prefix at build time>";

/// A `libavcodec` major version: the ABI generation a set of struct layouts
/// belongs to.
///
/// Held as `i64` so both C-side representations widen into it infallibly —
/// `LIBAVCODEC_VERSION_MAJOR` is a `c_int` and `avcodec_version()` returns a
/// `c_uint`, and a fallible cast here would have to invent a fallback major
/// that could itself fake a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvcodecMajor(i64);

impl AvcodecMajor {
    /// `FFmpeg` packs versions as `major << 16 | minor << 8 | micro`
    /// (`AV_VERSION_INT`, `libavutil/version.h`), so the major of what
    /// `avcodec_version()` returns lives in the high bits.
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

impl fmt::Display for AvcodecMajor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The `FFmpeg` install prefix this crate's `build.rs` resolved, baked in via
/// `cargo:rustc-env=RDLP_FFMPEG_PREFIX`.
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

/// The generated bindings and the loaded shared library belong to different
/// `FFmpeg` ABI generations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "FFmpeg ABI mismatch: these bindings were generated against libavcodec major {compiled} \
     (build-time prefix {prefix}), but the libavcodec loaded at run time reports major {linked}. \
     Struct layouts change across a major bump while symbol names survive it, so the link \
     succeeds and every FFmpeg field access reads the wrong offset. Rebuild the bindings \
     against the FFmpeg you intend to link: `cargo clean -p ffmpeg-sys-the-third` with \
     PKG_CONFIG_PATH pointing at it."
)]
pub struct AbiMismatch {
    /// Major the bindings' struct layouts assume.
    pub compiled: AvcodecMajor,
    /// Major the loaded `libavcodec` reports.
    pub linked: AvcodecMajor,
    /// Build-time prefix, or [`UNKNOWN_PREFIX`] when none was resolved.
    pub prefix: String,
}

/// Compare the compile-time and run-time `libavcodec` majors.
///
/// Pure over its three inputs so the mismatch path is testable with a
/// fabricated pair; `prefix` contributes to the message only.
///
/// # Errors
///
/// Returns [`AbiMismatch`] when the two majors differ.
pub fn check_avcodec_abi(
    compiled: AvcodecMajor,
    linked: AvcodecMajor,
    prefix: BuildPrefix<'_>,
) -> Result<(), AbiMismatch> {
    if compiled == linked {
        return Ok(());
    }
    Err(AbiMismatch {
        compiled,
        linked,
        prefix: prefix.to_string(),
    })
}

/// Compare the bindings this crate was compiled against with the `libavcodec`
/// loaded into this process.
///
/// # Errors
///
/// Returns [`AbiMismatch`] when the two `libavcodec` majors differ.
pub fn check_linked_avcodec_abi() -> Result<(), AbiMismatch> {
    // SAFETY: `avcodec_version()` takes no arguments and returns a scalar, so
    // it dereferences no caller-supplied pointer and reads no struct through a
    // possibly-wrong layout. That is what makes it callable even when the ABI
    // it is being used to test turns out to disagree.
    let linked = unsafe { ffmpeg_the_third::ffi::avcodec_version() };

    check_avcodec_abi(
        AvcodecMajor::new(i64::from(ffmpeg_the_third::ffi::LIBAVCODEC_VERSION_MAJOR)),
        AvcodecMajor::from_packed(linked),
        BuildPrefix::from_env_value(env!("RDLP_FFMPEG_PREFIX")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The major `ffmpeg-the-third 4.1` targets (`FFmpeg` 8.0).
    const FFMPEG_8_AVCODEC_MAJOR: i64 = 62;
    /// The next major up (`FFmpeg` 9.0) — the drift rdlp#656 observed.
    const FFMPEG_9_AVCODEC_MAJOR: i64 = 63;

    const A_PREFIX: &str = "/home/user/.local/mediaforge";

    #[test]
    fn agreeing_majors_are_accepted() {
        let result = check_avcodec_abi(
            AvcodecMajor::new(FFMPEG_8_AVCODEC_MAJOR),
            AvcodecMajor::new(FFMPEG_8_AVCODEC_MAJOR),
            BuildPrefix::from_env_value(A_PREFIX),
        );
        assert!(result.is_ok(), "agreeing majors must not be a mismatch");
    }

    #[test]
    fn differing_majors_are_reported_with_both_values() {
        let err = check_avcodec_abi(
            AvcodecMajor::new(FFMPEG_8_AVCODEC_MAJOR),
            AvcodecMajor::new(FFMPEG_9_AVCODEC_MAJOR),
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
    fn absent_prefix_is_not_a_mismatch() {
        // build.rs bakes an empty RDLP_FFMPEG_PREFIX when pkg-config yields
        // nothing usable. That is "unknown", and must not be read as drift.
        let result = check_avcodec_abi(
            AvcodecMajor::new(FFMPEG_8_AVCODEC_MAJOR),
            AvcodecMajor::new(FFMPEG_8_AVCODEC_MAJOR),
            BuildPrefix::from_env_value(""),
        );
        assert!(
            result.is_ok(),
            "an unknown prefix alongside agreeing majors is not a mismatch"
        );
    }

    #[test]
    fn absent_prefix_still_reports_a_real_mismatch_as_unknown() {
        let err = check_avcodec_abi(
            AvcodecMajor::new(FFMPEG_8_AVCODEC_MAJOR),
            AvcodecMajor::new(FFMPEG_9_AVCODEC_MAJOR),
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
    fn major_is_the_high_half_of_a_packed_version() {
        // AV_VERSION_INT(63, 21, 100) — libavcodec 63.21.100. Hex because the
        // components are bit-packed fields, not arithmetic values.
        let packed = (0x3f_u32 << 16) | (0x15_u32 << 8) | 0x64_u32;
        assert_eq!(
            AvcodecMajor::from_packed(packed),
            AvcodecMajor::new(FFMPEG_9_AVCODEC_MAJOR)
        );
    }
}
