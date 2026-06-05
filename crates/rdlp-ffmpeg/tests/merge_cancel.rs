//! Cooperative cancellation of the video+audio merge loops (#334).
//!
//! A pre-cancelled `CancellationToken` passed into `merge` must abort the mux
//! promptly with a "cancelled" error, rather than running the full two-way
//! interleaved packet loop to completion.
//!
//! There are TWO genuine packet loops with different teardown shapes:
//!
//! - `.mkv` output → `merge_mkv_raw_ffi` (RAII `AvPacketOwned`, `?`-safe).
//! - non-`.mkv` output → the bare `av_packet_alloc` loop in `merge_sync`
//!   (manual cleanup; gated via `break` + post-cleanup `bail!`).
//!
//! Both paths are exercised below (`.mkv` and `.mp4`).
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixtures).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};
use tokio_util::sync::CancellationToken;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a multi-second video-only fixture (lavfi testsrc → .mp4) so the merge
/// loop has many packets to process — proves an aborted run does not finish.
fn build_video_fixture(dir: &Path) -> Result<PathBuf, ()> {
    let src = dir.join("video.mp4");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=30:duration=5",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-an",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

/// Build a multi-second audio-only fixture (lavfi sine → .m4a).
fn build_audio_fixture(dir: &Path) -> Result<PathBuf, ()> {
    let src = dir.join("audio.m4a");
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
            "-vn",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

/// Shared assertion: pre-cancelled merge into `output` must return Err mentioning cancellation.
async fn assert_precancelled_merge_aborts(output_name: &str) {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let (Ok(video), Ok(audio)) = (
        build_video_fixture(dir.path()),
        build_audio_fixture(dir.path()),
    ) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    let out = dir.path().join(output_name);

    let token = CancellationToken::new();
    token.cancel();

    let res = runner
        .merge(
            &video,
            &audio,
            &out,
            &RemuxOptions::default(),
            None,
            Some(token),
        )
        .await;

    assert!(
        res.is_err(),
        "pre-cancelled merge into {output_name} must return Err, got Ok"
    );
    let msg = format!("{:#}", res.unwrap_err());
    assert!(
        msg.to_lowercase().contains("cancel"),
        "error should mention cancellation; got: {msg}"
    );
}

/// `.mkv` output exercises `merge_mkv_raw_ffi` (RAII IIFE loop).
#[tokio::test]
async fn precancelled_token_aborts_mkv_merge() {
    assert_precancelled_merge_aborts("out.mkv").await;
}

/// `.mp4` output exercises the bare-pointer loop in `merge_sync`.
#[tokio::test]
async fn precancelled_token_aborts_mp4_merge() {
    assert_precancelled_merge_aborts("out.mp4").await;
}
