//! A stream-copy remux carries embedded cover art as cover art, and drops it
//! where the target cannot carry a cover stream (#643).
//!
//! Two defects this pins, both found by the in-process fixture:
//! - the copy paths never carried the input disposition, unlike `ffmpeg`'s
//!   own stream copy (`fftools/ffmpeg_mux_init.c`, `set_dispositions`), so a
//!   remuxed cover landed as a plain one-frame video track;
//! - a target that declares no video codec (`wav`) refused or, after the
//!   first fix, failed at `avformat_write_header`; `ffmpeg`'s default stream
//!   selection drops the picture there, and so does rdlp now.
//!
//! Built in-process (`rdlp_ffmpeg::test_support`) — no `ffmpeg` binary.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rdlp_ffmpeg::test_support::{SineAudio, StillImage, write_sine_audio_with_cover};
use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions, StreamKind};
use rdlp_types::ContainerFormat;

#[tokio::test]
async fn remux_keeps_cover_art_as_cover_art_where_the_target_carries_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("with_cover.m4a");
    write_sine_audio_with_cover(&src, &SineAudio::default(), &StillImage::default()).unwrap();
    let ffmpeg = FFmpegRunner::new().unwrap();

    // `.m4a` → `.mp4`: the mov muxer tags the cover from its own cover table.
    let out = dir.path().join("out.mp4");
    ffmpeg
        .remux(
            &src,
            &out,
            &RemuxOptions::for_container(ContainerFormat::Mp4, None),
            None,
        )
        .await
        .expect("remux to mp4");
    let info = ffmpeg.probe(&out).await.unwrap();
    assert!(info.has_audio && !info.has_video, "{info:?}");
    assert!(
        info.streams
            .iter()
            .any(|s| s.codec_type == StreamKind::AttachedPicture),
        "the cover must arrive with its ATTACHED_PIC disposition, not as a video track: {info:?}"
    );
}

#[tokio::test]
async fn remux_drops_cover_art_where_the_target_declares_no_video_codec() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("with_cover.m4a");
    write_sine_audio_with_cover(&src, &SineAudio::default(), &StillImage::default()).unwrap();
    let ffmpeg = FFmpegRunner::new().unwrap();

    // wav declares no video codec: it drops the cover rather than failing.
    let out = dir.path().join("out.wav");
    ffmpeg
        .remux(
            &src,
            &out,
            &RemuxOptions::for_container(ContainerFormat::Wav, None),
            None,
        )
        .await
        .expect("a cover must not make the wav remux fail");
    let info = ffmpeg.probe(&out).await.unwrap();
    assert!(info.has_audio && !info.has_video, "{info:?}");
    assert!(
        !info.has_attached_picture(),
        "wav cannot carry a cover stream; it is dropped: {info:?}"
    );

    // ogg declares Theora but `ogg_init` refuses every codec outside its own
    // list, mjpeg included. A FLAC source, since Ogg carries FLAC audio.
    let flac = dir.path().join("with_cover.flac");
    write_sine_audio_with_cover(&flac, &SineAudio::default(), &StillImage::default()).unwrap();
    let out = dir.path().join("out.ogg");
    ffmpeg
        .remux(
            &flac,
            &out,
            &RemuxOptions::for_container(ContainerFormat::Ogg, None),
            None,
        )
        .await
        .expect("a cover must not make the ogg remux fail");
    let info = ffmpeg.probe(&out).await.unwrap();
    assert!(info.has_audio && !info.has_video, "{info:?}");
    assert!(
        !info.has_attached_picture(),
        "ogg cannot carry a cover stream; it is dropped: {info:?}"
    );
}
