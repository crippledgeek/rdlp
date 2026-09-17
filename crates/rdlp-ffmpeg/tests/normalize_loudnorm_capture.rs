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
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use rdlp_ffmpeg::{AudioNormMode, FFmpegRunner, NormalizeOptions};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn build_audio_fixture(dir: &Path) -> Option<std::path::PathBuf> {
    let src = dir.join("src.m4a");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:a",
            "aac",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then_some(src)
}

#[tokio::test]
async fn loudnorm_two_pass_succeeds_at_the_operator_error_log_level() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(src) = build_audio_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };

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
    assert!(out.exists() && std::fs::metadata(&out).unwrap().len() > 0);
    assert_eq!(
        ffmpeg_the_third::log::get_level().expect("named level"),
        level_before,
        "normalization must not change the operator's global log level"
    );
}
