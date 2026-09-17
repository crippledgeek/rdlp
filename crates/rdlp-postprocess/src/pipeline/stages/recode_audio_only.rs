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

use rdlp_ffmpeg::ffmpeg::source::SourceAudio;
use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner};
use rdlp_types::ContainerFormat;

use super::audio_convert;
use super::policy_refusal::keep_download_on_policy_refusal;
use super::recode::RecodeStage;
use crate::pipeline::PipelineMessage;

/// Stage name reported in logs and the callback factory.
const STAGE_NAME: &str = "RecodeStage";

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
///    decided by <code>[muxer_decides]::<[Audio]></code>;
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
    audio: &SourceAudio,
) -> anyhow::Result<PipelineMessage> {
    let input_file = msg.tracker.primary();
    let target_ext = target.as_ext();

    let recode_audio = RecodeStage::resolve_recode_audio_mode(&msg);
    let (audio_copy, encoder_name) =
        match RecodeStage::resolve_audio_params(recode_audio.as_ref(), target, audio) {
            Ok(params) => params,
            Err(e) => {
                keep_download_on_policy_refusal(&e, &mut msg, STAGE_NAME);
                return Err(
                    anyhow::Error::new(e).context("recode stage failed for audio-only source")
                );
            }
        };

    let output_path = msg.tracker.temp_path(&input_file, target_ext);
    let opts = AudioExtractOptions {
        // `encoder_name` is not used again after this — move it rather than
        // cloning.
        encoder_name,
        copy: audio_copy,
        bitrate_kbps: None,
        quality_scale: None,
    };
    let summary = format!(
        "Recode: audio-only source, container={target_ext}, audio={}",
        rdlp_ffmpeg::ffmpeg::stream_tag_component(
            opts.copy,
            opts.encoder_name
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str)
        ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_ffmpeg::ffmpeg::source::{Audio, Source, muxer_decides};
    use rdlp_types::CodecName;

    /// Builds a [`SourceAudio`] whose codec is named.
    fn audio_codec(name: &'static str) -> SourceAudio {
        Source::from_probe(true, Some(CodecName::from_static(name)))
    }

    /// Builds a [`SourceAudio`] with a stream present but an unnameable codec.
    fn audio_unnamed() -> SourceAudio {
        Source::from_probe(true, None)
    }

    /// Builds a [`SourceAudio`] with no audio stream at all.
    fn audio_absent() -> SourceAudio {
        Source::from_probe(false, None)
    }

    /// The yes/no form of `muxer_decides::<Audio>`, for the tests below that
    /// only care about the answer.
    fn can_copy_audio_only(output: ContainerFormat, audio: &SourceAudio) -> bool {
        muxer_decides::<Audio>(output).eval(audio)
    }

    /// Tier 1: the container carries the codec, so copy — and report no
    /// encoder, which is what makes `encoding_tool` read "copy". The
    /// audio-only route shares `RecodeStage::resolve_audio_params` (#645);
    /// these pin it from this route's angle with the mode unspecified.
    #[test]
    fn audio_only_params_copy_when_representable() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (copy, encoder) =
            RecodeStage::resolve_audio_params(None, ContainerFormat::Mkv, &audio_codec("aac"))
                .expect("mkv carries aac");
        assert!(copy);
        assert_eq!(encoder, None);
    }

    /// Tier 2: `WebM` cannot carry AAC, so with no mode specified the copy
    /// preference yields to the container's own default rather than failing
    /// at `write_header`.
    #[test]
    fn audio_only_params_downgrade_impossible_copy_to_the_container_default() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (copy, encoder) =
            RecodeStage::resolve_audio_params(None, ContainerFormat::WebM, &audio_codec("aac"))
                .expect("webm resolves an encoder");
        assert!(!copy, "webm cannot stream-copy aac");
        assert!(
            encoder.is_some_and(|e| e.as_str().contains("opus") || e.as_str().contains("vorbis")),
            "webm must fall back to its own default audio encoder"
        );
    }

    /// An unnamed audio codec proves nothing, so it must not authorise a copy
    /// — but it is still audio and must still convert.
    #[test]
    fn audio_only_params_reencode_when_the_codec_is_unnamed() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (copy, encoder) =
            RecodeStage::resolve_audio_params(None, ContainerFormat::Mkv, &audio_unnamed())
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
            // #633: mpegts implements AAC but advertises it through neither a
            // codec-tag table nor `query_codec`, so this answered `false`
            // until the allow-list landed and an `.m4a → .ts` recode
            // needlessly re-encoded aac → mp2.
            ContainerFormat::Ts,
        ] {
            assert!(
                can_copy_audio_only(target, &audio_codec("aac")),
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
            !can_copy_audio_only(ContainerFormat::Mxf, &audio_codec("aac")),
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
                !can_copy_audio_only(target, &audio_codec("aac")),
                "{target:?} cannot carry aac, so it must re-encode rather than copy"
            );
        }
    }

    /// No audio codec — no audio stream at all, or one this build cannot name
    /// — is not evidence a copy works, so it must not authorise one. Mirrors
    /// <code>[Source]<[Video](rdlp_ffmpeg::ffmpeg::source::Video)></code>'s
    /// `Unnamed` rule on the video side.
    #[test]
    fn audio_only_input_without_a_named_codec_does_not_copy() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        for audio in [audio_absent(), audio_unnamed()] {
            assert!(
                !can_copy_audio_only(ContainerFormat::Mkv, &audio),
                "{audio:?} proves nothing about representability, so it must not copy"
            );
        }
    }
}
