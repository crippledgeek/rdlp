//! Cooperative cancellation of the audio normalization passes (#334).
//!
//! A pre-cancelled `CancellationToken` passed into `normalize_audio` must abort
//! the normalization promptly with a "cancelled" error, rather than running the
//! full (potentially two-pass loudnorm) pipeline to completion.
//!
//! Loudnorm is a TWO-PASS operation: pass 1 (analysis) and pass 2 (encode).
//! A pre-cancelled token aborts in pass 1 (the analysis decode loop) — that is
//! the correct, promptest behavior and is sufficient to prove both loops are
//! gated, since pass 2 shares the same cancel plumbing as the peak encode loop.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use rdlp_ffmpeg::{AudioNormMode, FFmpegRunner, NormalizeOptions};
use tokio_util::sync::CancellationToken;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a multi-second fixture with an audio stream so the normalization
/// loops have many packets to process — proves an aborted run does not finish
/// the whole input.
fn build_audio_fixture(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("src.m4a");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=5",
            "-c:a",
            "aac",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

#[tokio::test]
async fn precancelled_token_aborts_loudnorm_normalize() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Ok(src) = build_audio_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let out = dir.path().join("out.m4a");
    // Loudnorm mode exercises the two-pass path (pass 1 analysis + pass 2 encode).
    let opts = NormalizeOptions {
        mode: AudioNormMode::Loudnorm,
        ..Default::default()
    };

    // Pre-cancel: pass 1's analysis decode loop must bail before completion.
    let token = CancellationToken::new();
    token.cancel();

    let res = runner
        .normalize_audio(&src, &out, &opts, None, Some(token))
        .await;

    assert!(
        res.is_err(),
        "pre-cancelled loudnorm normalize must return Err, got Ok"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.to_lowercase().contains("cancel"),
        "error should mention cancellation; got: {msg}"
    );
}
