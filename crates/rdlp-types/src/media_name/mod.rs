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
//! Wrapping [`ffmpeg_the_third::codec::Id`] (a closed 538-variant enum) was
//! considered and rejected for the same reason: `muxer_defaults.rs` documents
//! live ABI skew where the linked libavcodec has no name for a muxer-declared
//! id, and a closed enum makes that state unrepresentable.
//!
//! So: an **open** newtype with **closed validation**.
//!
//! # Why several types and not one
//!
//! These are four vocabularies, not four spellings of one:
//!
//! | Type | Example | Boundary API |
//! |---|---|---|
//! | [`CodecName`] | `h264`, `aac` | `avcodec_descriptor_get_by_name` |
//! | [`AudioEncoderName`] | `libfdk_aac` | `encoder::find_by_name` |
//! | [`VideoEncoderName`] | `libx264` | `encoder::find_by_name` |
//! | [`Rfc6381Codec`] | `avc1.640028` | HLS `CODECS=` / DASH `codecs=` |
//!
//! Codec and encoder names genuinely **overlap** — `aac` is valid in both sets,
//! while `libfdk_aac` is only an encoder — which is precisely why keeping them
//! apart earns its keep. The `"avc"` collision recorded in
//! `muxer_defaults.rs` is the concrete hazard: `"avc"` is a real descriptor,
//! but it is `AV_CODEC_ID_ON2AVC`, an *audio* codec, so a video routing
//! decision could be settled by an audio codec's representability.
//!
//! [`Rfc6381Codec`] is a different alphabet entirely: `avc1.640028` is not an
//! `FFmpeg` descriptor name, so feeding one to a muxer predicate answers
//! `false` for the wrong reason. Keeping it a separate type makes that
//! confusion a compile error rather than a silent wrong answer.
//!
//! # Representation
//!
//! Each wraps `Cow<'static, str>`, which lets the static preference tables stay
//! allocation-free via the `const` [`CodecName::from_static`] constructor while
//! runtime-discovered names own their storage. `const` validation means an
//! invalid table entry is a **build error**, not a runtime surprise.
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
//! reason to exist — if any of them ever starts compiling, the type distinction
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

use std::fmt;

mod codec_name;
mod encoder_name;
mod rfc6381;

pub use codec_name::CodecName;
pub use encoder_name::{AudioEncoderName, VideoEncoderName};
pub use rfc6381::Rfc6381Codec;

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

impl fmt::Display for InvalidMediaName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "media name must not be empty",
            Self::NonAscii => "media name must be ASCII",
            Self::Whitespace => "media name must not contain whitespace",
            Self::ControlCharacter => "media name must not contain control characters",
        })
    }
}

impl std::error::Error for InvalidMediaName {}

/// Validates a candidate media name, returning the reason it is unacceptable.
///
/// `const` so the [`from_static`](CodecName::from_static) constructors can run
/// it during const-eval and turn a bad static-table entry into a build error.
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

/// Generates one validated media-name newtype.
///
/// Every one of these types has the same representation, validation, and wire
/// format; only the doc comment and the concept differ. Generating them keeps
/// the four definitions from drifting apart — a hand-written fourth copy is
/// exactly how one of them would end up with a subtly different constructor.
macro_rules! define_media_name {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        ///
        /// Construct via [`new`](Self::new) at runtime or
        /// [`from_static`](Self::from_static) in a `const` / static table.
        /// Both enforce the invariant documented on
        /// [`InvalidMediaName`](crate::media_name::InvalidMediaName).
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize)]
        #[serde(transparent)]
        pub struct $name(::std::borrow::Cow<'static, str>);

        impl $name {
            /// Validates and wraps a runtime-discovered name.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidMediaName`](crate::media_name::InvalidMediaName)
            /// if the name is empty, non-ASCII, or contains whitespace or a
            /// control character.
            pub fn new(
                name: impl AsRef<str>,
            ) -> ::std::result::Result<Self, crate::media_name::InvalidMediaName> {
                let name = name.as_ref();
                match crate::media_name::validate(name.as_bytes()) {
                    ::std::option::Option::Some(e) => ::std::result::Result::Err(e),
                    ::std::option::Option::None => ::std::result::Result::Ok(Self(
                        ::std::borrow::Cow::Owned(name.to_owned()),
                    )),
                }
            }

            /// Wraps a compile-time-known name without allocating.
            ///
            /// # Panics
            ///
            /// Panics during const-eval if the name is invalid, which surfaces
            /// as a build error rather than a runtime failure. That is the
            /// point: a malformed static-table entry cannot ship.
            #[must_use]
            pub const fn from_static(name: &'static str) -> Self {
                match crate::media_name::validate(name.as_bytes()) {
                    ::std::option::Option::None => {
                        Self(::std::borrow::Cow::Borrowed(name))
                    }
                    ::std::option::Option::Some(
                        crate::media_name::InvalidMediaName::Empty,
                    ) => panic!("media name must not be empty"),
                    ::std::option::Option::Some(
                        crate::media_name::InvalidMediaName::NonAscii,
                    ) => panic!("media name must be ASCII"),
                    ::std::option::Option::Some(
                        crate::media_name::InvalidMediaName::Whitespace,
                    ) => panic!("media name must not contain whitespace"),
                    ::std::option::Option::Some(
                        crate::media_name::InvalidMediaName::ControlCharacter,
                    ) => panic!("media name must not contain control characters"),
                }
            }

            /// Borrows the underlying name.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// `true` when this value borrows a `'static` name rather than
            /// owning one — i.e. it came from [`from_static`](Self::from_static)
            /// and cost no allocation.
            #[must_use]
            pub const fn is_borrowed(&self) -> bool {
                matches!(self.0, ::std::borrow::Cow::Borrowed(_))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl ::std::convert::AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = crate::media_name::InvalidMediaName;

            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let raw = <::std::string::String as ::serde::Deserialize>::deserialize(
                    deserializer,
                )?;
                Self::new(raw).map_err(::serde::de::Error::custom)
            }
        }
    };
}

pub(crate) use define_media_name;

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
}
