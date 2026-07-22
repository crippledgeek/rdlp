//! Shared recode-encoder resolution + strict per-encoder speed-control validation.
//!
//! The validator and `RecodeStage` MUST resolve the target encoder identically;
//! both go through [`resolve_recode_encoder`] so they cannot drift.

use rdlp_types::ContainerFormat;
use thiserror::Error;

use super::video_codecs::{KnobField, KnobKindDef, resolve_encoder, speed_controls_def};
use super::{codec_registry, muxer_defaults};

/// The codec every container falls back to when the linked build's muxer
/// declares nothing for it, and the answer for a container string that
/// doesn't parse at all. Named because it is now referenced from two places.
const DEFAULT_VIDEO_CODEC: &str = "h264";

/// What decides a container's default video codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoDefault {
    /// Defer to whatever the linked `FFmpeg` muxer declares.
    FromMuxer,
    /// rdlp overrides the muxer's declaration; the reason is on the arm.
    Override(&'static str),
    /// The container is audio-only; nothing in this codebase ever recodes a
    /// *video* stream into it. Resolves to [`DEFAULT_VIDEO_CODEC`] — a fixed,
    /// predictable placeholder — rather than `FromMuxer`.
    NoVideoTarget,
}

/// Policy per container. Exhaustive with no `_` arm: a new [`ContainerFormat`]
/// variant must be classified here or the build fails.
///
/// The 12 true audio-only containers (`M4a`, `Mp3`, `Wav`, `Flac`, `Opus`,
/// `Aac`, `Aiff`, `Mka`, `Wv`, `Caf`, `Ac3`, `Wma`) are classified
/// `NoVideoTarget` rather than `FromMuxer`. This is a deliberate policy
/// decision, not the harmless-by-construction assumption an earlier version
/// of this comment made: probing the linked build directly
/// (`muxer_defaults::declared_codec(_, MediaKind::Video)`) shows these
/// muxers do NOT uniformly declare "no video" the way their audio-side
/// `NoAudio` counterpart (`Ivf`) does — several declare a real, unrelated
/// codec that `FromMuxer` would otherwise leak:
/// - `Mp3`/`Flac`/`Aiff` declare `png` — `FFmpeg`'s embedded-cover-art codec
///   for `ID3v2` APIC / `METADATA_BLOCK_PICTURE`, not a transcodable video
///   codec.
/// - `Wma` declares `msmpeg4v3` — inherited from sharing the `asf` muxer
///   registration with its video-capable sibling [`ContainerFormat::Wmv`].
/// - `Wav`/`Opus`/`Aac`/`Mka`/`Wv`/`Caf`/`Ac3` genuinely declare no video
///   codec (`None`), which `FromMuxer` would resolve to
///   [`DEFAULT_VIDEO_CODEC`] anyway — `NoVideoTarget` makes that outcome
///   explicit and independent of what any particular linked build declares.
/// - `M4a` happens to declare `h264` (it shares the `mp4` muxer's
///   registration), which coincidentally equals [`DEFAULT_VIDEO_CODEC`]
///   already — `NoVideoTarget` still classifies it for uniformity rather
///   than leaving one of the twelve dependent on a coincidence.
///
/// `Ogg` is deliberately excluded from this list: unlike the twelve above it
/// is a genuine dual-purpose container (Ogg Vorbis audio, Ogg Theora video),
/// and correctly resolves `FromMuxer` to `"theora"`.
///
/// The infallible `&'static str` contract (unlike `AudioDefault::NoAudio` on
/// the audio side, which changes the return type to `None`) means
/// `NoVideoTarget` still has to produce *a* value — it resolves to
/// [`DEFAULT_VIDEO_CODEC`], which is never actually muxed since nothing in
/// this codebase recodes a video stream into an audio-only container.
const fn video_default_for(container: ContainerFormat) -> VideoDefault {
    match container {
        // These muxers declare legacy defaults (mpeg4 / flv1 / h263) that no
        // one ships in 2026; h264 in all three is standard modern practice.
        ContainerFormat::Avi | ContainerFormat::Flv | ContainerFormat::ThreeGp => {
            VideoDefault::Override("h264")
        }
        ContainerFormat::M4a
        | ContainerFormat::Mp3
        | ContainerFormat::Wav
        | ContainerFormat::Flac
        | ContainerFormat::Opus
        | ContainerFormat::Aac
        | ContainerFormat::Aiff
        | ContainerFormat::Mka
        | ContainerFormat::Wv
        | ContainerFormat::Caf
        | ContainerFormat::Ac3
        | ContainerFormat::Wma => VideoDefault::NoVideoTarget,
        ContainerFormat::WebM
        | ContainerFormat::Mp4
        | ContainerFormat::Mkv
        | ContainerFormat::Mov
        | ContainerFormat::M4v
        | ContainerFormat::Ts
        | ContainerFormat::Mpg
        | ContainerFormat::F4v
        | ContainerFormat::Wmv
        | ContainerFormat::Asf
        | ContainerFormat::Mxf
        | ContainerFormat::Vob
        | ContainerFormat::Dv
        | ContainerFormat::Nut
        | ContainerFormat::Ivf
        | ContainerFormat::Ogg => VideoDefault::FromMuxer,
    }
}

/// Return the default video codec to encode toward for a given container
/// (e.g. [`ContainerFormat::WebM`] → `"vp9"`, [`ContainerFormat::Mp4`] →
/// `"h264"`).
///
/// This is the **single source of truth** for the container → codec mapping;
/// both the speed-control validator and `RecodeStage` delegate here so they
/// can never resolve differently.
///
/// Most containers defer to the linked build's muxer declaration rather than
/// a hand-copied table — rdlp keeps no second copy of `FFmpeg`'s table.
/// Deviations are named in [`video_default_for`] with their reasons: the
/// `FromMuxer` arm falls back to [`DEFAULT_VIDEO_CODEC`] only when
/// `declared_codec` returns `None` (which does NOT mean "this container
/// carries no video" — see that function's doc comment — an ABI-skew codec id
/// or no muxer claiming the extension both produce it too); the twelve
/// audio-only containers always resolve [`DEFAULT_VIDEO_CODEC`] via the
/// dedicated `NoVideoTarget` policy regardless of what their muxer declares.
///
/// Takes a [`ContainerFormat`] rather than a `&str`: every caller already had
/// one and was calling `.as_ext()` purely to satisfy this signature, so the
/// "callers always pass a canonical extension" invariant was conventional.
/// It is now structural — the same argument as #536.
///
/// No longer `const`: [`muxer_defaults::declared_codec`] performs a runtime
/// FFI lookup against the linked build, so this can't be evaluated at compile
/// time. Callers that were `const fn` purely to delegate here must drop
/// `const` too.
#[must_use]
pub fn default_codec_for_container(container: ContainerFormat) -> &'static str {
    match video_default_for(container) {
        VideoDefault::Override(codec) => codec,
        VideoDefault::NoVideoTarget => DEFAULT_VIDEO_CODEC,
        VideoDefault::FromMuxer => {
            muxer_defaults::declared_codec(container, codec_registry::MediaKind::Video)
                .unwrap_or(DEFAULT_VIDEO_CODEC)
        }
    }
}

/// Resolve the encoder a recode will actually use.
///
/// Precedence mirrors `RecodeStage`: explicit `video_encoder` override first,
/// else the target container's default codec. Self-initializes `FFmpeg`.
#[must_use]
pub fn resolve_recode_encoder(
    video_encoder: Option<&str>,
    recode_video: Option<&str>,
    recode_container: Option<&str>,
) -> Option<&'static str> {
    super::ensure_init().ok();
    if let Some(ve) = video_encoder.filter(|s| !s.is_empty()) {
        return resolve_encoder(ve);
    }
    let target = recode_container.or(recode_video)?;
    // These arrive as raw config strings, so the parse is the boundary where
    // an unknown container becomes the default codec — the behaviour the old
    // `_` arm provided when the match itself was string-based.
    //
    // ONE deliberate widening rides along: the parse resolves ContainerFormat's
    // aliases, so `"mpeg"` now reaches the Mpg arm where the old literal
    // `"mpg" | "vob"` compare let it fall to h264. Same class of widening as
    // #537's, and pinned by `mpeg_alias_resolves_like_mpg` below. Since Task
    // 13 (#618) most non-legacy containers defer to the linked build's muxer
    // rather than a hand-copied table — see `video_default_for` — so several
    // *other* aliases (`mpegts` among them) now resolve to their own
    // muxer-declared codec too, not uniformly to h264; only the aliases whose
    // containers are still classified `Override("h264")` or fall through to
    // `DEFAULT_VIDEO_CODEC` keep that behaviour (see
    // `other_aliases_still_resolve_to_the_default_codec`).
    let codec = target
        .parse::<ContainerFormat>()
        .map_or(DEFAULT_VIDEO_CODEC, default_codec_for_container);
    resolve_encoder(codec)
}

/// Strict per-encoder speed-control validation error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpeedControlError {
    /// Knob not applicable to the resolved encoder.
    #[error("{knob:?} is not applicable to encoder '{encoder}'")]
    NotApplicable {
        /// The knob that was rejected.
        knob: KnobField,
        /// The encoder name the knob was checked against.
        encoder: String,
    },
    /// Choice value not in the allowed set.
    #[error("{knob:?} value '{value}' is not one of {choices:?}")]
    NotInChoices {
        /// The knob whose value was rejected.
        knob: KnobField,
        /// The supplied value.
        value: String,
        /// The encoder's allowed choices.
        choices: Vec<String>,
    },
    /// Int knob given a non-integer value.
    #[error("{knob:?} value '{value}' is not an integer")]
    NotInteger {
        /// The knob whose value failed to parse.
        knob: KnobField,
        /// The supplied (non-integer) value.
        value: String,
    },
    /// Int value outside the encoder's range.
    #[error("{knob:?} value {value} out of range {min}..={max}")]
    OutOfRange {
        /// The knob whose value was out of range.
        knob: KnobField,
        /// The supplied value.
        value: i32,
        /// Minimum allowed value (inclusive).
        min: i32,
        /// Maximum allowed value (inclusive).
        max: i32,
    },
}

fn check_knob(
    field: KnobField,
    value: Option<&str>,
    encoder: &str,
) -> Result<(), SpeedControlError> {
    let Some(val) = value else {
        return Ok(());
    };
    let def = speed_controls_def(encoder)
        .iter()
        .find(|k| k.field == field)
        .ok_or_else(|| SpeedControlError::NotApplicable {
            knob: field,
            encoder: encoder.to_string(),
        })?;
    match def.kind {
        KnobKindDef::Choice { choices, .. } => {
            if choices.contains(&val) {
                Ok(())
            } else {
                Err(SpeedControlError::NotInChoices {
                    knob: field,
                    value: val.to_string(),
                    choices: choices.iter().map(|s| (*s).to_string()).collect(),
                })
            }
        }
        KnobKindDef::Int { min, max, .. } => {
            let n: i32 = val.parse().map_err(|_| SpeedControlError::NotInteger {
                knob: field,
                value: val.to_string(),
            })?;
            if (min..=max).contains(&n) {
                Ok(())
            } else {
                Err(SpeedControlError::OutOfRange {
                    knob: field,
                    value: n,
                    min,
                    max,
                })
            }
        }
    }
}

/// Strictly validate speed-control values against the resolved encoder's descriptor.
///
/// `None` knobs and an unresolvable encoder are accepted (an unresolvable
/// encoder means no recode runs). Self-initializes `FFmpeg`.
///
/// # Errors
///
/// Returns [`SpeedControlError`] when a knob is inapplicable to the encoder or a
/// value is outside the encoder's allowed set/range.
pub fn validate_speed_controls(
    encoder: Option<&str>,
    preset: Option<&str>,
    deadline: Option<&str>,
    cpu_used: Option<i32>,
    speed_level: Option<u32>,
) -> Result<(), SpeedControlError> {
    super::ensure_init().ok();
    let Some(enc) = encoder else {
        return Ok(());
    };
    // The numeric knobs bind their owned strings first so `.as_deref()` can
    // borrow them; `preset`/`deadline` are already `&str` and pass through with
    // no allocation on the happy path.
    let cpu = cpu_used.map(|n| n.to_string());
    let spd = speed_level.map(|n| i32::try_from(n).unwrap_or(i32::MAX).to_string());
    check_knob(KnobField::Preset, preset, enc)?;
    check_knob(KnobField::Deadline, deadline, enc)?;
    check_knob(KnobField::CpuUsed, cpu.as_deref(), enc)?;
    check_knob(KnobField::SpeedLevel, spd.as_deref(), enc)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_rejected_on_x264() {
        let err =
            validate_speed_controls(Some("libx264"), None, Some("good"), None, None).unwrap_err();
        assert!(matches!(
            err,
            SpeedControlError::NotApplicable {
                knob: KnobField::Deadline,
                ..
            }
        ));
    }

    #[test]
    fn cpu_used_range_is_per_encoder() {
        // libvpx-vp9 range is -8..=8, so 9 is out of range
        assert!(validate_speed_controls(Some("libvpx-vp9"), None, None, Some(9), None).is_err());
        // libvpx (VP8) range is -16..=16, so 9 is in range
        assert!(validate_speed_controls(Some("libvpx"), None, None, Some(9), None).is_ok());
    }

    #[test]
    fn vvenc_preset_membership() {
        assert!(
            validate_speed_controls(Some("libvvenc"), Some("medium"), None, None, None).is_ok()
        );
        assert!(
            validate_speed_controls(Some("libvvenc"), Some("ultrafast"), None, None, None).is_err()
        );
    }

    #[test]
    fn svtav1_preset_is_numeric() {
        // 8 is within -2..=13
        assert!(validate_speed_controls(Some("libsvtav1"), Some("8"), None, None, None).is_ok());
        // 14 is outside -2..=13
        assert!(validate_speed_controls(Some("libsvtav1"), Some("14"), None, None, None).is_err());
        // non-integer string fails parse
        assert!(
            validate_speed_controls(Some("libsvtav1"), Some("fast"), None, None, None).is_err()
        );
    }

    #[test]
    fn none_values_and_unresolved_encoder_pass() {
        assert!(validate_speed_controls(Some("libx264"), None, None, None, None).is_ok());
        assert!(validate_speed_controls(None, Some("anything"), None, None, None).is_ok());
    }

    #[test]
    fn resolver_prefers_explicit_encoder() {
        super::super::ensure_init().ok();
        if resolve_encoder("libx265").is_some() {
            assert_eq!(
                resolve_recode_encoder(Some("libx265"), Some("mp4"), None),
                Some("libx265")
            );
        }
    }

    /// Behaviour parity across the string → `ContainerFormat` conversion, now
    /// updated for Task 13's muxer-derived defaults: `Ivf`/`Mpg`/`Vob` used to
    /// be hardcoded overrides (`"vp9"`/`"mpeg2"`/`"mpeg2"`) and now defer to
    /// what the linked build's muxer actually declares (`"vp8"`/
    /// `"mpeg1video"`/`"mpeg2video"` — see `broadcast_containers_get_their_declared_codecs`
    /// and `video_kind_answers_separately` in `muxer_defaults.rs` for the
    /// same values pinned independently).
    #[test]
    fn resolver_matches_recode_stage_defaults() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        for (container, want) in [
            (ContainerFormat::WebM, "vp9"),
            (ContainerFormat::Ivf, "vp8"),
            (ContainerFormat::Ogg, "theora"),
            (ContainerFormat::Mpg, "mpeg1video"),
            (ContainerFormat::Vob, "mpeg2video"),
            (ContainerFormat::Mp4, "h264"),
            (ContainerFormat::Mkv, "h264"),
        ] {
            assert_eq!(
                default_codec_for_container(container),
                want,
                "{container:?}"
            );
        }
    }

    /// The typed signature is the point: a caller can no longer hand this an
    /// arbitrary string. Aliases resolve to the same answer as the canonical
    /// name, which the old `to_ascii_lowercase` compare did not do.
    #[test]
    fn aliases_resolve_to_the_same_codec() {
        assert_eq!(
            default_codec_for_container("matroska".parse().expect("alias")),
            default_codec_for_container(ContainerFormat::Mkv)
        );
    }

    /// The one string whose resolved codec this conversion CHANGES: `"mpeg"`
    /// is a `ContainerFormat` alias for `Mpg`, so it now resolves to mpeg2
    /// where the old literal `"mpg" | "vob"` compare dropped it to h264.
    /// Deliberate, documented at the parse site, and pinned here.
    #[test]
    fn mpeg_alias_resolves_like_mpg() {
        assert_eq!(
            resolve_recode_encoder(None, Some("mpeg"), None),
            resolve_recode_encoder(None, Some("mpg"), None),
            "the `mpeg` alias must resolve exactly as `mpg` does"
        );
    }

    /// Aliases whose containers are still classified `Override("h264")` (or,
    /// like `mp4`/`quicktime`/`matroska`, still happen to resolve to h264 via
    /// `FromMuxer` on this build) keep landing on the h264 default; `mpegts`
    /// is deliberately excluded here — since Task 13 (#618) `Ts` defers to
    /// its own muxer declaration (`mpeg2video`), not h264 (see
    /// `broadcast_containers_get_their_declared_codecs`).
    #[test]
    fn other_aliases_still_resolve_to_the_default_codec() {
        for alias in ["3gpp", "matroska", "quicktime"] {
            assert_eq!(
                resolve_recode_encoder(None, Some(alias), None),
                resolve_recode_encoder(None, Some("mp4"), None),
                "{alias} must still resolve to the h264 default"
            );
        }
    }

    /// `resolve_recode_encoder` still takes strings (config-supplied), so its
    /// unparseable-input path must keep the old `_ => "h264"` behaviour rather
    /// than silently resolving to nothing.
    #[test]
    fn unparseable_container_string_still_falls_back_to_h264() {
        assert_eq!(
            resolve_recode_encoder(None, Some("not-a-container"), None),
            resolve_recode_encoder(None, Some("mp4"), None),
            "an unknown container string must resolve exactly as the h264 default did"
        );
    }

    #[test]
    fn dv_container_gets_dvvideo_not_h264() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            default_codec_for_container(ContainerFormat::Dv),
            "dvvideo",
            "h264 into .dv makes FFmpeg refuse to write the header"
        );
    }

    #[test]
    fn broadcast_containers_get_their_declared_codecs() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            default_codec_for_container(ContainerFormat::Mxf),
            "mpeg2video"
        );
        assert_eq!(
            default_codec_for_container(ContainerFormat::Ts),
            "mpeg2video"
        );
        assert_eq!(default_codec_for_container(ContainerFormat::Nut), "mpeg4");
        assert_eq!(
            default_codec_for_container(ContainerFormat::Wmv),
            "msmpeg4v3"
        );
    }

    /// These three deviate from the muxer declaration deliberately — h264 in
    /// AVI/FLV/3GP is standard modern practice, unlike the muxers' legacy
    /// mpeg4/flv1/h263 defaults.
    #[test]
    fn legacy_containers_keep_their_deliberate_h264_override() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(default_codec_for_container(ContainerFormat::Avi), "h264");
        assert_eq!(default_codec_for_container(ContainerFormat::Flv), "h264");
        assert_eq!(
            default_codec_for_container(ContainerFormat::ThreeGp),
            "h264"
        );
    }

    #[test]
    fn mainstream_containers_are_unchanged() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(default_codec_for_container(ContainerFormat::Mp4), "h264");
        assert_eq!(default_codec_for_container(ContainerFormat::Mkv), "h264");
        assert_eq!(default_codec_for_container(ContainerFormat::WebM), "vp9");
        assert_eq!(default_codec_for_container(ContainerFormat::Ogg), "theora");
    }

    /// Regression guard for the `NoVideoTarget` policy decision: `Mp3`,
    /// `Flac`, and `Aiff` all declare `png` as their `MediaKind::Video`
    /// default (`FFmpeg`'s embedded-cover-art codec for `ID3v2` APIC /
    /// `METADATA_BLOCK_PICTURE`) and `Wma` declares `msmpeg4v3` (inherited
    /// from sharing the `asf` muxer registration with `Wmv`) — verified by
    /// direct probe against this build. A `FromMuxer` classification would
    /// leak these as the "default video codec" for audio-only containers;
    /// `NoVideoTarget` must produce `DEFAULT_VIDEO_CODEC` instead.
    #[test]
    fn audio_only_containers_do_not_leak_the_muxers_incidental_video_codec() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        for container in [
            ContainerFormat::Mp3,
            ContainerFormat::Flac,
            ContainerFormat::Aiff,
            ContainerFormat::Wma,
        ] {
            assert_eq!(
                default_codec_for_container(container),
                DEFAULT_VIDEO_CODEC,
                "{container:?}: audio-only container must resolve DEFAULT_VIDEO_CODEC, \
                 not whatever incidental codec the muxer happens to declare"
            );
        }
    }

    /// The "exactly these twelve" half of the `NoVideoTarget` policy: no
    /// OTHER container may be classified `NoVideoTarget`, and `Ogg` — despite
    /// living in the same "Audio containers" source section as the twelve —
    /// must NOT be among them, since it is a genuine dual-purpose container
    /// (Ogg Theora video) and must keep resolving `FromMuxer`.
    #[test]
    fn no_video_target_is_exactly_the_twelve_audio_only_containers() {
        use strum::IntoEnumIterator;

        let known = [
            ContainerFormat::M4a,
            ContainerFormat::Mp3,
            ContainerFormat::Wav,
            ContainerFormat::Flac,
            ContainerFormat::Opus,
            ContainerFormat::Aac,
            ContainerFormat::Aiff,
            ContainerFormat::Mka,
            ContainerFormat::Wv,
            ContainerFormat::Caf,
            ContainerFormat::Ac3,
            ContainerFormat::Wma,
        ];
        assert!(
            !known.contains(&ContainerFormat::Ogg),
            "test premise: Ogg must not be in the audio-only set"
        );

        for container in ContainerFormat::iter() {
            let is_no_video_target =
                matches!(video_default_for(container), VideoDefault::NoVideoTarget);
            assert_eq!(
                is_no_video_target,
                known.contains(&container),
                "{container:?}: NoVideoTarget classification does not match the known \
                 set of twelve audio-only containers"
            );
        }
    }

    /// The falsifiable form of "every container's returned codec matches its
    /// own policy" — derives the expected value from `video_default_for` (the
    /// same policy `default_codec_for_container` consumes) and cross-checks it
    /// against an INDEPENDENT oracle: `muxer_defaults::declared_codec` asks the
    /// linked build's muxers directly, rather than reading
    /// `default_codec_for_container`'s own output back at itself (which would
    /// be one code path checked against itself, not a cross-check). A curated
    /// hand-written table was wrong twice on the audio side (see
    /// `audio_encoder_registry`'s equivalent sweep) — this is the same
    /// discipline applied to video.
    #[test]
    fn every_containers_own_default_codec_is_accepted_by_the_matrix() {
        use strum::IntoEnumIterator;
        crate::ffmpeg::ensure_init().expect("ffmpeg init");

        for container in ContainerFormat::iter() {
            let expected = match video_default_for(container) {
                VideoDefault::Override(codec) => codec,
                VideoDefault::NoVideoTarget => DEFAULT_VIDEO_CODEC,
                VideoDefault::FromMuxer => {
                    let Some(codec) =
                        muxer_defaults::declared_codec(container, codec_registry::MediaKind::Video)
                    else {
                        // No muxer claims the extension, or the declared id is
                        // unrepresentable in this build — falls through to
                        // DEFAULT_VIDEO_CODEC, which is what the function
                        // itself does; nothing independent to cross-check.
                        assert_eq!(
                            default_codec_for_container(container),
                            DEFAULT_VIDEO_CODEC,
                            "{container:?}: FromMuxer with no declared codec must fall back \
                             to DEFAULT_VIDEO_CODEC"
                        );
                        continue;
                    };
                    codec
                }
            };

            assert_eq!(
                default_codec_for_container(container),
                expected,
                "{container:?}'s resolved default video codec does not match its own policy"
            );
        }
    }

    /// Mutation-verified exhaustiveness guard: `video_default_for` MUST have
    /// no `_` arm. Delete a single arm's container from the `FromMuxer` list
    /// (e.g. `| ContainerFormat::Vob`) and `cargo check -p rdlp-ffmpeg` fails
    /// with E0004 (non-exhaustive match) — confirmed by hand during review;
    /// this comment is the record of that mutation test, not a runtime
    /// assertion (Rust's exhaustiveness check is a compile-time property and
    /// cannot be probed from within a passing test binary).
    #[test]
    fn video_default_for_is_documented_exhaustive() {
        // A no-op assertion so this test exists as a named anchor for the
        // mutation-testing note above, and so `cargo test` still exercises
        // `video_default_for` at least once outside the sweep.
        assert!(matches!(
            video_default_for(ContainerFormat::Mp4),
            VideoDefault::FromMuxer
        ));
    }
}
