//! Loudnorm's pass-1 measurements are read from `FFmpeg`'s INFO-level log
//! output through the install-once log callback (#781).
//!
//! `ensure_init` sets the operator's global `FFmpeg` log level to ERROR. The
//! capture path used to raise that global to INFO for the duration of the
//! analysis and restore it afterwards — a save/restore on a process-global
//! that raced with every other guard. It now leaves the level alone, relying
//! on `av_vlog` handing the callback every line unfiltered
//! (libavutil/log.c: only `av_log_default_callback` applies the level). This
//! test pins that a full two-pass loudnorm normalization succeeds at the
//! operator's ERROR level — i.e. the INFO block was captured without touching
//! the global.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rdlp_ffmpeg::test_support::{SineAudio, write_sine_audio};
use rdlp_ffmpeg::{AudioNormMode, FFmpegRunner, NormalizeOptions};

#[tokio::test]
async fn loudnorm_two_pass_succeeds_at_the_operator_error_log_level() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src.m4a");
    write_sine_audio(&src, &SineAudio::default()).expect("synthesise fixture");

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    // The operator level after init is ERROR; pass 1 must still see loudnorm's
    // INFO block, or it fails with "missing 'input_i' in loudnorm output".
    let level_before = ffmpeg_the_third::log::get_level().expect("named level");
    assert_eq!(level_before, ffmpeg_the_third::log::Level::Error);

    let out = dir.path().join("out.m4a");
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        ..Default::default()
    };
    let res = runner.normalize_audio(&src, &out, &opts, None, None).await;

    assert!(
        res.is_ok(),
        "loudnorm normalize failed: {:#}",
        res.unwrap_err()
    );
    let written = tokio::fs::metadata(&out).await.expect("output exists");
    assert!(written.len() > 0, "output is empty");
    assert_eq!(
        ffmpeg_the_third::log::get_level().expect("named level"),
        level_before,
        "normalization must not change the operator's global log level"
    );
}
