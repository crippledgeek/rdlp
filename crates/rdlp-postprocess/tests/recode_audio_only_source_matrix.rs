//! `--recode-video` on an audio-only source must not fail with
//! "No video stream found in input" (#637).
//!
//! `RecodeStage`'s transcode path requires a video stream
//! (`open_input_and_decoder` → `NoVideoStream`) and nothing gated the stage on
//! `has_video`, so every container whose `can_remux` arm answered `false` for
//! an audio-only source routed into a path that structurally cannot serve it.
//! Measured on `develop` @ 22ad252d: 12 of 16 containers failed, 8 of them
//! **falsely** — a plain stream copy works for all 8.
//!
//! Ground truth for an audio-only AAC source, `ffmpeg -i a.m4a -c copy out.X`
//! (exit status, never file size — a failed webm still leaves 264 bytes):
//!
//! - copies cleanly: mp4 mov m4v ts flv mkv nut avi mka 3gp asf wmv
//! - refuses: webm ("Only VP8/VP9/AV1 video and Vorbis/Opus audio"),
//!   mpg ("Unsupported audio codec"), ogg ("Unsupported codec id"),
//!   mxf ("there must be exactly one video stream and it must be the first")
//!
//! webm/mpg/ogg cannot carry AAC but *can* carry audio, so they re-encode to
//! the container's own default. MXF cannot hold an audio-only file at all and
//! must refuse naming **that**, not a missing video stream.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod common;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use common::{FIXTURE_FAILED, ffmpeg_cli_available, make_msg, opts};
use rdlp_postprocess::PostProcess;
use rdlp_postprocess::pipeline::PipelineStage as _;
use rdlp_postprocess::pipeline::stages::RecodeStage;
use rdlp_types::ContainerFormat;

/// What the stage must do with an audio-only source for a given container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expected {
    /// Produces an audio-only file (by stream copy or by re-encoding — which
    /// one is an optimisation, not a contract, so the test does not pin it).
    Produces,
    /// The container genuinely cannot hold an audio-only file. The refusal
    /// must name that, not a missing video stream.
    RefusesNamingVideoStreamRequirement,
}

/// Every container `--recode-video` accepts, with what an audio-only source
/// must produce. Derived from the ffmpeg ground truth in the module docs.
const CASES: &[(ContainerFormat, Expected)] = &[
    // Carries AAC directly — a stream copy is valid.
    (ContainerFormat::Mp4, Expected::Produces),
    (ContainerFormat::Mov, Expected::Produces),
    (ContainerFormat::M4v, Expected::Produces),
    (ContainerFormat::Ts, Expected::Produces),
    (ContainerFormat::Flv, Expected::Produces),
    (ContainerFormat::Mkv, Expected::Produces),
    (ContainerFormat::Nut, Expected::Produces),
    (ContainerFormat::Avi, Expected::Produces),
    (ContainerFormat::Mka, Expected::Produces),
    (ContainerFormat::ThreeGp, Expected::Produces),
    (ContainerFormat::Asf, Expected::Produces),
    (ContainerFormat::Wmv, Expected::Produces),
    // Cannot carry AAC, but can carry audio — re-encode to its own default.
    (ContainerFormat::WebM, Expected::Produces),
    (ContainerFormat::Mpg, Expected::Produces),
    (ContainerFormat::Ogg, Expected::Produces),
    // Requires exactly one video stream; an audio-only file is impossible.
    (
        ContainerFormat::Mxf,
        Expected::RefusesNamingVideoStreamRequirement,
    ),
];

/// Audio-only AAC source, 44.1 kHz mono — the shape `--extract-audio` leaves
/// behind and the input #637 was reported against.
fn build_audio_only_fixture(path: &Path) -> Result<(), ()> {
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
            path.to_str().unwrap(),
        ])
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(()) }
}

/// Decode the output for real and return its audio codec.
///
/// Never a size check: a failed mux leaves a partial header on disk, so
/// `len() > 0` scores failure as success — the trap that produced a wrong
/// answer while diagnosing this issue.
fn decoded_audio_codec(path: &Path) -> Option<String> {
    let out = Command::new("ffprobe")
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
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let codec = lines.next()?.trim().to_string();
    let frames: u64 = lines.next()?.trim().parse().ok()?;
    if codec.is_empty() || frames == 0 {
        return None;
    }
    Some(codec)
}

/// The failure #637 is about. Any error mentioning it means the stage still
/// routed an audio-only source into the video transcode path.
fn names_missing_video_stream(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("no video stream")
}

#[tokio::test]
async fn recode_of_an_audio_only_source_never_reports_a_missing_video_stream() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("audio_only.m4a");
    build_audio_only_fixture(&src).expect(FIXTURE_FAILED);

    let mut produced = Vec::new();
    let mut failures = Vec::new();

    for &(target, expected) in CASES {
        let ext = target.as_ext();
        let input = dir.path().join(format!("in_{ext}.m4a"));
        std::fs::copy(&src, &input).expect("copy fixture");

        let config = PostProcess {
            recode_video: Some(target),
            ..PostProcess::default()
        };
        let msg = make_msg(vec![input], config, opts("audio_only", false));
        let stage = RecodeStage::new(Arc::new(
            rdlp_ffmpeg::FFmpegRunner::new().expect("FFmpegRunner"),
        ));

        match (stage.process(msg).await, expected) {
            (Ok(out), Expected::Produces) => {
                let path = out.tracker.primary();
                match decoded_audio_codec(&path) {
                    Some(codec) => produced.push(format!("{ext}→{codec}")),
                    None => failures.push(format!(
                        "{ext}: stage succeeded but {} is not decodable",
                        path.display()
                    )),
                }
            }
            (Err(e), Expected::Produces) => {
                let msg = format!("{e:#}");
                let note = if names_missing_video_stream(&msg) {
                    " [#637: routed into the video transcode path]"
                } else {
                    ""
                };
                failures.push(format!("{ext}: expected a file, got error: {msg}{note}"));
            }
            (Ok(_), Expected::RefusesNamingVideoStreamRequirement) => failures.push(format!(
                "{ext}: expected a refusal — this container cannot hold an audio-only file"
            )),
            (Err(e), Expected::RefusesNamingVideoStreamRequirement) => {
                let msg = format!("{e:#}");
                if names_missing_video_stream(&msg) {
                    failures.push(format!(
                        "{ext}: refused, but blamed a missing video stream instead of the \
                         container's own requirement: {msg}"
                    ));
                } else {
                    produced.push(format!("{ext}→refused truthfully"));
                }
            }
        }
    }

    eprintln!("results: {}", produced.join(", "));

    assert!(
        failures.is_empty(),
        "{} of {} containers behaved wrongly for an audio-only source:\n  {}",
        failures.len(),
        CASES.len(),
        failures.join("\n  ")
    );
}
