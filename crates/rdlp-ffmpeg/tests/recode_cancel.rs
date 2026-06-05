//! Cooperative cancellation of the video RECODE encode loop (#334).
//!
//! A pre-cancelled `CancellationToken` passed into `convert_video` must abort
//! the transcode promptly with a "cancelled" error, rather than running the
//! full encode loop to completion.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use rdlp_ffmpeg::{FFmpegRunner, VideoConvertOptions};
use tokio_util::sync::CancellationToken;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a multi-second H.264 yuv420p fixture so the encode loop has many
/// packets to process — proves an aborted run does not finish the whole input.
fn build_h264_fixture(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("src.mp4");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=5:s=640x480",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

#[tokio::test]
async fn precancelled_token_aborts_recode() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Ok(src) = build_h264_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let out = dir.path().join("out.mp4");
    let opts = VideoConvertOptions {
        remux_only: false,
        video_codec: Some("libx264".to_string()),
        audio_copy: false,
        ..Default::default()
    };

    // Pre-cancel: the encode loop must bail before processing the whole input.
    let token = CancellationToken::new();
    token.cancel();

    let res = runner
        .convert_video(&src, &out, &opts, None, None, Some(token))
        .await;

    assert!(res.is_err(), "pre-cancelled recode must return Err, got Ok");
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.to_lowercase().contains("cancel"),
        "error should mention cancellation; got: {msg}"
    );
}
