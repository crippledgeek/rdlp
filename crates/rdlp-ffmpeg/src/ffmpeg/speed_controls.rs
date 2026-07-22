//! Shared recode-encoder resolution + strict per-encoder speed-control validation.
//!
//! The validator and `RecodeStage` MUST resolve the target encoder identically;
//! both go through [`resolve_recode_encoder`] so they cannot drift.

use rdlp_types::ContainerFormat;
use rdlp_types::media_name::CodecName;
use thiserror::Error;

use super::container_default::{ContainerDefault, Policy};
use super::source::Video;
use super::video_codecs::{
    KnobField, KnobKindDef, SpeedKnobDef, resolve_encoder, speed_controls_def,
};
use super::{codec_registry, muxer_defaults};

/// The codec every container falls back to when the linked build's muxer
/// declares nothing for it, and the answer for a container string that
/// doesn't parse at all. Named because it is now referenced from several
/// places.
///
/// [`Policy::Override`] is public and its `CodecName` may hold a
/// `Cow::Owned` (constructed from a runtime string), so `into_static()` on an
/// arbitrary `Policy::Override(codec)` is not guaranteed to succeed — only
/// the literal built here is. Every non-test call site that needs the
/// fallback as a plain string uses this constant directly instead of
/// round-tripping through a `CodecName` and its fallible `into_static()`.
const DEFAULT_VIDEO_CODEC_STR: &str = "h264";

/// The [`CodecName`] spelling of [`DEFAULT_VIDEO_CODEC_STR`], built from it
/// so the two representations cannot drift apart.
///
/// `#[cfg(test)]`-only: production code never needs the `CodecName` form of
/// the fallback, only the `&'static str` (see `DEFAULT_VIDEO_CODEC_STR`'s doc
/// comment); the tests below use this form to compare against values that
/// already arrive as `CodecName` (e.g. `resolve_encoder`'s return type).
#[cfg(test)]
const DEFAULT_VIDEO_CODEC: CodecName = CodecName::from_static(DEFAULT_VIDEO_CODEC_STR);

/// Policy per container. Exhaustive with no `_` arm: a new [`ContainerFormat`]
/// variant must be classified here or the build fails.
///
/// The 12 containers below are classified `Policy::NotATarget` (for the
/// video stream kind) rather than `FromMuxer`. That classification is
/// deliberately a *policy* statement ("rdlp does not target video at this
/// container"), not a capability claim — probing
/// the linked build directly (`muxer_defaults::declared_codec(_,
/// MediaKind::Video)`) shows the twelve are not one uniform case; they split
/// into three materially different reasons:
///
/// - **No video slot at all** (`Wav`, `Opus`, `Aac`, `Mka`, `Wv`, `Caf`,
///   `Ac3`): the muxer genuinely declares no video codec (`None`), which
///   `FromMuxer` would resolve to [`DEFAULT_VIDEO_CODEC_STR`] anyway —
///   `Policy::NotATarget` makes that outcome explicit and independent of
///   what any particular linked build declares.
/// - **An image slot, not a video slot** (`Mp3`, `Flac`, `Aiff`): these
///   declare `png` — `FFmpeg`'s embedded-cover-art codec for `ID3v2` APIC /
///   `METADATA_BLOCK_PICTURE`, not a transcodable video codec. Live-verified:
///   muxing a real video stream into `.flac` fails with `Video stream #0 is
///   not an attached picture. Ignoring` → `Could not write header`. That
///   image slot is owned by `ThumbnailEmbedStrategy`'s `Id3Apic`/
///   `FlacAttachedPic` arms (`thumbnail/embed_strategy.rs`), not by this
///   enum — `FromMuxer` would otherwise leak `png` as the "default video
///   codec" for these containers.
/// - **A real, genuinely video-capable slot, declined by rdlp policy**
///   (`M4a`, `Wma`): `M4a` shares the `ipod` muxer registration with
///   [`ContainerFormat::M4v`] and declares `h264` — a real video muxer, not a
///   coincidence. `Wma` shares the `asf` muxer registration with its
///   video-capable sibling [`ContainerFormat::Wmv`] and declares
///   `msmpeg4v3`. Both genuinely mux video (verified live); rdlp declines to
///   target video at them purely by container-naming convention, the same
///   reasoning already written at `ContainerFormat`'s `M4a`/`Mp4` extension
///   split (`rdlp-types/src/container.rs`).
///
/// `Ogg` is deliberately excluded from this list: unlike the twelve above it
/// is a genuine dual-purpose container (Ogg Vorbis audio, Ogg Theora video),
/// and correctly resolves `FromMuxer` to `"theora"`.
///
/// Unlike the audio side's `Policy::NotATarget` (which changes
/// `select_audio_encoder_for_container`'s return type to `None`), the video
/// side's [`default_codec_for_container`] keeps an infallible `&'static str`
/// contract, so this arm still has to produce *a* value — it resolves to
/// [`DEFAULT_VIDEO_CODEC_STR`], which is never actually muxed since nothing in
/// this codebase recodes a video stream into these twelve containers.
//
// Mutation-verified exhaustiveness guard: this match MUST have no `_` arm.
// Delete a single arm's container from the `FromMuxer` list (e.g.
// `| ContainerFormat::Vob`) and `cargo check -p rdlp-ffmpeg` fails with
// E0004 (non-exhaustive match) — confirmed by hand during review. Rust's
// exhaustiveness check is a compile-time property and cannot be probed from
// within a passing test binary, so this is a comment, not a test (Minor-7 of
// PR-3's review: the prior anchor test's name promised an exhaustiveness
// check its body could not perform).
const fn video_default_for(container: ContainerFormat) -> ContainerDefault<Video> {
    match container {
        // `AVOutputFormat.video_codec` is a legacy lowest-common-denominator
        // default, not a recommendation. Citations below name the FFmpeg
        // muxer *symbol*, not a line number — line numbers rot across
        // rebases (Item 13 of PR-3's re-review; verified against a live
        // checkout: 6 of 7 pinned line numbers had already drifted), the
        // symbol does not. The nine containers below are general-purpose
        // delivery containers whose declaration is historical, so rdlp
        // overrides it with h264 — see PR-3's Part A policy note for the
        // three independent proofs (FFmpeg's own `ff_hls_muxer`/`ff_dash_muxer`
        // disagree with the raw mpegts/mxf muxers over the same payload;
        // `FF_OFMT_FLAG_ONLY_DEFAULT_CODECS` frames deference as opt-in, not
        // universal).
        ContainerFormat::Ts => {
            // `ff_mpegts_muxer.p.video_codec` declares mpeg2video;
            // `ff_hls_muxer.p.video_codec` declares h264 for the identical
            // MPEG-TS payload. ITU-T H.222.0 stream_type 0x1B = AVC, and
            // every real-world HLS ladder is h264 — the muxer's own default
            // is the historical outlier here.
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::Flv => {
            // Adobe FLV spec CodecID 7 = AVC; the muxer declares flv1
            // (Sorenson H.263), a codec no encoder ships for in 2026.
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::F4v => {
            // ISOBMFF-derived via movenc; h264 is F4V's actual install base
            // (Flash Video's MP4-compatible successor format).
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::Avi => {
            // avienc.c declares mpeg4 part 2. AVI is FOURCC-dispatched with
            // no codec binding at the container level, so h264-in-AVI is
            // standard modern practice.
            //
            // PSNR-measurement-artifact retraction (this policy's load-bearing
            // note, Item 15 of PR-3's re-review — `ffmpeg -lavfi psnr` looked
            // like corruption at first and would have reverted this and the
            // ASF/WMV arms below): `-lavfi psnr` aligns frames by
            // `best_effort_timestamp`, which falls back to DTS; AVI/ASF carry
            // no PTS, so the *filter* desyncs and reports ~23 dB. H.264
            // reorders from in-bitstream POC, not from container timestamps —
            // verified 47.44 dB decoding through VLC 3.0.23 and mpv 0.41.0,
            // and an ordinal frame compare bypassing timestamp sync gives
            // 45.67 dB. Residual caveat: mpv logs `No video PTS! Making
            // something up`, i.e. playback timestamps are synthesized —
            // benign for the CFR output rdlp produces, but would drift on a
            // VFR source, and B-frame seeking in legacy AVI parsers stays
            // imprecise.
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::Asf | ContainerFormat::Wmv => {
            // asfenc.c declares msmpeg4v3. Like AVI, ASF/WMV are
            // BITMAPINFOHEADER-dispatched with no codec binding, so the same
            // reasoning applies — including the PSNR-measurement-artifact
            // retraction on the `Avi` arm above (`-lavfi psnr` desyncs on
            // ASF/WMV's missing PTS the same way it does on AVI's).
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::Nut => {
            // nutenc.c declares mpeg4. NUT is codec-agnostic by
            // construction — it has no legacy install base to defer to.
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::Mxf => {
            // mxfenc.c declares mpeg2video, but AVC-in-MXF is standardized
            // by SMPTE ST 381-3 (revised 2025) — not a workaround.
            //
            // Known separate failure (**#629**, not caused by this override):
            // `--recode-video=mxf` currently fails at mux time regardless of
            // which encoder is selected here, because the output stream's
            // frame rate is never set and `mxfenc` demands a non-zero one.
            // That failure sits upstream of codec choice — do not "fix" this
            // arm in response to it.
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
        }
        ContainerFormat::ThreeGp => {
            // 3GPP TS 26.244 §5 lists AVC (H.264) among 3GP's permitted
            // video codecs; the muxer's own declared default predates that
            // profile and is not what modern 3GP consumers expect.
            ContainerDefault::new(Policy::Override(CodecName::from_static("h264")))
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
        | ContainerFormat::Wma => ContainerDefault::new(Policy::NotATarget),
        // Why these stay `FromMuxer` while the nine above are `Override`
        // (Item 16(a) of PR-3's re-review, correcting the review's own first
        // draft of this reasoning): it is NOT that these are merely
        // "build-capability probes, not codec opinions" — overriding also
        // hard-fails on a build lacking libx264/libopenh264 for the nine
        // `Override` containers, and rdlp did that anyway. The real
        // distinction is that for `Mp4`/`Mkv`/`Mov`/`M4v` the muxer's
        // declaration ALREADY EQUALS rdlp's opinion, so overriding would buy
        // no correctness and would cost the graceful `mpeg4`/`vp8`
        // degradation on a thinner build; for the nine `Override` containers
        // the declaration is *wrong* (see each arm's citation above), so
        // overriding buys correctness and the degraded path was undesirable
        // anyway. `Mp4`/`Mkv`/`Mov` declare h264 (build-conditional on
        // `CONFIG_LIBX264_ENCODER`, else mpeg4 — `ff_mov_muxer.p.video_codec`,
        // `ff_matroska_muxer.p.video_codec`) — see Important-1's `matches!`
        // guard on the tests below for why that isn't pinned as an exact
        // literal. `M4v` is grouped here too but is NOT build-conditional:
        // it resolves to `ff_ipod_muxer` (extensions `"m4v,m4a,m4b"`), whose
        // `.p.video_codec = AV_CODEC_ID_H264` is unconditional — a separate
        // muxer registration from `Mp4`'s, which merely happens to declare
        // the same codec on this build (Item 14 of PR-3's re-review).
        //
        // `Mpg`/`Vob`: FFmpeg's MPEG-PS muxer will happily write libx264
        // into these (measured: 50/50 frames decode clean), which is
        // spec-illegal — VOB/SVCD mandate MPEG-1/MPEG-2 video. `FromMuxer`
        // gives the right answer here by accident; do not "fix" this to an
        // override.
        ContainerFormat::WebM
        | ContainerFormat::Mp4
        | ContainerFormat::Mkv
        | ContainerFormat::Mov
        | ContainerFormat::M4v
        | ContainerFormat::Mpg
        | ContainerFormat::Vob
        | ContainerFormat::Dv
        | ContainerFormat::Ivf
        | ContainerFormat::Ogg => ContainerDefault::new(Policy::FromMuxer),
    }
}

/// Return the default video codec to encode toward for a given container.
///
/// E.g. [`ContainerFormat::Ts`] → `"h264"`, unconditionally;
/// [`ContainerFormat::WebM`]/[`ContainerFormat::Mp4`] → whatever the linked
/// `FFmpeg` build's own muxer declares, typically `"vp9"`/`"h264"` on a
/// full-featured build — see the build-conditional `matches!` guards on the
/// tests below, which exist precisely because these two are NOT fixed
/// values.
///
/// This is the **single source of truth** for the container → codec mapping;
/// both the speed-control validator and `RecodeStage` delegate here so they
/// can never resolve differently.
///
/// Of the 31 [`ContainerFormat`] variants, 10 defer to the linked build's own
/// muxer declaration (`FromMuxer`), 9 override it (`Override`), and 12 are
/// declared not a video target (`Policy::NotATarget`) — see [`video_default_for`]
/// for the exact classification and per-arm citations (corrected count, Item
/// 16(b) of PR-3's re-review: the prior prose said "most" defer, which was
/// true before Part A reclassified nine containers from `FromMuxer` to
/// `Override`). rdlp keeps no second copy of `FFmpeg`'s table for the ten
/// that do defer. Deviations are named in [`video_default_for`] with their
/// reasons: the
/// `FromMuxer` arm falls back to [`DEFAULT_VIDEO_CODEC_STR`] only when
/// `declared_codec` returns `None` (which does NOT mean "this container
/// carries no video" — see that function's doc comment — an ABI-skew codec id
/// or no muxer claiming the extension both produce it too); the twelve
/// audio-only containers always resolve [`DEFAULT_VIDEO_CODEC_STR`] via the
/// dedicated `Policy::NotATarget` (video kind) regardless of what their
/// muxer declares.
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
///
/// Requires [`super::ensure_init`] to have been called first (self-initializes
/// here, matching the two siblings in this module that already call `FFmpeg`).
#[must_use]
pub fn default_codec_for_container(container: ContainerFormat) -> &'static str {
    super::ensure_init().ok();
    let default = video_default_for(container);
    match default.policy() {
        // `codec` may hold a `Cow::Owned` in the general case (`Policy::Override`
        // is a public constructor), so this is the one arm that cannot use
        // `DEFAULT_VIDEO_CODEC_STR` directly; it degrades to the same fallback
        // an unclassifiable container already resolves to.
        Policy::Override(codec) => codec
            .clone()
            .into_static()
            .unwrap_or(DEFAULT_VIDEO_CODEC_STR),
        Policy::NotATarget => DEFAULT_VIDEO_CODEC_STR,
        Policy::FromMuxer => {
            muxer_defaults::declared_codec(container, codec_registry::MediaKind::Video)
                // `declared_codec` validates FFmpeg's own static descriptor-table
                // name via `CodecName::new_static`, so it is always recoverable
                // as `&'static str` here — see `MediaName::into_static`.
                .and_then(rdlp_types::media_name::CodecName::into_static)
                .unwrap_or(DEFAULT_VIDEO_CODEC_STR)
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
    // `resolve_encoder` resolves only ever through `VIDEO_REGISTRY`'s
    // `from_static` table entries, so its result always recovers as
    // `&'static str` — see `MediaName::into_static`. Keeps this function's
    // own `&'static str` return type, so its CLI/desktop pre-flight callers
    // need no change.
    if let Some(ve) = video_encoder.filter(|s| !s.is_empty()) {
        return resolve_encoder(ve).and_then(rdlp_types::media_name::VideoEncoderName::into_static);
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
        .map_or(DEFAULT_VIDEO_CODEC_STR, default_codec_for_container);
    resolve_encoder(codec).and_then(rdlp_types::media_name::VideoEncoderName::into_static)
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
    // `encoder` arrives as a raw `&str` from the public `Option<&str>` API
    // boundary (an operator-supplied CLI/config value, not yet a validated
    // vocabulary member) — construct the typed name here rather than widen
    // `speed_controls_def`'s signature back to `&str`. An encoder string that
    // fails `MediaName` validation (empty, non-ASCII, whitespace, a control
    // character) can never match a real table row either, so it degrades to
    // "no knobs for this encoder" — the same answer an unrecognised-but-valid
    // name gets.
    let no_knobs: &[SpeedKnobDef] = &[];
    let knobs = rdlp_types::media_name::VideoEncoderName::new(encoder)
        .map_or(no_knobs, |name| speed_controls_def(&name));
    let def = knobs.iter().find(|k| k.field == field).ok_or_else(|| {
        SpeedControlError::NotApplicable {
            knob: field,
            encoder: encoder.to_string(),
        }
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

    /// Behaviour parity across the string → `ContainerFormat` conversion.
    /// `Ivf`/`Mpg`/`Vob`/`Ogg` are `FromMuxer` and NOT build-conditional
    /// (their declared codec doesn't depend on `CONFIG_LIBX264_ENCODER` /
    /// `CONFIG_LIBVPX_VP9_ENCODER`), so they stay exact pins. `Mp4`/`Mkv`/
    /// `WebM` ARE build-conditional (Important-1 of PR-3's review) and are
    /// asserted separately with `matches!` below rather than pinned exactly.
    #[test]
    fn resolver_matches_recode_stage_defaults() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        for (container, want) in [
            (ContainerFormat::Ivf, "vp8"),
            (ContainerFormat::Ogg, "theora"),
            (ContainerFormat::Mpg, "mpeg1video"),
            (ContainerFormat::Vob, "mpeg2video"),
        ] {
            assert_eq!(
                default_codec_for_container(container),
                want,
                "{container:?}"
            );
        }

        // `ff_mov_muxer.p.video_codec` / `ff_matroska_muxer.p.video_codec`:
        // CONFIG_LIBX264_ENCODER ? h264 : mpeg4.
        for container in [ContainerFormat::Mp4, ContainerFormat::Mkv] {
            assert!(
                matches!(default_codec_for_container(container), "h264" | "mpeg4"),
                "{container:?}"
            );
        }
        // `ff_webm_muxer.p.video_codec`: CONFIG_LIBVPX_VP9_ENCODER ? vp9 : vp8.
        assert!(matches!(
            default_codec_for_container(ContainerFormat::WebM),
            "vp9" | "vp8"
        ));
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
    /// is a `ContainerFormat` alias for `Mpg`, so it now resolves to
    /// `mpeg1video` where the old literal `"mpg" | "vob"` compare dropped it
    /// to h264 (Minor-8 of PR-3's review: the prose here previously said
    /// "mpeg2", which was never what `Mpg` resolves — `Mpg` is `mpeg1video`,
    /// `Vob` is `mpeg2video`; the body's relative comparison was always
    /// correct, only the prose drifted). Deliberate, documented at the parse
    /// site, and pinned here.
    #[test]
    fn mpeg_alias_resolves_like_mpg() {
        assert_eq!(
            resolve_recode_encoder(None, Some("mpeg"), None),
            resolve_recode_encoder(None, Some("mpg"), None),
            "the `mpeg` alias must resolve exactly as `mpg` does"
        );
    }

    /// `3gpp` (→ `ThreeGp`) and `mpegts` (→ `Ts`, since Part A of PR-3's
    /// review reclassified `Ts` from `FromMuxer` to `Override("h264")`) both
    /// resolve to containers with an UNCONDITIONAL `Override("h264")`
    /// classification — no `FFmpeg` build flag changes the answer. The oracle
    /// here MUST be `resolve_encoder(DEFAULT_VIDEO_CODEC.as_str())` directly
    /// (Important-5 of PR-3's review), not `resolve_recode_encoder(None,
    /// Some("mp4"), None)`: `Mp4` is `FromMuxer` and build-conditional (see
    /// `resolver_matches_recode_stage_defaults`), so on a build lacking both
    /// libx264 and libopenh264 it would resolve `"mpeg4"` while these
    /// unconditional-override aliases resolve `None` — the two oracles would
    /// silently diverge.
    #[test]
    fn override_aliases_resolve_to_the_h264_default() {
        for alias in ["3gpp", "mpegts"] {
            assert_eq!(
                resolve_recode_encoder(None, Some(alias), None),
                resolve_encoder(DEFAULT_VIDEO_CODEC.as_str())
                    .as_ref()
                    .map(rdlp_types::media_name::MediaName::as_str),
                "{alias} must resolve exactly as the unconditional h264 default does"
            );
        }
    }

    /// `matroska` (→ `Mkv`) and `quicktime` (→ `Mov`) are `FromMuxer` and
    /// build-conditional on the exact same `CONFIG_LIBX264_ENCODER` flag as
    /// `Mp4` (`ff_mov_muxer.p.video_codec`, `ff_matroska_muxer.p.video_codec`),
    /// so comparing them
    /// against `Mp4`'s own resolution — rather than the unconditional
    /// `DEFAULT_VIDEO_CODEC` oracle used above — stays valid on any build:
    /// all three flip together.
    #[test]
    fn from_muxer_aliases_track_mp4s_build_conditional_default() {
        for alias in ["matroska", "quicktime"] {
            assert_eq!(
                resolve_recode_encoder(None, Some(alias), None),
                resolve_recode_encoder(None, Some("mp4"), None),
                "{alias} must resolve exactly as mp4's build-conditional default does"
            );
        }
    }

    /// `resolve_recode_encoder` still takes strings (config-supplied), so its
    /// unparseable-input path must keep the old `_ => "h264"` behaviour rather
    /// than silently resolving to nothing. Oracle fixed to
    /// `resolve_encoder(DEFAULT_VIDEO_CODEC.as_str())` for the same reason as
    /// `override_aliases_resolve_to_the_h264_default` above (Important-5):
    /// the unparseable-string fallback resolves `DEFAULT_VIDEO_CODEC`
    /// directly, so `Mp4`'s build-conditional resolution is the wrong oracle.
    #[test]
    fn unparseable_container_string_still_falls_back_to_h264() {
        assert_eq!(
            resolve_recode_encoder(None, Some("not-a-container"), None),
            resolve_encoder(DEFAULT_VIDEO_CODEC.as_str())
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
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

    /// The nine containers Part A of PR-3's review classifies
    /// `Override("h264")` — general-purpose delivery containers whose muxer
    /// declaration is historical (see each arm's citation in
    /// `video_default_for`). Unlike the `FromMuxer` group, these are
    /// unconditional: no `FFmpeg` build-time flag changes the answer, so they
    /// stay exact pins. Supersedes the old `broadcast_containers_get_their_declared_codecs`
    /// (which pinned `Ts`/`Wmv`/`Nut`/`Mxf` to their PRE-Part-A `FromMuxer`
    /// values — wrong under the new policy) and the old
    /// `legacy_containers_keep_their_deliberate_h264_override` (which only
    /// covered three of the nine).
    #[test]
    fn override_containers_get_the_deliberate_h264_override() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        for container in [
            ContainerFormat::Ts,
            ContainerFormat::Flv,
            ContainerFormat::F4v,
            ContainerFormat::Avi,
            ContainerFormat::Asf,
            ContainerFormat::Wmv,
            ContainerFormat::Nut,
            ContainerFormat::Mxf,
            ContainerFormat::ThreeGp,
        ] {
            assert_eq!(
                default_codec_for_container(container),
                "h264",
                "{container:?}"
            );
        }
    }

    /// The "exactly these nine" half of the `Override` policy, mirroring the
    /// audio registry's `overrides_are_exactly_these_four`: no OTHER
    /// container may be classified `Override`. Without this, adding e.g.
    /// `Dv => Override("h264")` to `video_default_for` would leave
    /// `override_containers_get_the_deliberate_h264_override` green.
    #[test]
    fn video_override_set_is_exactly_these_nine() {
        use strum::IntoEnumIterator;

        let known = [
            ContainerFormat::Ts,
            ContainerFormat::Flv,
            ContainerFormat::F4v,
            ContainerFormat::Avi,
            ContainerFormat::Asf,
            ContainerFormat::Wmv,
            ContainerFormat::Nut,
            ContainerFormat::Mxf,
            ContainerFormat::ThreeGp,
        ];
        for container in ContainerFormat::iter() {
            let is_override = matches!(video_default_for(container).policy(), Policy::Override(_));
            assert_eq!(
                is_override,
                known.contains(&container),
                "{container:?}: Override classification does not match the known set of nine"
            );
        }
    }

    /// `Mp4`/`Mkv`/`WebM` stay `FromMuxer` and build-conditional (Important-1
    /// of PR-3's review), so they're asserted with `matches!` rather than
    /// pinned exactly — see `resolver_matches_recode_stage_defaults` for the
    /// same guard with the `FFmpeg` source citations.
    #[test]
    fn mainstream_containers_are_unchanged() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(matches!(
            default_codec_for_container(ContainerFormat::Mp4),
            "h264" | "mpeg4"
        ));
        assert!(matches!(
            default_codec_for_container(ContainerFormat::Mkv),
            "h264" | "mpeg4"
        ));
        assert!(matches!(
            default_codec_for_container(ContainerFormat::WebM),
            "vp9" | "vp8"
        ));
        assert_eq!(default_codec_for_container(ContainerFormat::Ogg), "theora");
    }

    /// Regression guard for the "not a video target" policy decision: `Mp3`,
    /// `Flac`, and `Aiff` all declare `png` as their `MediaKind::Video`
    /// default (`FFmpeg`'s embedded-cover-art codec for `ID3v2` APIC /
    /// `METADATA_BLOCK_PICTURE`) and `Wma` declares `msmpeg4v3` (inherited
    /// from sharing the `asf` muxer registration with `Wmv`) — verified by
    /// direct probe against this build. A `FromMuxer` classification would
    /// leak these as the "default video codec" for audio-only containers;
    /// `Policy::NotATarget` must produce `DEFAULT_VIDEO_CODEC` instead.
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

    /// The "exactly these twelve" half of the "not a video target" policy: no
    /// OTHER container may be classified `Policy::NotATarget` for video, and
    /// `Ogg` — despite
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
                matches!(video_default_for(container).policy(), Policy::NotATarget);
            assert_eq!(
                is_no_video_target,
                known.contains(&container),
                "{container:?}: Policy::NotATarget (video) classification does not match \
                 the known set of twelve audio-only containers"
            );
        }
    }

    /// Important-3 of PR-3's review: the superseded `every_containers_own_default_codec_is_accepted_by_the_matrix`
    /// was misnamed (it never called `resolve_encoder` — nothing there
    /// touched "the matrix") AND tautological on the `FromMuxer` arm: its
    /// `expected` was derived from `video_default_for` + `declared_codec`,
    /// the SAME policy function and FFI call `default_codec_for_container`
    /// itself uses, so for `FromMuxer` containers it reduced to
    /// `declared_codec(c) == declared_codec(c)` — incapable of catching a
    /// misclassification, because `expected` followed the misclassification.
    ///
    /// This is the assertion the old name promised: for every container this
    /// codebase actually targets for video (all but the twelve classified
    /// `Policy::NotATarget`), the resolved default codec must have an AVAILABLE
    /// encoder in this build. This is also the exact check that would have
    /// caught the `dvvideo`/`msmpeg4v3` gap by hand — before those rows
    /// existed in `CODEC_PREFERENCES`, `default_codec_for_container(Dv)`
    /// correctly returned `"dvvideo"` but `resolve_encoder("dvvideo")`
    /// returned `None` (no row matched by codec-key or literal-encoder-name),
    /// so `--recode-video=dv` failed with "pipeline terminated with no
    /// output and no error" — the exact silent-no-op shape Important-4 also
    /// closes at the `RecodeStage` call site.
    ///
    /// This is a deliberate pin on a full-featured `FFmpeg` build (Item 19 of
    /// PR-3's re-review, mirroring `muxer_defaults.rs`'s
    /// `every_container_resolves_and_only_ivf_lacks_audio`): it requires
    /// libtheora (`Ogg`), libvpx (`Ivf`/`WebM`) and an available h264 encoder
    /// to ALL be linked, alongside every other codec this policy resolves
    /// toward. On a build missing any of those — `--disable-muxer`,
    /// `--disable-encoder`, a minimal vendored build — this test fails for a
    /// real and meaningful reason: rdlp genuinely cannot recode to that
    /// container on that build. Unlike the value-pinning tests above, there
    /// is no `matches!` softening available here — if it fires, the fix is
    /// widening the expected set (or accepting the build genuinely lacks the
    /// capability), not treating it as a real regression.
    #[test]
    fn every_video_targetable_containers_default_codec_has_an_available_encoder() {
        use strum::IntoEnumIterator;
        crate::ffmpeg::ensure_init().expect("ffmpeg init");

        for container in ContainerFormat::iter() {
            if matches!(video_default_for(container).policy(), Policy::NotATarget) {
                continue;
            }
            let codec = default_codec_for_container(container);
            assert!(
                resolve_encoder(codec).is_some(),
                "{container:?}: default codec {codec:?} has no available encoder in \
                 this FFmpeg build"
            );
        }
    }

    /// Item 18(a) of PR-3's re-review: a user-visible boundary move with no
    /// prior pin. Pre-Part-A, `--recode-video=dv` resolved libx264 (via the
    /// old hand-copied table's fallback), which has preset knobs, so
    /// `--recode-preset=…` was silently accepted. Since wave 1's `dvvideo`
    /// row, `dv` resolves to its own real encoder (`dvvideo`), which has no
    /// preset knob at all — more correct (x264-in-.dv made `FFmpeg` refuse the
    /// header), but a regression in the narrow sense that this exact
    /// combination used to be accepted and now errors.
    ///
    /// This is a deliberate pin on a full-featured `FFmpeg` build: if it
    /// fires on a minimal build lacking `dvvideo`, the fix is widening the
    /// expected set, not treating it as a real regression.
    #[test]
    fn dv_preset_now_errors_not_applicable_to_dvvideo() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let encoder = resolve_recode_encoder(None, Some("dv"), None);
        assert_eq!(
            encoder,
            Some("dvvideo"),
            "dv must resolve to its real encoder, not libx264"
        );
        let err = validate_speed_controls(encoder, Some("medium"), None, None, None)
            .expect_err("dvvideo has no preset knob");
        assert!(matches!(
            err,
            SpeedControlError::NotApplicable {
                knob: KnobField::Preset,
                ..
            }
        ));
    }

    /// Item 18(b): `--recode-video=ivf` resolves `vp8` → `libvpx`, whose
    /// `cpu-used` range is `-16..=16` — wider than `libvpx-vp9`'s `-8..=8`
    /// that the old hand-copied table implicitly assumed for any VPX target.
    /// `9` is out of range for vp9 (see `cpu_used_range_is_per_encoder`
    /// above) but in range here — untested until now.
    ///
    /// This is a deliberate pin on a full-featured `FFmpeg` build: if it
    /// fires on a minimal build lacking `libvpx`, the fix is widening the
    /// expected set, not treating it as a real regression.
    #[test]
    fn ivf_cpu_used_range_widened_from_vp9_to_vp8() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let encoder = resolve_recode_encoder(None, Some("ivf"), None);
        assert_eq!(
            encoder,
            Some("libvpx"),
            "ivf must resolve to libvpx (VP8), not libvpx-vp9"
        );
        assert!(
            validate_speed_controls(encoder, None, None, Some(9), None).is_ok(),
            "9 is within libvpx's -16..=16 cpu-used range"
        );
    }
}
