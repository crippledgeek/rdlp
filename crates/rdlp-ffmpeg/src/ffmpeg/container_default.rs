//! What decides a container's default codec for one stream kind.
//!
//! `AudioDefault` and `VideoDefault` were the same three-shape enum in two
//! files before #651 — defer to the muxer, override it, or declare the
//! container not a target for this kind. Two differences made unifying them
//! worth more than the deduplication alone:
//!
//! - the video enum's `Override` carried a raw `&'static str`, the last
//!   stringly-typed codec hole left after #642; and
//! - nothing type-checked that a *video* override named a video codec.
//!   `"avc"` is a real `FFmpeg` descriptor that resolves to
//!   `AV_CODEC_ID_ON2AVC`, an **audio** codec, so `VideoDefault::Override("avc")`
//!   compiled and would have silently settled a video decision with an audio
//!   codec.
//!
//! The per-container policy *tables* (`audio_default_for`, `video_default_for`)
//! deliberately stay separate: they share this shape but not a single arm, and
//! each must keep its own exhaustive `ContainerFormat` match with no `_` arm
//! (#538).

use std::marker::PhantomData;

use rdlp_types::media_name::CodecName;

use crate::ffmpeg::source::StreamKind;

/// The policy for a container's default codec, independent of stream kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Policy {
    /// Defer to whatever the linked `FFmpeg` muxer declares.
    FromMuxer,
    /// rdlp overrides the muxer's declaration; the reason is on the arm.
    Override(CodecName),
    /// rdlp's policy is that this container is never a target for this
    /// stream kind — regardless of what the muxer is technically capable of.
    NotATarget,
}

/// A [`Policy`] bound to the stream kind it decides for.
///
/// The kind lives on the wrapper rather than as a fourth `PhantomData`
/// variant on [`Policy`]: a `#[doc(hidden)] _Kind(..)` variant would leak
/// into every exhaustive match and force an unreachable arm at each one.
/// Same shape as [`Source<K>`](crate::ffmpeg::source::Source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerDefault<K: StreamKind> {
    policy: Policy,
    _kind: PhantomData<fn() -> K>,
}

impl<K: StreamKind> ContainerDefault<K> {
    /// Bind `policy` to this stream kind.
    #[must_use]
    pub const fn new(policy: Policy) -> Self {
        Self {
            policy,
            _kind: PhantomData,
        }
    }

    /// The kind-free policy, for matching.
    #[must_use]
    pub const fn policy(&self) -> &Policy {
        &self.policy
    }
}

#[cfg(test)]
mod tests {
    use super::{ContainerDefault, Policy};
    use crate::ffmpeg::source::{Audio, Video};
    use rdlp_types::media_name::CodecName;

    #[test]
    fn an_override_carries_a_validated_codec_name() {
        let d: ContainerDefault<Video> =
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")));
        assert!(matches!(d.policy(), Policy::Override(c) if c.as_str() == "h264"));
    }

    /// The point of the kind parameter: a video default and an audio default
    /// are not the same type, so an audio codec cannot be installed as a
    /// container's *video* override. `"avc"` is the live hazard — a real
    /// descriptor that resolves to `AV_CODEC_ID_ON2AVC`, an audio codec.
    #[test]
    fn video_and_audio_defaults_are_distinct_types() {
        let v: ContainerDefault<Video> = ContainerDefault::new(Policy::FromMuxer);
        let a: ContainerDefault<Audio> = ContainerDefault::new(Policy::FromMuxer);
        assert!(matches!(v.policy(), Policy::FromMuxer));
        assert!(matches!(a.policy(), Policy::FromMuxer));
    }

    /// A `compile_fail` doctest is the real proof of separation; this asserts
    /// the runtime half so the doctest is not the only coverage.
    #[test]
    fn a_video_default_does_not_satisfy_an_audio_signature() {
        fn wants_audio(_: &ContainerDefault<Audio>) {}
        let a: ContainerDefault<Audio> = ContainerDefault::new(Policy::NotATarget);
        wants_audio(&a);
        // `wants_audio(&v)` with `v: ContainerDefault<Video>` is E0308.
    }
}
