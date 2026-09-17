//! `--recode-video` on an audio file that carries embedded cover art must take
//! the audio-only route (#643).
//!
//! Demuxers force `codec_type = VIDEO` onto an attached picture
//! (`libavformat/demux_utils.c`, `ff_add_attached_pic`), so a probe that
//! counted every video-medium stream as video classified such a file as a
//! video source, asked `can_remux_video` about the *mjpeg still*, and
//! transcoded one cover image into a video track. The #637 matrix could not
//! see this: its fixtures carry no artwork.
//!
//! The fixture is built in-process (`rdlp_ffmpeg::test_support`) through the
//! crate's own embed path, so it carries exactly the `ATTACHED_PIC` stream
//! rdlp writes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::{make_msg, opts};
use rdlp_ffmpeg::test_support::{SineAudio, StillImage, write_sine_audio_with_cover};
use rdlp_postprocess::PostProcess;
use rdlp_postprocess::pipeline::PipelineStage as _;
use rdlp_postprocess::pipeline::stages::RecodeStage;
use rdlp_types::ContainerFormat;

#[tokio::test]
async fn recode_of_audio_with_cover_art_takes_the_audio_only_route() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ffmpeg = Arc::new(rdlp_ffmpeg::FFmpegRunner::new().expect("FFmpegRunner"));

    let src = dir.path().join("with_cover.m4a");
    write_sine_audio_with_cover(&src, &SineAudio::default(), &StillImage::default())
        .expect("fixture");
    let probe = ffmpeg.probe(&src).await.expect("probe fixture");
    assert!(
        probe.has_attached_picture() && !probe.has_video,
        "fixture must be cover-art audio: {probe:?}"
    );

    // Two targets: one that carries AAC by copy, one that must re-encode.
    for target in [ContainerFormat::Mkv, ContainerFormat::Ogg] {
        let input = dir.path().join(format!("in_{}.m4a", target.as_ext()));
        tokio::fs::copy(&src, &input).await.expect("copy fixture");
        let config = PostProcess {
            recode_video: Some(target),
            ..PostProcess::default()
        };
        let msg = make_msg(vec![input], config, opts("with_cover", false));
        let out = RecodeStage::new(Arc::clone(&ffmpeg))
            .process(msg)
            .await
            .unwrap_or_else(|e| panic!("{target}: audio-only route must succeed: {e:#}"));

        let info = ffmpeg
            .probe(&out.tracker.primary())
            .await
            .expect("probe output");
        assert!(
            info.has_audio,
            "{target}: output must carry audio: {info:?}"
        );
        assert!(
            !info.has_video,
            "{target}: the cover must not have been transcoded into a video track: {info:?}"
        );
        // The audio-only route's tag names the audio outcome alone — no
        // video encoder ran, so none may be claimed.
        let tool = out.encoding_tool.as_deref().unwrap_or_default();
        assert!(
            !tool.contains(" + "),
            "{target}: encoding_tool {tool:?} claims a video component"
        );
    }
}
