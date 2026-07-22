//! The audio-only branch of `RecodeStage` (#637).
//!
//! `--recode-video` on a source with no video stream used to fail for 12 of 16
//! containers: the transcode path requires video (`open_input_and_decoder`
//! ends at `NoVideoStream`) and nothing gated the stage on `has_video`. Eight
//! of those failures were false — a plain stream copy works. Neither ffmpeg
//! nor yt-dlp imposes a video precondition on a container conversion, and
//! `RemuxStage` never did either.
//!
//! Kept in its own file rather than in `recode.rs`: that file is already well
//! past the 500-line guidance in `CODING_RULES.md`, and this branch is a
//! self-contained decision (which container, which audio codec, copy or
//! encode) that shares nothing with the video route except
//! `resolve_audio_params`. Free functions rather than more methods on
//! `RecodeStage`, so the `impl` block is not split across files.

use log::{debug, warn};

use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner, PostProcessError};
use rdlp_types::{ContainerFormat, RecodeAudioMode};

use super::audio_convert;
use super::recode::RecodeStage;
use crate::pipeline::PipelineMessage;

/// What the probe established about the source's audio stream.
///
/// The audio mirror of `SourceVideo`, and it exists for the same reason: a
/// bare `Option<&str>` merges "no audio stream at all" with "an audio stream
/// this build cannot name", and the audio-only route needs to tell them apart
/// — the first has nothing to convert, the second can still be re-encoded
/// through the muxer's default.
#[derive(Debug, Clone, Copy)]
pub(super) enum SourceAudio<'a> {
    /// No audio stream.
    Absent,
    /// An audio stream whose codec this build cannot name. Proves nothing
    /// about representability, so it must not authorise a stream copy — but
    /// it is still audio, and still convertible.
    Unnamed,
    /// The source's audio codec, as `FFmpeg`'s own descriptor name.
    Codec(&'a str),
}

impl<'a> SourceAudio<'a> {
    /// Classify what `probe` reported.
    pub(super) const fn from_probe(has_audio: bool, audio_codec: Option<&'a str>) -> Self {
        match (has_audio, audio_codec) {
            (false, _) => Self::Absent,
            (true, Some(codec)) => Self::Codec(codec),
            (true, None) => Self::Unnamed,
        }
    }

    /// Whether there is an audio stream at all, named or not.
    const fn is_present(self) -> bool {
        !matches!(self, Self::Absent)
    }

    /// The codec name, when the build could name it.
    const fn name(self) -> Option<&'a str> {
        match self {
            Self::Codec(c) => Some(c),
            Self::Absent | Self::Unnamed => None,
        }
    }
}

/// Stage name reported in logs and the callback factory.
const STAGE_NAME: &str = "RecodeStage";

/// Whether an **audio-only** source can be stream-copied into `output`
/// (#637).
///
/// Deliberately a separate function from `RecodeStage::can_remux_video` rather
/// than a branch inside it. The two ask different questions — "can this
/// container carry that *video* codec" versus "…that *audio* codec" — and
/// folding them together is what produced #637: every per-container arm of
/// the video predicate answers about video, and none of them can answer
/// for a file that has none. Four were hardcoded to `true` (which picked a
/// remux MXF then refused at `write_header`) and the rest fell through to
/// a codec check or an extension compare that routed to a transcode dying
/// in `open_input_and_decoder` with `NoVideoStream`.
///
/// Asked of the same #630 predicate with `MediaKind::Audio`, so routing
/// and the mux agree here exactly as they do for video. An unnamed or
/// absent codec is no positive evidence a copy works, so it re-encodes.
pub(super) fn can_copy_audio_only(output: ContainerFormat, audio: SourceAudio<'_>) -> bool {
    audio.name().is_some_and(|codec| {
        rdlp_ffmpeg::muxer_can_represent(output, codec, rdlp_ffmpeg::MediaKind::Audio)
    })
}

/// Convert a source that has no video stream into `target` (#637).
///
/// Rebuilding a video-free path through `convert_video` would mean a
/// second implementation of decode → encode → mux, so this delegates to
/// `extract_audio` (via the shared [`audio_convert::run_audio_extract`]),
/// which is already exactly that pipeline for a single audio stream and —
/// since #638 — adapts frames to any encoder.
///
/// Three outcomes, none of them new machinery:
///
/// 1. the container can carry the source's audio codec → stream copy,
///    decided by [`can_copy_audio_only`];
/// 2. it cannot (`webm` + aac) → re-encode to the container's own default
///    via [`RecodeStage::resolve_audio_params`], which is yt-dlp's behaviour;
/// 3. it cannot hold an audio-only file at all (`mxf` requires exactly one
///    video stream) → the muxer's own refusal surfaces unchanged, naming
///    the real requirement. Since #632 that error is truthful, so there is
///    deliberately no special case for it here.
pub(super) async fn recode_audio_only(
    ffmpeg: &FFmpegRunner,
    mut msg: PipelineMessage,
    target: ContainerFormat,
    audio: SourceAudio<'_>,
) -> anyhow::Result<PipelineMessage> {
    let input_file = msg.tracker.primary();
    let target_ext = target.as_ext();

    let recode_audio = RecodeStage::resolve_recode_audio_mode(&msg);
    let (audio_copy, encoder_name) = resolve_audio_only_params(&recode_audio, target, audio)?;

    let output_path = msg.tracker.temp_path(&input_file, target_ext);
    let opts = AudioExtractOptions {
        encoder_name,
        copy: audio_copy,
        bitrate_kbps: None,
        quality_scale: None,
    };
    let summary = format!(
        "Recode: audio-only source, container={target_ext}, audio={}",
        rdlp_ffmpeg::ffmpeg::audio_tag_component(opts.copy, opts.encoder_name.as_deref()),
    );

    audio_convert::run_audio_extract(
        ffmpeg,
        &mut msg,
        audio_convert::AudioExtractJob {
            stage_name: STAGE_NAME,
            input: input_file,
            output: output_path,
            opts,
            summary: Some(summary),
            // Every non-copy path here resolves a named encoder, so there
            // is no muxer-default case to describe.
            fallback_codec: None,
            error_context: "recode stage failed for audio-only source",
        },
    )
    .await?;

    Ok(msg)
}

/// Decide copy-vs-encode for an audio-only source, and which encoder.
///
/// Split out of [`recode_audio_only`] so the three-tier decision is
/// testable without running a mux.
pub(super) fn resolve_audio_only_params(
    recode_audio: &RecodeAudioMode,
    target: ContainerFormat,
    audio: SourceAudio<'_>,
) -> Result<(bool, Option<String>), PostProcessError> {
    let target_ext = target.as_ext();

    if can_copy_audio_only(target, audio) {
        debug!("RecodeStage: audio-only source → {target_ext} (stream copy)");
        return Ok((true, None));
    }

    // `Copy` is the default audio mode (`RecodeAudioMode::Copy` is
    // `#[default]`), and for a video recode the muxer can usually meet it.
    // Here it provably cannot: the target has already told us it does not
    // represent this codec, so honouring `Copy` would hand it a stream it
    // rejects and fail with a bare "Invalid argument" from `write_header`.
    // Falling back to the container's own default is what ffmpeg and
    // yt-dlp's `FFmpegVideoConvertorPP` both do — copy is a preference,
    // and an impossible preference yields to a working conversion.
    //
    // This is `warn!`, not `debug!`: it overrides something the user asked
    // for, and the conversion can be lossy in a way they did not choose —
    // a FLAC audio-only source into WebM is re-encoded to Opus, turning a
    // lossless input into a lossy output. The two neighbouring overrides
    // in `resolve_audio_params` are both `warn!` and are strictly less
    // consequential than this one.
    let effective = match recode_audio {
        RecodeAudioMode::Copy => RecodeAudioMode::Auto,
        other => other.clone(),
    };

    let (audio_copy, encoder_name) =
        RecodeStage::resolve_audio_params(&effective, target, audio.is_present())?;

    if matches!(recode_audio, RecodeAudioMode::Copy) && !audio_copy && audio.is_present() {
        // Two things this message is careful about.
        //
        // It does not name `--recode-audio=copy`: copy is also the default,
        // and `resolve_recode_audio_mode` forces it when `normalize_audio` is
        // on, so naming the flag would contradict that function's own "forcing
        // audio copy mode" warning for a user who never passed it.
        //
        // And it says rdlp cannot *confirm* the container carries the codec,
        // not that the container cannot — because for MPEG-TS the latter is
        // false. mpegts does carry AAC; rdlp declines the copy only because
        // the muxer advertises it through neither a codec-tag table nor
        // `query_codec`, so `oformat_can_represent` has no positive evidence
        // (#633). Claiming a container limitation that does not exist would
        // undercut the point of promoting this to `warn!`.
        //
        // Gated on `is_present` so a source with no audio at all does not warn
        // about a codec it does not have, immediately before failing with
        // `NoAudioStream`.
        warn!(
            "RecodeStage: cannot confirm {target_ext} carries {}, so it cannot be \
             stream-copied safely; re-encoding to {} instead (this may be lossy)",
            audio.name().unwrap_or("the source audio codec"),
            encoder_name.as_deref().unwrap_or("the container default"),
        );
    }

    if !audio_copy {
        debug!(
            "RecodeStage: audio-only source → {target_ext} (re-encoding to {})",
            encoder_name.as_deref().unwrap_or("container default"),
        );
    }

    Ok((audio_copy, encoder_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tier 1: the container carries the codec, so copy — and report no
    /// encoder, which is what makes `encoding_tool` read "copy".
    #[test]
    fn audio_only_params_copy_when_representable() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (copy, encoder) = resolve_audio_only_params(
            &RecodeAudioMode::Copy,
            ContainerFormat::Mkv,
            SourceAudio::Codec("aac"),
        )
        .expect("mkv carries aac");
        assert!(copy);
        assert_eq!(encoder, None);
    }

    /// Tier 2: `WebM` cannot carry AAC, so an impossible `Copy` yields to the
    /// container's own default rather than failing at `write_header`.
    #[test]
    fn audio_only_params_downgrade_impossible_copy_to_the_container_default() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (copy, encoder) = resolve_audio_only_params(
            &RecodeAudioMode::Copy,
            ContainerFormat::WebM,
            SourceAudio::Codec("aac"),
        )
        .expect("webm resolves an encoder");
        assert!(!copy, "webm cannot stream-copy aac");
        assert!(
            encoder.is_some_and(|e| e.contains("opus") || e.contains("vorbis")),
            "webm must fall back to its own default audio encoder"
        );
    }

    /// An unnamed audio codec proves nothing, so it must not authorise a copy
    /// — but it is still audio and must still convert.
    #[test]
    fn audio_only_params_reencode_when_the_codec_is_unnamed() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (copy, encoder) = resolve_audio_only_params(
            &RecodeAudioMode::Copy,
            ContainerFormat::Mkv,
            SourceAudio::Unnamed,
        )
        .expect("mkv resolves an encoder");
        assert!(!copy);
        assert!(encoder.is_some(), "an unnamed codec still re-encodes");
    }

    /// An audio-only source copies into any container that can carry its
    /// **audio** codec — the question #637 replaced the old blanket `true`
    /// with. `--recode-video=mkv` on an `.m4a` worked before #630 and must
    /// keep working, and now mp4/mov/flv/3gp/asf do too.
    #[test]
    fn audio_only_input_copies_when_the_container_carries_its_audio_codec() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        for target in [
            ContainerFormat::Mkv,
            ContainerFormat::Nut,
            ContainerFormat::Avi,
            ContainerFormat::Mka,
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Flv,
            ContainerFormat::ThreeGp,
            ContainerFormat::Asf,
        ] {
            assert!(
                can_copy_audio_only(target, SourceAudio::Codec("aac")),
                "{target:?}: carries aac, so an audio-only source must stream-copy"
            );
        }
    }

    /// The `Absent => true` blanket was too broad for MXF, which requires
    /// **exactly one video stream**: it picked a remux that could not work and
    /// failed at `write_header`. Now the audio question is asked, MXF answers
    /// no (its muxer declares nothing via a tag table or `query_codec`), and
    /// the refusal names the real requirement. #637's "1 too-broad `true`".
    #[test]
    fn audio_only_input_does_not_copy_into_mxf() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(
            !can_copy_audio_only(ContainerFormat::Mxf, SourceAudio::Codec("aac")),
            "mxf needs exactly one video stream; an audio-only copy cannot work"
        );
    }

    /// Containers that genuinely cannot carry AAC must not be routed to a
    /// stream copy — they re-encode to their own default instead (#637 tier 2).
    #[test]
    fn audio_only_input_does_not_copy_into_a_container_that_rejects_its_codec() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        for target in [
            ContainerFormat::WebM,
            ContainerFormat::Mpg,
            ContainerFormat::Ogg,
        ] {
            assert!(
                !can_copy_audio_only(target, SourceAudio::Codec("aac")),
                "{target:?} cannot carry aac, so it must re-encode rather than copy"
            );
        }
    }

    /// No audio codec — no audio stream at all, or one this build cannot name
    /// — is not evidence a copy works, so it must not authorise one. Mirrors
    /// `SourceVideo::Unnamed`'s rule on the video side.
    #[test]
    fn audio_only_input_without_a_named_codec_does_not_copy() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        for audio in [SourceAudio::Absent, SourceAudio::Unnamed] {
            assert!(
                !can_copy_audio_only(ContainerFormat::Mkv, audio),
                "{audio:?} proves nothing about representability, so it must not copy"
            );
        }
    }
}
