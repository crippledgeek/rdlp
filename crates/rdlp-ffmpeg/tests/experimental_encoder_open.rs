//! #639: an encoder `FFmpeg` marks `AV_CODEC_CAP_EXPERIMENTAL` (the native
//! `dca` for DTS) is refused by `avcodec_open2` unless the context's
//! `strict_std_compliance` is `FF_COMPLIANCE_EXPERIMENTAL`
//! (`libavcodec/avcodec.c`: "The encoder 'dca' is experimental but
//! experimental codecs are not enabled, add '-strict -2'"). rdlp never set
//! it, so `--audio-format=dts` could not succeed on any build using the
//! native encoder. The open path now enables it for — and only for — an
//! encoder that carries the flag.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rdlp_ffmpeg::test_support::{SineAudio, write_sine_audio};
use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner};
use rdlp_types::media_name::AudioEncoderName;

#[tokio::test]
async fn an_explicitly_requested_experimental_encoder_opens_and_encodes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src.m4a");
    write_sine_audio(&src, &SineAudio::default()).expect("fixture");

    let runner = FFmpegRunner::new().expect("runner");
    let out = dir.path().join("out.dts");
    let opts = AudioExtractOptions {
        encoder_name: Some(AudioEncoderName::from_static("dca")),
        ..AudioExtractOptions::default()
    };
    let res = runner.extract_audio(&src, &out, &opts, None, None).await;
    assert!(res.is_ok(), "dca extract failed: {:#}", res.unwrap_err());

    let info = runner.probe(&out).await.expect("probe");
    assert_eq!(
        info.audio_codec.as_ref().map(|c| c.as_str()),
        Some("dts"),
        "{info:?}"
    );
}
