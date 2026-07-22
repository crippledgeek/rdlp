//! `--extract-audio` must produce a decodable file for EVERY `AudioFormat`
//! the linked FFmpeg build can encode (#638).
//!
//! This drives `AudioExtractStage` — the stage the CLI actually runs — rather
//! than `FFmpegRunner::extract_audio` directly, so encoder selection, quality
//! handling, and the temp-path/tracker plumbing are all in the covered path.
//!
//! Before #638 only `aac`/`m4a` succeeded: the extract route never set the
//! buffersink frame size, so every encoder whose `frame_size` differed from
//! the AAC decoder's 1024 samples rejected the frame with `EINVAL`.
//!
//! Iteration is over `AudioFormat::iter()`, not a hand-written list, so a
//! newly added variant is covered automatically instead of silently skipped.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod common;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use strum::IntoEnumIterator as _;

use common::{FIXTURE_FAILED, build_av_fixture, ffmpeg_cli_available, make_msg, opts};
use rdlp_postprocess::PostProcess;
use rdlp_postprocess::pipeline::PipelineStage as _;
use rdlp_postprocess::pipeline::stages::AudioExtractStage;
use rdlp_types::AudioFormat;

/// Formats whose encoder FFmpeg marks experimental, so `avcodec_open2`
/// refuses it at the default compliance level regardless of frame adaptation.
///
/// This is a different root cause from #638 — the open fails before a single
/// frame is sent — and is tracked separately as #639. Re-enable here when that
/// lands; the assertion below still guarantees the rest of the matrix passes.
const EXPERIMENTAL_ENCODER_GATED: &[AudioFormat] = &[AudioFormat::Dts];

/// Decode the file for real and return its audio codec name.
///
/// Deliberately not a size or existence check: a failed mux still leaves a
/// partial header on disk, so `len() > 0` counts a failure as a success — the
/// verification trap that produced a wrong answer while diagnosing #637/#638.
fn decoded_audio_codec(path: &Path) -> Option<String> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-count_frames",
            "-show_entries",
            "stream=codec_name,nb_read_frames",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !probe.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&probe.stdout);
    let mut lines = text.lines();
    let codec = lines.next()?.trim().to_string();
    let frames: u64 = lines.next()?.trim().parse().ok()?;
    if codec.is_empty() || frames == 0 {
        return None;
    }
    Some(codec)
}

/// Whether the linked build can encode `format`, so an absent encoder skips
/// rather than fails. `mediaforge` and a distro build differ here.
///
/// Resolves the encoder exactly the way `AudioExtractStage` does — through
/// the same `get_audio_codec` row — so the skip list can never diverge from
/// what the stage would actually attempt.
fn encoder_present(format: AudioFormat) -> bool {
    let Some(cfg) = rdlp_ffmpeg::ffmpeg::get_audio_codec(format.codec_name()) else {
        return false;
    };
    match cfg.encoder {
        // A named encoder must actually exist in this build.
        Some(enc) => rdlp_ffmpeg::ffmpeg::audio_encoder_registry::is_audio_encoder_available(enc),
        // `None` is not "unsupported" — it means the row defers to the output
        // muxer's default codec (the `wav`/PCM row). That is a real, reachable
        // branch of `extract_audio_transcode_sync` and must be exercised, not
        // skipped.
        None => true,
    }
}

#[tokio::test]
async fn every_supported_audio_format_extracts_to_a_decodable_file() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("video.mp4");
    build_av_fixture(&src).expect(FIXTURE_FAILED);

    let mut covered = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    for format in AudioFormat::iter() {
        if EXPERIMENTAL_ENCODER_GATED.contains(&format) {
            skipped.push(format!("{format} (#639)"));
            continue;
        }
        if !encoder_present(format) {
            skipped.push(format.to_string());
            continue;
        }

        // Each format gets its own copy of the source: the stage replaces the
        // tracker's files with the extracted output, so a shared input would
        // make every iteration after the first operate on the previous result.
        let input = dir.path().join(format!("in_{format}.mp4"));
        std::fs::copy(&src, &input).expect("copy fixture");

        let config = PostProcess {
            extract_audio: true,
            audio_format: Some(format),
            ..PostProcess::default()
        };
        let msg = make_msg(vec![input], config, opts("video", false));

        let stage = AudioExtractStage::new(Arc::new(
            rdlp_ffmpeg::FFmpegRunner::new().expect("FFmpegRunner"),
        ));
        match stage.process(msg).await {
            Ok(out) => {
                let produced = out.tracker.primary();
                match decoded_audio_codec(&produced) {
                    Some(codec) => covered.push(format!("{format}→{codec}")),
                    None => failures.push(format!(
                        "{format}: stage reported success but {} is not decodable",
                        produced.display()
                    )),
                }
            }
            Err(e) => failures.push(format!("{format}: {e:#}")),
        }
    }

    eprintln!("extracted: {}", covered.join(", "));
    if !skipped.is_empty() {
        eprintln!(
            "skipped (no encoder in this build, or tracked separately): {}",
            skipped.join(", ")
        );
    }

    assert!(
        failures.is_empty(),
        "audio extraction failed for {} format(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    assert!(
        covered.len() > 1,
        "only {} format(s) were exercised — the build supports too few \
         encoders for this matrix to prove anything",
        covered.len()
    );
}
