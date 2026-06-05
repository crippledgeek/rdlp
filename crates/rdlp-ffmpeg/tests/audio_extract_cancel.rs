//! Cooperative cancellation of the audio EXTRACT transcode loop (#334).
//!
//! A pre-cancelled `CancellationToken` passed into `extract_audio` must abort
//! the transcode promptly with a "cancelled" error, rather than running the
//! full transcode loop to completion.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner};
use tokio_util::sync::CancellationToken;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a multi-second AAC fixture with an audio stream so the transcode loop
/// has many packets to process — proves an aborted run does not finish the
/// whole input.
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
async fn precancelled_token_aborts_audio_extract() {
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

    let out = dir.path().join("out.mp3");
    // copy=false forces the transcode path (the :421 packet loop).
    let opts = AudioExtractOptions {
        encoder_name: Some("libmp3lame".to_string()),
        copy: false,
        ..Default::default()
    };

    // Pre-cancel: the transcode loop must bail before processing the whole input.
    let token = CancellationToken::new();
    token.cancel();

    let res = runner
        .extract_audio(&src, &out, &opts, None, Some(token))
        .await;

    assert!(
        res.is_err(),
        "pre-cancelled audio extract must return Err, got Ok"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.to_lowercase().contains("cancel"),
        "error should mention cancellation; got: {msg}"
    );
}
