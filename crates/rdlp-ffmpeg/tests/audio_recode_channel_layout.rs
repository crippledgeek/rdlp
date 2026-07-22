//! Audio re-encode during a video recode must preserve the source channel
//! layout beyond mono/stereo (#638).
//!
//! The recode path used to derive its channel layout as
//! `if channels == 1 { "mono" } else { "stereo" }` and declare that on *both*
//! the `abuffer` source and the `aformat` filter. For any source with more
//! than 2 channels the source therefore claimed stereo while the decoder
//! delivered 5.1 frames, and the whole recode failed with
//! `Changing audio frame properties on the fly is not supported` — measured
//! against `develop` @ 22ad252d. The shared builder
//! (`build_encoder_adapted_audio_filter`) declares the decoder's real layout
//! on the source and the encoder's real `ch_layout` on `aformat`, so
//! multichannel audio survives a recode instead of aborting it.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture and verify the result).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{ffmpeg_available, probe_audio_field};
use rdlp_ffmpeg::{FFmpegRunner, VideoConvertOptions};

/// 5.1 AAC audio alongside a tiny H.264 video, so the recode path takes its
/// audio-transcode branch.
fn build_surround_fixture(dir: &Path) -> Result<PathBuf, ()> {
    let src = dir.join("src.mkv");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=160x120:r=15",
            "-f",
            "lavfi",
            "-i",
            "sine=d=1:r=48000",
            "-ac",
            "6",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

/// Channel count of the first audio stream.
fn probe_channels(path: &Path) -> Option<u32> {
    probe_audio_field(path, "channels").and_then(|c| c.parse().ok())
}

#[tokio::test]
async fn recode_audio_reencode_preserves_surround_layout() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Ok(src) = build_surround_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    assert_eq!(
        probe_channels(&src),
        Some(6),
        "fixture is not 5.1 — the test would not discriminate"
    );

    let out = dir.path().join("out.mkv");
    let opts = VideoConvertOptions {
        video_codec: Some("libx264".into()),
        audio_codec: Some("aac".into()),
        ..Default::default()
    };

    FFmpegRunner::new()
        .expect("FFmpegRunner")
        .convert_video(&src, &out, &opts, None, None, None)
        .await
        .expect("recode with audio re-encode failed");

    assert_eq!(
        probe_channels(&out),
        Some(6),
        "recode did not preserve 5.1 — the filter graph's channel layout is \
         not following the decoder/encoder ch_layout"
    );
}
