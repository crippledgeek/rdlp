//! `extract_audio` must adapt decoded frames to the encoder's requirements (#638).
//!
//! The AAC decoder emits `fltp` in 1024-sample frames. An encoder whose
//! accepted sample formats or fixed `frame_size` differ rejects those frames
//! with `avcodec_send_frame: Invalid argument`. Before #638 only `aac`
//! survived, because it is the one target where *both* already match.
//!
//! The two failure modes are deliberately covered by different encoders so a
//! regression in either half is attributable:
//!
//! - `libmp3lame` accepts `fltp`, so it isolates the **frame-size** half
//!   (1024 → 1152).
//! - `libopus` accepts only packed `s16`/`flt`, so it isolates the
//!   **sample-format** half (planar → packed).
//! - `flac` differs on both (`s16`/`s32`, 4096 samples).
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture and to verify the muxed result).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True when the linked `FFmpeg` build actually provides this encoder.
///
/// The custom `mediaforge` build and a distro build differ in codec coverage,
/// so a missing encoder must skip rather than fail.
fn encoder_available(name: &str) -> bool {
    ffmpeg_the_third::encoder::find_by_name(name).is_some()
}

/// Stereo 48 kHz AAC source: `fltp`, 1024-sample frames.
fn build_aac_fixture(dir: &Path) -> Result<PathBuf, ()> {
    let src = dir.join("src.m4a");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3:sample_rate=48000",
            "-ac",
            "2",
            "-c:a",
            "aac",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

/// Decode the produced file end-to-end and return `(codec_name, frame_count)`.
///
/// Decoding rather than stat-ing the file is deliberate: a failed mux still
/// leaves a non-empty partial header on disk, so `[ -s file ]` and a size
/// assertion both count a failure as success (#637/#638 verification trap).
fn decode_probe(path: &Path) -> Option<(String, u64)> {
    let codec = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !codec.status.success() {
        return None;
    }
    let codec_name = String::from_utf8_lossy(&codec.stdout).trim().to_string();
    if codec_name.is_empty() {
        return None;
    }

    // `-c copy -f null` would not decode; decode for real so a structurally
    // valid but undecodable file fails here.
    let frames = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !frames.status.success() {
        return None;
    }
    let n = String::from_utf8_lossy(&frames.stdout)
        .trim()
        .parse::<u64>()
        .ok()?;
    Some((codec_name, n))
}

/// Extract to `encoder` and assert the result is a real, decodable file.
async fn assert_extracts(encoder: &str, ext: &str, expected_codec: &str) {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    if !encoder_available(encoder) {
        eprintln!("[SKIP] encoder {encoder} not in this FFmpeg build");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Ok(src) = build_aac_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    let out = dir.path().join(format!("out.{ext}"));

    let runner = FFmpegRunner::new().expect("FFmpegRunner");
    let opts = AudioExtractOptions {
        encoder_name: Some(encoder.to_string()),
        copy: false,
        bitrate_kbps: Some(128),
        quality_scale: None,
    };

    runner
        .extract_audio(&src, &out, &opts, None, None)
        .await
        .unwrap_or_else(|e| panic!("extract_audio to {encoder} failed: {e}"));

    let (codec, frames) = decode_probe(&out)
        .unwrap_or_else(|| panic!("output of {encoder} extract is not decodable"));
    assert_eq!(
        codec, expected_codec,
        "{encoder} produced the wrong codec in the output"
    );
    assert!(
        frames > 0,
        "{encoder} output decoded to zero frames — the mux produced an empty stream"
    );
}

/// Frame-size half: `libmp3lame` accepts the decoder's `fltp` but requires
/// 1152-sample frames against AAC's 1024. Fails before #638.
#[tokio::test]
async fn extract_audio_adapts_frame_size_for_mp3() {
    assert_extracts("libmp3lame", "mp3", "mp3").await;
}

/// Sample-format half: `libopus` accepts only packed `s16`/`flt` against the
/// decoder's planar `fltp`. Fails before #638.
#[tokio::test]
async fn extract_audio_adapts_sample_format_for_opus() {
    assert_extracts("libopus", "opus", "opus").await;
}

/// Both halves at once: `flac` differs on sample format (`s16`/`s32`) and on
/// frame size (4096). Fails before #638.
#[tokio::test]
async fn extract_audio_adapts_both_for_flac() {
    assert_extracts("flac", "flac", "flac").await;
}

/// Control: `aac` matched on both axes and worked before #638. It must keep
/// working — this is the regression guard on the adaptation not breaking the
/// one path that was already correct.
#[tokio::test]
async fn extract_audio_still_works_for_aac() {
    assert_extracts("aac", "m4a", "aac").await;
}
