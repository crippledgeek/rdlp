//! What a probe established about one stream of a source, parameterised by
//! which kind of stream it is.
//!
//! `SourceVideo` and `SourceAudio` were two structurally identical enums in
//! two crates before #651 — same three states, same `from_probe` shape, same
//! rationale in both doc comments — and both degraded an already-validated
//! `CodecName` to `&str`, which is why both re-validated it downstream. One
//! generic carrying a zero-sized kind marker replaces them, exactly as
//! `MediaName<K>` replaced four name newtypes in #642.
//!
//! The kinds must stay apart for the same reason the name kinds do: `"avc"`
//! is a real `FFmpeg` descriptor, but it resolves to `AV_CODEC_ID_ON2AVC`, an
//! *audio* codec. A video routing decision settled by an audio source is a
//! silent wrong answer, and this makes it a compile error.
//!
//! ```compile_fail,E0308
//! use rdlp_ffmpeg::ffmpeg::source::{Source, SourceAudio, SourceVideo};
//! fn wants_video(_: &SourceVideo) {}
//! let audio: SourceAudio = Source::from_probe(true, None);
//! wants_video(&audio);
//! ```

use std::marker::PhantomData;

use rdlp_types::media_name::CodecName;

use crate::ffmpeg::codec_registry::MediaKind;

mod sealed {
    pub trait Sealed {}
}

/// Which kind of stream a [`Source`] describes.
///
/// Sealed: the set is exactly the set of [`MediaKind`] variants rdlp asks
/// representability questions about, and a third one would need a matching
/// policy table rather than just a marker.
pub trait StreamKind: sealed::Sealed {
    /// The runtime kind this marker stands for.
    const MEDIA_KIND: MediaKind;
}

/// Marker for a video stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Video;

/// Marker for an audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Audio;

impl sealed::Sealed for Video {}
impl sealed::Sealed for Audio {}

impl StreamKind for Video {
    const MEDIA_KIND: MediaKind = MediaKind::Video;
}

impl StreamKind for Audio {
    const MEDIA_KIND: MediaKind = MediaKind::Audio;
}

/// The three states a probed stream can be in.
///
/// Public and matched exhaustively by rule factories on purpose: `Absent` and
/// `Unnamed` have opposite correct answers and collapsing them is the
/// #630/#637 regression. An accessor returning `Option<&CodecName>` would
/// merge them, so callers that must tell them apart match this instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    /// No stream of this kind at all.
    Absent,
    /// A stream whose codec this `FFmpeg` build could not name. Proves
    /// nothing about representability, so it must not authorise a copy.
    Unnamed,
    /// The stream's codec, as `FFmpeg`'s own descriptor name.
    Codec(CodecName),
}

/// What the probe established about the `K` stream of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source<K: StreamKind> {
    state: SourceState,
    _kind: PhantomData<fn() -> K>,
}

impl<K: StreamKind> Source<K> {
    /// Classify what `probe` reported. `has_stream` is the discriminator the
    /// codec name alone cannot provide: `MediaInfo` only sets the codec inside
    /// its `has_*` branch, so both "no stream" and "unnameable codec" arrive
    /// as `None`.
    ///
    /// Not a `const fn` — unlike its `&str`-carrying predecessor. The
    /// `(false, _)` arm drops the `Option<CodecName>`, and `CodecName` owns a
    /// `Cow`, so a const version is E0493 ("destructor cannot be evaluated at
    /// compile-time"). No call site is a const context.
    #[must_use]
    pub fn from_probe(has_stream: bool, codec: Option<CodecName>) -> Self {
        let state = match (has_stream, codec) {
            (false, _) => SourceState::Absent,
            (true, Some(codec)) => SourceState::Codec(codec),
            (true, None) => SourceState::Unnamed,
        };
        Self {
            state,
            _kind: PhantomData,
        }
    }

    /// The three-way classification, for callers that must distinguish
    /// `Absent` from `Unnamed`.
    #[must_use]
    pub const fn state(&self) -> &SourceState {
        &self.state
    }

    /// The codec name, when the build could name it. Returns `None` for both
    /// `Absent` and `Unnamed` — use [`state`](Self::state) when that
    /// distinction matters.
    #[must_use]
    pub const fn name(&self) -> Option<&CodecName> {
        match &self.state {
            SourceState::Codec(c) => Some(c),
            SourceState::Absent | SourceState::Unnamed => None,
        }
    }

    /// Whether there is a stream of this kind at all, named or not.
    #[must_use]
    pub const fn is_present(&self) -> bool {
        !matches!(self.state, SourceState::Absent)
    }
}

/// A probed video stream.
pub type SourceVideo = Source<Video>;

/// A probed audio stream.
pub type SourceAudio = Source<Audio>;

#[cfg(test)]
mod tests {
    use super::{Audio, Source, SourceState, SourceVideo, Video};
    use crate::ffmpeg::codec_registry::MediaKind;
    use rdlp_types::media_name::CodecName;
    use rdlp_types::rule::Rule;

    #[test]
    fn from_probe_distinguishes_absent_from_unnamed() {
        let absent: SourceVideo = Source::from_probe(false, None);
        let unnamed: SourceVideo = Source::from_probe(true, None);
        assert!(matches!(absent.state(), SourceState::Absent));
        assert!(matches!(unnamed.state(), SourceState::Unnamed));
    }

    #[test]
    fn a_named_codec_survives_as_a_validated_name() {
        let src: SourceVideo = Source::from_probe(true, Some(CodecName::from_static("h264")));
        assert_eq!(src.name().map(CodecName::as_str), Some("h264"));
        assert!(src.is_present());
    }

    #[test]
    fn markers_carry_their_media_kind() {
        assert_eq!(<Video as super::StreamKind>::MEDIA_KIND, MediaKind::Video);
        assert_eq!(<Audio as super::StreamKind>::MEDIA_KIND, MediaKind::Audio);
    }

    /// The whole reason `Source` is kind-parameterised: a rule asked about
    /// video must not accept an audio source.
    #[test]
    fn video_and_audio_sources_are_distinct_types() {
        let rule = |s: &SourceVideo| s.is_present();
        let video: SourceVideo = Source::from_probe(true, None);
        assert!(rule.eval(&video));
        // `rule.eval(&audio)` where `audio: SourceAudio` is a compile error —
        // proven by the compile_fail doctest on `Source` itself.
    }
}
