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
//! - `flac` differs on both (`s16`/`s32`, and 4608 samples — `flacenc` sets
//!   `frame_size` from its default `max_blocksize`, so FLAC is fixed-frame-size,
//!   not variable as is sometimes assumed).
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixture and to verify the muxed result).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{decoded_audio_frames, encoder_available, ffmpeg_available, probe_audio_field};
use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner};

/// Stereo AAC source at `rate` Hz: `fltp`, 1024-sample frames.
fn build_aac_fixture(dir: &Path, rate: u32) -> Result<PathBuf, ()> {
    let src = dir.join(format!("src_{rate}.m4a"));
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration=3:sample_rate={rate}"),
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

/// The source's sample rate, in Hz.
///
/// 48 kHz matches what every encoder here accepts, so it isolates the frame
/// size / sample format axes. 44.1 kHz is the rate `libopus` cannot take at
/// all, which is what exercises the resampling half of the `aformat` spec.
const RATE_48K: u32 = 48_000;
const RATE_44K1: u32 = 44_100;

/// One extraction case. A named-field struct rather than a positional
/// argument list so the three same-typed `&str` fields cannot be swapped.
struct Extraction {
    /// FFmpeg encoder name to extract with.
    encoder: &'static str,
    /// Output file extension (selects the muxer).
    ext: &'static str,
    /// Codec name the output must decode as.
    expected_codec: &'static str,
    /// Sample rate of the generated source.
    source_rate: u32,
}

/// Extract per `case` and assert the result is a real, decodable file.
///
/// Returns the output's sample rate so rate-conversion cases can assert on it.
async fn assert_extracts(case: Extraction) -> Option<u32> {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return None;
    }
    if !encoder_available(case.encoder) {
        eprintln!("[SKIP] encoder {} not in this FFmpeg build", case.encoder);
        return None;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Ok(src) = build_aac_fixture(dir.path(), case.source_rate) else {
        eprintln!("[SKIP] fixture build failed");
        return None;
    };
    let out = dir.path().join(format!("out.{}", case.ext));

    let runner = FFmpegRunner::new().expect("FFmpegRunner");
    let opts = AudioExtractOptions {
        encoder_name: Some(case.encoder.to_string()),
        copy: false,
        bitrate_kbps: Some(128),
        quality_scale: None,
    };

    runner
        .extract_audio(&src, &out, &opts, None, None)
        .await
        .unwrap_or_else(|e| panic!("extract_audio to {} failed: {e}", case.encoder));

    let codec = probe_audio_field(&out, "codec_name")
        .unwrap_or_else(|| panic!("output of {} extract is not probeable", case.encoder));
    assert_eq!(
        codec, case.expected_codec,
        "{} produced the wrong codec in the output",
        case.encoder
    );

    // Decode for real: a structurally valid but undecodable file must fail.
    let frames = decoded_audio_frames(&out)
        .unwrap_or_else(|| panic!("output of {} extract is not decodable", case.encoder));
    assert!(
        frames > 0,
        "{} output decoded to zero frames — the mux produced an empty stream",
        case.encoder
    );

    probe_audio_field(&out, "sample_rate").and_then(|r| r.parse().ok())
}

/// Frame-size half: `libmp3lame` accepts the decoder's `fltp` but requires
/// 1152-sample frames against AAC's 1024. Fails before #638.
#[tokio::test]
async fn extract_audio_adapts_frame_size_for_mp3() {
    assert_extracts(Extraction {
        encoder: "libmp3lame",
        ext: "mp3",
        expected_codec: "mp3",
        source_rate: RATE_48K,
    })
    .await;
}

/// Sample-format half: `libopus` accepts only packed `s16`/`flt` against the
/// decoder's planar `fltp`. Fails before #638.
#[tokio::test]
async fn extract_audio_adapts_sample_format_for_opus() {
    assert_extracts(Extraction {
        encoder: "libopus",
        ext: "opus",
        expected_codec: "opus",
        source_rate: RATE_48K,
    })
    .await;
}

/// Both halves at once: `flac` differs on sample format (`s16`/`s32`) and on
/// frame size (4608). Fails before #638.
#[tokio::test]
async fn extract_audio_adapts_both_for_flac() {
    assert_extracts(Extraction {
        encoder: "flac",
        ext: "flac",
        expected_codec: "flac",
        source_rate: RATE_48K,
    })
    .await;
}

/// Control: `aac` matched on both axes and worked before #638. It must keep
/// working — this is the regression guard on the adaptation not breaking the
/// one path that was already correct.
#[tokio::test]
async fn extract_audio_still_works_for_aac() {
    assert_extracts(Extraction {
        encoder: "aac",
        ext: "m4a",
        expected_codec: "aac",
        source_rate: RATE_48K,
    })
    .await;
}

/// Resampling: `libopus` accepts **only** 48 kHz, so a 44.1 kHz source must be
/// resampled on the way through.
///
/// The recode builder this change deleted appended a bare `,aresample` to its
/// filter spec; the shared builder relies on `aformat`'s own format
/// negotiation to insert the conversion. This pins that the rate actually
/// converts, so dropping the explicit stage cannot silently regress into a
/// wrong-rate or failed encode.
#[tokio::test]
async fn extract_audio_resamples_when_encoder_rejects_source_rate() {
    let Some(rate) = assert_extracts(Extraction {
        encoder: "libopus",
        ext: "opus",
        expected_codec: "opus",
        source_rate: RATE_44K1,
    })
    .await
    else {
        return; // skipped — no ffmpeg or no libopus in this build
    };
    assert_eq!(
        rate, RATE_48K,
        "libopus output must be resampled to 48 kHz from a {RATE_44K1} Hz source"
    );
}
