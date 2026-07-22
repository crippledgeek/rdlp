//! Validated newtypes for `FFmpeg`'s codec / encoder name vocabularies (#642).
//!
//! # Why these are newtypes and not enums
//!
//! `FFmpeg` codec and encoder names are an **open runtime vocabulary** — which
//! names exist depends on how the linked build was configured, which is exactly
//! why `is_audio_encoder_available` / `available_encoders` are runtime lookups.
//! A closed enum would be wrong here, unlike [`ContainerFormat`](crate::ContainerFormat),
//! whose set rdlp genuinely fixes.
//!
//! Wrapping `ffmpeg_the_third::codec::Id` (a closed 538-variant enum) was
//! considered and rejected for the same reason: `muxer_defaults.rs` documents
//! live ABI skew where the linked libavcodec has no name for a muxer-declared
//! id, and a closed enum makes that state unrepresentable.
//!
//! So: an **open** newtype with **closed validation**.
//!
//! # One generic type, several kinds
//!
//! These are four vocabularies, not four spellings of one:
//!
//! | Alias | Example | Boundary API |
//! |---|---|---|
//! | [`CodecName`] | `h264`, `aac` | `avcodec_descriptor_get_by_name` |
//! | [`AudioEncoderName`] | `libfdk_aac` | `encoder::find_by_name` |
//! | [`VideoEncoderName`] | `libx264` | `encoder::find_by_name` |
//! | [`Rfc6381Codec`] | `avc1.640028` | HLS `CODECS=` / DASH `codecs=` |
//!
//! They share their representation, validation, and wire format entirely, and
//! differ only in *which vocabulary they belong to*. So rather than four
//! separate structs — whether hand-written or macro-expanded, both of which are
//! four copies of one constructor waiting to drift — there is a single generic
//! [`MediaName<K>`] carrying a zero-sized [`NameKind`] marker. One impl block,
//! one constructor, one validation path.
//!
//! `MediaName<Codec>` and `MediaName<VideoEncoder>` are still **distinct
//! types**, so mixing them remains a compile error: the aliases below are type
//! aliases, but the underlying generic instantiations are not interchangeable.
//!
//! # Why the kinds must stay apart
//!
//! Codec and encoder names genuinely **overlap** — `aac` is valid in both sets,
//! while `libfdk_aac` is only an encoder — which is precisely why keeping them
//! apart earns its keep. The `"avc"` collision recorded in `muxer_defaults.rs`
//! is the concrete hazard: `"avc"` is a real descriptor, but it is
//! `AV_CODEC_ID_ON2AVC`, an *audio* codec, so a video routing decision could be
//! settled by an audio codec's representability.
//!
//! [`Rfc6381Codec`] is a different alphabet entirely: `avc1.640028` is not an
//! `FFmpeg` descriptor name, so feeding one to a muxer predicate answers
//! `false` for the wrong reason. Keeping it a separate kind makes that
//! confusion a compile error rather than a silent wrong answer.
//!
//! # Representation
//!
//! [`MediaName`] wraps `Cow<'static, str>`, which lets the static preference
//! tables stay allocation-free via the `const` [`MediaName::from_static`]
//! constructor while runtime-discovered names own their storage. `const`
//! validation means an invalid table entry is a **build error**, not a runtime
//! surprise.
//!
//! This crate is `unsafe_code = "forbid"`, so the unsized-`str`-transmute shape
//! used by validated-string crates such as `strck` is not available here; the
//! `Cow` shape needs no `unsafe`.
//!
//! # Wire format
//!
//! All four serialise as plain strings, so `config.toml` and the desktop IPC
//! contract are unchanged. Deserialisation re-runs the same validation, so
//! serde is not a hole past the constructor.
//!
//! # The separation is compiler-enforced
//!
//! These `compile_fail` doctests are the executable form of this module's whole
//! reason to exist — if any of them ever starts compiling, the kind distinction
//! has been lost and the silent-wrong-answer hazards described above are back.
//! They are doctests rather than a `trybuild` suite so the guarantee costs no
//! extra dependency.
//!
//! A codec name is not an encoder name:
//!
//! ```compile_fail
//! use rdlp_types::media_name::{CodecName, VideoEncoderName};
//! fn wants_encoder(_: VideoEncoderName) {}
//! wants_encoder(CodecName::from_static("h264"));
//! ```
//!
//! An audio encoder is not a video encoder:
//!
//! ```compile_fail
//! use rdlp_types::media_name::{AudioEncoderName, VideoEncoderName};
//! fn wants_video(_: VideoEncoderName) {}
//! wants_video(AudioEncoderName::from_static("libopus"));
//! ```
//!
//! A manifest codec string is not an `FFmpeg` codec name — the hazard that was
//! entirely unguarded when both were `Option<String>`:
//!
//! ```compile_fail
//! use rdlp_types::media_name::{CodecName, Rfc6381Codec};
//! fn wants_ffmpeg_codec(_: CodecName) {}
//! wants_ffmpeg_codec(Rfc6381Codec::from_static("avc1.640028"));
//! ```
//!
//! And a bare string is not any of them:
//!
//! ```compile_fail
//! use rdlp_types::media_name::CodecName;
//! fn wants_codec(_: CodecName) {}
//! wants_codec("h264");
//! ```
//!
//! Nor can a wrong-vocabulary value reach a `&str` parameter **by reference**.
//! `MediaName` does not implement [`Deref`](std::ops::Deref) precisely because
//! Rust's deref coercion would let `&AudioEncoderName` silently satisfy a
//! `&str` parameter — which is how a registry lookup keyed on the wrong
//! vocabulary used to typecheck at all: `speed_controls_def(&some_audio_name)`
//! compiled and quietly returned `&[]`. [`AsRef::as_ref`] and
//! [`Borrow::borrow`](std::borrow::Borrow) stay implemented because both
//! require an explicit call, so they can't be reached by coercion:
//!
//! ```compile_fail
//! use rdlp_types::media_name::AudioEncoderName;
//! fn wants_str(_: &str) {}
//! let name = AudioEncoderName::from_static("libopus");
//! wants_str(&name);
//! ```

use std::borrow::Cow;
use std::ffi::CString;
use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;

mod kind;

pub use kind::{AudioEncoder, Codec, Rfc6381, VideoEncoder};

/// An `FFmpeg` codec descriptor name (`h264`, `pcm_s16le`). See [`Codec`].
pub type CodecName = MediaName<Codec>;

/// An audio encoder to invoke (`libfdk_aac`, `libopus`). See [`AudioEncoder`].
pub type AudioEncoderName = MediaName<AudioEncoder>;

/// A video encoder to invoke (`libx264`, `libsvtav1`). See [`VideoEncoder`].
pub type VideoEncoderName = MediaName<VideoEncoder>;

/// An RFC 6381 manifest codec string (`avc1.640028`). See [`Rfc6381`].
pub type Rfc6381Codec = MediaName<Rfc6381>;

/// Marker trait identifying which media-name vocabulary a [`MediaName`] belongs to.
///
/// Implementors are zero-sized markers; the trait carries no behaviour beyond
/// naming the concept for error and `Debug` output. It is sealed — the set of
/// vocabularies is rdlp's to define, and an external kind would not correspond
/// to any real `FFmpeg` boundary.
pub trait NameKind:
    sealed::Sealed + Copy + Clone + fmt::Debug + PartialEq + Eq + PartialOrd + Ord + Hash
{
    /// Human-readable name of the concept, used in `Debug` and error output.
    const CONCEPT: &'static str;
}

pub(crate) mod sealed {
    /// Prevents external implementations of [`super::NameKind`].
    pub trait Sealed {}
}

/// Why a string was rejected as a media name.
///
/// The variants are distinct because they mean different things about the
/// caller's bug: [`Empty`](Self::Empty) is a missing value that used to be
/// filtered by hand, [`ControlCharacter`](Self::ControlCharacter) is what would
/// have failed `CString::new` at the FFI boundary, and the other two indicate a
/// value that was never a codec name to begin with (usually a mis-split token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InvalidMediaName {
    /// The name was the empty string.
    Empty,
    /// The name contained a non-ASCII character.
    NonAscii,
    /// The name contained whitespace.
    Whitespace,
    /// The name contained an ASCII control character (including an interior NUL,
    /// the one input that makes `CString::new` fail).
    ControlCharacter,
}

impl InvalidMediaName {
    /// The failure reason, without any concept prefix.
    ///
    /// Private: [`Display`](fmt::Display) is the only caller, and the reason
    /// is not meaningful without the concept prefix `Display` adds.
    #[must_use]
    const fn reason(self) -> &'static str {
        match self {
            Self::Empty => "must not be empty",
            Self::NonAscii => "must be ASCII",
            Self::Whitespace => "must not contain whitespace",
            Self::ControlCharacter => "must not contain control characters",
        }
    }
}

impl fmt::Display for InvalidMediaName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "media name {}", self.reason())
    }
}

impl std::error::Error for InvalidMediaName {}

/// Validates a candidate media name, returning the reason it is unacceptable.
///
/// `const` so [`MediaName::from_static`] can run it during const-eval and turn
/// a bad static-table entry into a build error.
///
/// # Why this floor and not a stricter charset
///
/// The rule is: non-empty, ASCII, no whitespace, no control characters. That is
/// the minimum which guarantees the value survives `CString::new` and rejects
/// the empty-string bug, without guessing at which characters a future `FFmpeg`
/// build might use. Being *under*-inclusive is the safe direction for an open
/// vocabulary — the same doctrine the muxer allow-list follows: a rejected
/// valid name is a hard failure, whereas the floor above only rejects values
/// that could never have worked.
///
/// All four kinds share this floor, so it is one free function rather than a
/// per-kind trait method — which also keeps it callable from `const` context,
/// where trait methods are not available on stable.
#[must_use]
const fn validate(bytes: &[u8]) -> Option<InvalidMediaName> {
    if bytes.is_empty() {
        return Some(InvalidMediaName::Empty);
    }

    let mut i = 0;
    while i < bytes.len() {
        // Indexing rather than iterating: slice iterators are not available in
        // const context. The bound is the loop condition.
        #[allow(clippy::indexing_slicing)]
        let c = bytes[i];

        if !c.is_ascii() {
            return Some(InvalidMediaName::NonAscii);
        }
        // Whitespace is checked before the general control-character rule so
        // that `\n` and `\t` report the more specific reason.
        if c.is_ascii_whitespace() {
            return Some(InvalidMediaName::Whitespace);
        }
        if c.is_ascii_control() {
            return Some(InvalidMediaName::ControlCharacter);
        }
        i += 1;
    }

    None
}

/// A validated media name belonging to the vocabulary `K`.
///
/// Prefer the aliases — [`CodecName`], [`AudioEncoderName`],
/// [`VideoEncoderName`], [`Rfc6381Codec`] — over spelling the generic form.
///
/// Construct via [`new`](Self::new) at runtime or
/// [`from_static`](Self::from_static) in a `const` / static table. Both enforce
/// the invariant documented on [`InvalidMediaName`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaName<K: NameKind>(Cow<'static, str>, PhantomData<K>);

impl<K: NameKind> MediaName<K> {
    /// Validates and wraps a runtime-discovered name.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidMediaName`] if the name is empty, non-ASCII, or
    /// contains whitespace or a control character.
    pub fn new(name: impl AsRef<str>) -> Result<Self, InvalidMediaName> {
        let name = name.as_ref();
        if let Some(reason) = validate(name.as_bytes()) {
            return Err(reason);
        }
        Ok(Self(Cow::Owned(name.to_owned()), PhantomData))
    }

    /// Wraps a compile-time-known name without allocating.
    ///
    /// # Panics
    ///
    /// Panics during const-eval if the name is invalid, which surfaces as a
    /// build error rather than a runtime failure. That is the point: a
    /// malformed static-table entry cannot ship.
    #[must_use]
    pub const fn from_static(name: &'static str) -> Self {
        match validate(name.as_bytes()) {
            None => Self(Cow::Borrowed(name), PhantomData),
            // `K::CONCEPT` cannot be interpolated here — const panic messages
            // must be literals — so the reason alone is the message. The build
            // error names the offending expression, which supplies the rest.
            Some(InvalidMediaName::Empty) => panic!("media name must not be empty"),
            Some(InvalidMediaName::NonAscii) => panic!("media name must be ASCII"),
            Some(InvalidMediaName::Whitespace) => panic!("media name must not contain whitespace"),
            Some(InvalidMediaName::ControlCharacter) => {
                panic!("media name must not contain control characters")
            }
        }
    }

    /// Validates a runtime name that is *known to be* `'static` — e.g. a name
    /// returned by an FFI call into a library's own static table (`FFmpeg`'s
    /// codec descriptor table, in this crate's motivating case) — without
    /// paying `from_static`'s "invalid input is a build error" contract.
    ///
    /// Unlike [`from_static`](Self::from_static), which is only sound to call
    /// on a literal known-good at compile time, this is for a `&'static str`
    /// that is only known-good at *runtime* (a library's documentation
    /// promises the value but the type system can't prove it), so a bad value
    /// returns [`InvalidMediaName`] instead of panicking.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidMediaName`] under the same conditions as
    /// [`new`](Self::new).
    // `Option::map_or` is not yet const-stable at this crate's MSRV, so the
    // `match` clippy's `option_if_let_else` would rather see as a
    // `map_or` call stays as a `match` here — the only way to keep this
    // constructor `const fn`.
    #[allow(clippy::option_if_let_else)]
    pub const fn new_static(name: &'static str) -> Result<Self, InvalidMediaName> {
        match validate(name.as_bytes()) {
            None => Ok(Self(Cow::Borrowed(name), PhantomData)),
            Some(reason) => Err(reason),
        }
    }

    /// Borrows the underlying name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `true` when this value borrows a `'static` name rather than owning one —
    /// i.e. it came from [`from_static`](Self::from_static) or
    /// [`new_static`](Self::new_static) and cost no allocation.
    #[must_use]
    pub const fn is_borrowed(&self) -> bool {
        matches!(self.0, Cow::Borrowed(_))
    }

    /// Recovers the `&'static str` when this value was constructed via
    /// [`from_static`](Self::from_static) or [`new_static`](Self::new_static)
    /// — `None` for a [`new`](Self::new)-constructed, owned value.
    ///
    /// The escape hatch that lets a function whose *entire* input table is
    /// `'static` (a codec/encoder preference table, `FFmpeg`'s own static
    /// descriptor table) keep returning `&'static str` through a `MediaName`
    /// round-trip, so converting an existing `&'static str`-returning
    /// signature to validate through `MediaName` needs no further ripple at
    /// its call sites.
    #[must_use]
    pub fn into_static(self) -> Option<&'static str> {
        match self.0 {
            Cow::Borrowed(s) => Some(s),
            Cow::Owned(_) => None,
        }
    }

    /// Converts to a [`CString`] for an FFI call.
    ///
    /// Infallible in practice: [`validate`] already rejects any control
    /// character — including an interior NUL, the one input
    /// [`CString::new`] can fail on — so a `MediaName` can never fail this
    /// conversion. This is the intended replacement for a scattered
    /// `CString::new(name).ok()?` guard at an FFI boundary that already
    /// receives a validated `MediaName`: the fallibility was real only
    /// because the name hadn't been validated yet.
    ///
    /// # Panics
    ///
    /// Panics only if the `validate`/`CString::new` invariant above is ever
    /// violated, which would itself be a bug in this module.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn to_cstring(&self) -> CString {
        CString::new(self.0.as_bytes())
            .expect("MediaName invariant: validate() rejects interior NUL bytes")
    }

    /// Reinterprets this validated name under a different vocabulary `K2`.
    ///
    /// The **only** sanctioned way to cross between vocabularies — greppable
    /// so every crossing can be audited. Infallible by construction: every
    /// kind shares the exact same [`validate`] floor (see the module
    /// documentation), so a byte sequence that is already a valid
    /// `MediaName<K>` is necessarily a valid `MediaName<K2>` too. Preserves
    /// the `Cow` variant untouched — a `from_static`/`new_static`-backed
    /// value stays allocation-free — unlike a `to_string()` + reparse
    /// round-trip.
    ///
    /// Justify each call site with *why* the value genuinely belongs to both
    /// vocabularies (e.g. `aac` names both a codec and an encoder to invoke);
    /// this is a deliberate, audited vocabulary crossing, not a generic
    /// escape hatch around the kind system.
    #[must_use]
    pub fn retag<K2: NameKind>(self) -> MediaName<K2> {
        MediaName(self.0, PhantomData)
    }
}

/// Canonical `FFmpeg` codec-ID names used in more than one place in the
/// workspace. Single-use codec names stay as literals at their definition
/// site — a const for a value used once is a Lazy Element.
impl MediaName<Codec> {
    /// H.264 / AVC.
    pub const H264: Self = Self::from_static("h264");
    /// H.265 / HEVC.
    pub const HEVC: Self = Self::from_static("hevc");
    /// VP9.
    pub const VP9: Self = Self::from_static("vp9");
    /// VP8.
    pub const VP8: Self = Self::from_static("vp8");
    /// AV1.
    pub const AV1: Self = Self::from_static("av1");
    /// AAC.
    pub const AAC: Self = Self::from_static("aac");
    /// AC-3 (Dolby Digital).
    pub const AC3: Self = Self::from_static("ac3");
    /// E-AC-3 (Dolby Digital Plus).
    pub const EAC3: Self = Self::from_static("eac3");
    /// Opus.
    pub const OPUS: Self = Self::from_static("opus");
    /// FLAC.
    pub const FLAC: Self = Self::from_static("flac");
    /// Vorbis.
    pub const VORBIS: Self = Self::from_static("vorbis");
}

/// Borrows the underlying name as a plain `&str`, so a `MediaName` can be
/// used as the needle in a lookup keyed by `&str` (e.g.
/// `HashMap<MediaName<K>, _>::get(some_str)`).
///
/// Consistent with the derived [`Hash`] and [`Ord`] impls, which both
/// delegate to the wrapped `Cow<str>` and therefore agree with `str`'s own —
/// the contract [`std::borrow::Borrow`] requires.
impl<K: NameKind> std::borrow::Borrow<str> for MediaName<K> {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Compares the name against a plain `&str`, so a resolved `MediaName` can
/// be asserted against a literal without spelling out the kind at the call
/// site (`resolved_encoder == "libx264"`).
impl<K: NameKind> PartialEq<&str> for MediaName<K> {
    fn eq(&self, other: &&str) -> bool {
        self.0 == **other
    }
}

impl<K: NameKind> PartialEq<MediaName<K>> for &str {
    fn eq(&self, other: &MediaName<K>) -> bool {
        *self == other.0
    }
}

/// As [`PartialEq<&str>`], but against an unsized `str` — needed for the
/// standard library's blanket `impl<A, B> PartialEq<&B> for &A where A:
/// PartialEq<B>`, which is what lets `&MediaName<K> == &str` compile (e.g.
/// comparing an `Option<&MediaName<K>>::is_some_and` closure's borrowed
/// parameter against a `&str`, rather than an owned `MediaName<K>`).
impl<K: NameKind> PartialEq<str> for MediaName<K> {
    fn eq(&self, other: &str) -> bool {
        self.0 == *other
    }
}

impl<K: NameKind> PartialEq<MediaName<K>> for str {
    fn eq(&self, other: &MediaName<K>) -> bool {
        *self == *other.0
    }
}

/// Renders as `codec name("h264")` rather than exposing the `PhantomData`
/// field, so the kind stays visible in test failures and logs without the noise.
impl<K: NameKind> fmt::Debug for MediaName<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:?})", K::CONCEPT, self.0)
    }
}

impl<K: NameKind> fmt::Display for MediaName<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<K: NameKind> AsRef<str> for MediaName<K> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<K: NameKind> std::str::FromStr for MediaName<K> {
    type Err = InvalidMediaName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl<K: NameKind> serde::Serialize for MediaName<K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, K: NameKind> serde::Deserialize<'de> for MediaName<K> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(|e| serde::de::Error::custom(format!("{}: {}", K::CONCEPT, e)))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_typical_ffmpeg_names() {
        assert!(validate(b"h264").is_none());
        assert!(validate(b"pcm_s16le").is_none());
        assert!(validate(b"libfdk_aac").is_none());
        assert!(validate(b"avc1.640028").is_none());
    }

    #[test]
    fn validate_reports_the_most_specific_reason() {
        assert_eq!(validate(b""), Some(InvalidMediaName::Empty));
        // `\n` is both whitespace and a control character; whitespace wins.
        assert_eq!(validate(b"h264\n"), Some(InvalidMediaName::Whitespace));
        assert_eq!(
            validate(b"h264\x07"),
            Some(InvalidMediaName::ControlCharacter)
        );
        assert_eq!(
            validate(b"h264\x00"),
            Some(InvalidMediaName::ControlCharacter)
        );
        assert_eq!(validate("é".as_bytes()), Some(InvalidMediaName::NonAscii));
    }

    /// The error type is surfaced to operators, so its wording is part of the
    /// contract rather than an implementation detail.
    #[test]
    fn error_display_is_actionable() {
        assert_eq!(
            InvalidMediaName::Empty.to_string(),
            "media name must not be empty"
        );
        assert_eq!(
            InvalidMediaName::ControlCharacter.to_string(),
            "media name must not contain control characters"
        );
    }

    /// `Debug` must name the kind — otherwise a mixed-up value in a test
    /// failure looks identical whichever vocabulary it came from.
    #[test]
    fn debug_names_the_kind() {
        assert_eq!(
            format!("{:?}", CodecName::from_static("h264")),
            r#"codec name("h264")"#
        );
        assert_eq!(
            format!("{:?}", VideoEncoderName::from_static("libx264")),
            r#"video encoder name("libx264")"#
        );
    }

    /// The associated consts must spell exactly the literal they replace —
    /// this is the characterization test locking the refactor in #653: every
    /// call site that used to spell `CodecName::from_static("h264")` etc.
    /// now reads `CodecName::H264`, and this test is what proves the two are
    /// the same value.
    #[test]
    fn codec_name_associated_consts_match_their_literal() {
        assert_eq!(CodecName::H264.as_str(), "h264");
        assert_eq!(CodecName::HEVC.as_str(), "hevc");
        assert_eq!(CodecName::VP9.as_str(), "vp9");
        assert_eq!(CodecName::VP8.as_str(), "vp8");
        assert_eq!(CodecName::AV1.as_str(), "av1");
        assert_eq!(CodecName::AAC.as_str(), "aac");
        assert_eq!(CodecName::AC3.as_str(), "ac3");
        assert_eq!(CodecName::EAC3.as_str(), "eac3");
        assert_eq!(CodecName::OPUS.as_str(), "opus");
        assert_eq!(CodecName::FLAC.as_str(), "flac");
        assert_eq!(CodecName::VORBIS.as_str(), "vorbis");
    }

    /// A deserialisation failure must say which vocabulary rejected the value.
    #[test]
    fn deserialise_error_names_the_kind() {
        let err = serde_json::from_str::<AudioEncoderName>("\"\"").unwrap_err();
        assert!(
            err.to_string().contains("audio encoder name"),
            "expected the kind in: {err}"
        );
    }
}
