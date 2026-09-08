//! The value objects the ABI check compares: which library, what version, and
//! the build-time prefix that says which side drifted.

use std::fmt;

/// Rendered in place of the build-time prefix when there is none.
///
/// `build.rs` emits an empty `RDLP_FFMPEG_PREFIX` when pkg-config yields
/// nothing usable, which is "we could not tell", not "the two disagree".
pub(super) const UNKNOWN_PREFIX: &str = "<unknown — pkg-config resolved no prefix at build time>";

/// One of the `FFmpeg` shared libraries whose struct layouts this crate reads
/// through the generated bindings.
///
/// Each variant names types this crate actually touches, so the list can be
/// audited against the sources rather than taken on trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfmpegLibrary {
    /// `AVCodec`, `AVCodecContext`, `AVPacket`, `AVCodecDescriptor`.
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

/// An `FFmpeg` library version, to the precision that affects struct layout.
///
/// The major is the layout *generation*: it changes when fields move. The minor
/// matters because within one major `FFmpeg` only ever **appends** fields, so
/// the two sides are compatible in one direction only — bindings generated
/// against 62.11 read a field that a loaded 62.1 never allocated.
///
/// Held as `i64` so both C-side representations widen into it infallibly —
/// `LIBAV*_VERSION_*` are `c_int` and the `*_version()` functions return a
/// `c_uint`, and a fallible cast here would have to invent a fallback version
/// that could itself fake a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiVersion {
    major: i64,
    minor: i64,
}

impl AbiVersion {
    /// `FFmpeg` packs versions as `major << 16 | minor << 8 | micro`
    /// (`AV_VERSION_INT`, `libavutil/version.h`).
    const MAJOR_SHIFT: u32 = 16;
    const MINOR_SHIFT: u32 = 8;
    /// One packed field is 8 bits wide.
    const FIELD_MASK: u32 = 0xff;

    /// Components that are already unpacked, e.g. `LIBAVCODEC_VERSION_MAJOR`
    /// and `LIBAVCODEC_VERSION_MINOR`.
    #[must_use]
    pub const fn new(major: i64, minor: i64) -> Self {
        Self { major, minor }
    }

    /// The version carried by an `AV_VERSION_INT`-packed value such as the
    /// return of `avcodec_version()`.
    #[must_use]
    pub fn from_packed(version: u32) -> Self {
        Self {
            major: i64::from(version >> Self::MAJOR_SHIFT),
            minor: i64::from((version >> Self::MINOR_SHIFT) & Self::FIELD_MASK),
        }
    }

    /// The layout generation. Two versions with different majors describe
    /// incompatible struct layouts.
    #[must_use]
    pub const fn major(self) -> i64 {
        self.major
    }

    /// Whether a library at `self` can stand in for bindings built against
    /// `compiled`: the same major, and not an older minor.
    #[must_use]
    pub const fn can_satisfy(self, compiled: Self) -> bool {
        self.major == compiled.major && self.minor >= compiled.minor
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// The `FFmpeg` install prefix this crate's `build.rs` resolved (from
/// `libavcodec.pc`), baked in via `cargo:rustc-env=RDLP_FFMPEG_PREFIX`.
///
/// It is diagnostic only — it tells the operator *which side* drifted, and is
/// never the thing compared. An absent prefix therefore cannot make a
/// mismatch, only make one harder to explain.
///
/// Printing it is a deliberate decision, not an oversight: it is a build-machine
/// path that can contain a username, and it is surfaced in a runtime error that
/// may reach desktop event payloads. It is kept because "which side drifted" is
/// unanswerable without it, and because the string was already baked into the
/// binary before this check existed — anyone holding the binary already has it.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_unpacked_from_an_av_version_int() {
        // AV_VERSION_INT(63, 21, 100) — libavcodec 63.21.100. Hex because the
        // components are bit-packed fields, not arithmetic values.
        let packed = (0x3f_u32 << 16) | (0x15_u32 << 8) | 0x64_u32;
        assert_eq!(AbiVersion::from_packed(packed), AbiVersion::new(63, 21));
    }

    #[test]
    fn a_minor_of_255_does_not_bleed_into_the_major() {
        // The mask is what keeps these separate; without it the minor would
        // carry the major's bits.
        // Micro is 0 and so is not written out — `| 0` is an identity op.
        let packed = (0x3e_u32 << 16) | (0xff_u32 << 8);
        assert_eq!(AbiVersion::from_packed(packed), AbiVersion::new(62, 255));
    }

    #[test]
    fn the_same_version_satisfies_itself() {
        let v = AbiVersion::new(62, 11);
        assert!(v.can_satisfy(v));
    }

    #[test]
    fn a_newer_minor_satisfies_older_bindings() {
        // Fields are only ever appended within a major, so every field the
        // bindings know about exists in the loaded library.
        assert!(AbiVersion::new(62, 20).can_satisfy(AbiVersion::new(62, 11)));
    }

    #[test]
    fn an_older_minor_does_not_satisfy_newer_bindings() {
        // The direction that a major-only check misses: the bindings know a
        // field the loaded library never allocated.
        assert!(!AbiVersion::new(62, 1).can_satisfy(AbiVersion::new(62, 11)));
    }

    #[test]
    fn a_different_major_never_satisfies_however_high_the_minor() {
        assert!(!AbiVersion::new(63, 99).can_satisfy(AbiVersion::new(62, 11)));
        assert!(!AbiVersion::new(61, 99).can_satisfy(AbiVersion::new(62, 11)));
    }

    #[test]
    fn absent_prefix_renders_as_unknown_not_as_a_gap() {
        assert_eq!(BuildPrefix::from_env_value("").to_string(), UNKNOWN_PREFIX);
        assert_eq!(
            BuildPrefix::from_env_value("/opt/ff").to_string(),
            "/opt/ff"
        );
    }
}
