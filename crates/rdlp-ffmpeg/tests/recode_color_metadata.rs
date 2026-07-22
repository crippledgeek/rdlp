//! Regression test: the video RECODE path (`FFmpegRunner::convert_video` with
//! `remux_only: false`) MUST propagate the source's color/signal metadata
//! (color_range / primaries / transfer / matrix) from the decoder to the
//! encoder. Without the propagation the encoder writes its unspecified
//! defaults, which produces washed-out levels on full-range sources and the
//! wrong matrix on SD / BT.601 content.
//!
//! Stream-copy paths are fine (`avcodec_parameters_copy` carries these fields);
//! only the re-encode path drops them. This test recodes a tagged fixture and
//! asserts the OUTPUT still carries the tags.
//!
//! # Self-skip contract
//!
//! If the system `ffmpeg` / `ffprobe` binaries are absent or any
//! fixture-generation step fails, the test prints a diagnostic and returns
//! early (self-skip). It does NOT fail — the absence of the binary is an
//! environment limitation, not a code defect.

// expect/unwrap intentional in test code — panics surface failures clearly.
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// process::Command is used only to build fixtures via the system ffmpeg binary.
// This is acceptable in integration tests (not in library code).
#![allow(clippy::disallowed_methods)]

use std::process::Command;

use rdlp_ffmpeg::{FFmpegRunner, VideoConvertOptions};

// ── Fixture helpers ────────────────────────────────────────────────────────

/// Returns `false` and prints a skip message if `ffmpeg` is not on PATH.
fn ffmpeg_available() -> bool {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(out) if out.status.success() => true,
        _ => {
            eprintln!(
                "[SKIP] system `ffmpeg` binary not found or not executable; \
                 skipping recode_color_metadata test"
            );
            false
        }
    }
}

/// Run a `ffmpeg` command, returning `Ok(())` on success or printing a skip
/// message and returning `Err(())` on failure.
fn run_ffmpeg(args: &[&str]) -> Result<(), ()> {
    let status = Command::new("ffmpeg").args(args).status().map_err(|e| {
        eprintln!("[SKIP] failed to spawn ffmpeg: {e}");
    })?;
    if status.success() {
        Ok(())
    } else {
        eprintln!("[SKIP] ffmpeg command failed: ffmpeg {}", args.join(" "));
        Err(())
    }
}

/// Build `tagged.mp4` — an H.264 MP4 tagged with full-range (`pc`) color range
/// and a BT.601-625 (`bt470bg`) matrix, deliberately non-default values that
/// the encoder would otherwise leave unspecified.
fn build_tagged_mp4(dir: &std::path::Path) -> Result<std::path::PathBuf, ()> {
    let tagged = dir.join("tagged.mp4");
    run_ffmpeg(&[
        "-y",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc=d=1:s=320x240",
        "-c:v",
        "libx264",
        "-color_range",
        "pc",
        "-colorspace",
        "bt470bg",
        "-pix_fmt",
        "yuv420p",
        tagged.to_str().unwrap(),
    ])?;
    Ok(tagged)
}

/// Probe `color_range` and `color_space` from the first video stream of `file`.
/// Returns the raw ffprobe stdout, or `Err(())` (self-skip) on failure.
fn probe_color(file: &std::path::Path) -> Result<String, ()> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=color_range,color_space",
            "-of",
            "default=noprint_wrappers=1",
            file.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| {
            eprintln!("[SKIP] failed to spawn ffprobe: {e}");
        })?;
    if !out.status.success() {
        eprintln!("[SKIP] ffprobe failed on {}", file.display());
        return Err(());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// ── The test ───────────────────────────────────────────────────────────────

/// Recodes a tagged fixture through the public `convert_video` API and asserts
/// the OUTPUT preserves the source's `color_range=pc` and `color_space=bt470bg`
/// tags. Pre-fix, the encoder writes its unspecified defaults and these tags
/// are lost.
#[tokio::test]
async fn recode_preserves_color_range_and_matrix() {
    if !ffmpeg_available() {
        return; // self-skip
    }

    let dir = tempfile::tempdir().expect("failed to create temp dir");

    let tagged = match build_tagged_mp4(dir.path()) {
        Ok(p) => p,
        Err(()) => return, // self-skip — fixture generation failed
    };

    // Sanity: the fixture itself must carry the tags, else the test is vacuous.
    let src_probe = match probe_color(&tagged) {
        Ok(s) => s,
        Err(()) => return, // self-skip
    };
    eprintln!("source tags:\n{src_probe}");
    if !src_probe.contains("color_range=pc") || !src_probe.contains("color_space=bt470bg") {
        eprintln!(
            "[SKIP] fixture did not carry the expected source tags \
             (ffmpeg build may not honor -color_range/-colorspace); got:\n{src_probe}"
        );
        return;
    }

    let out_mp4 = dir.path().join("recoded.mp4");

    let opts = VideoConvertOptions {
        remux_only: false,
        video_codec: Some(rdlp_types::media_name::VideoEncoderName::from_static(
            "libx264",
        )),
        crf: Some(30), // fast, low-quality — we only care about the tags
        ..Default::default()
    };

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new failed");
    let result = runner
        .convert_video(&tagged, &out_mp4, &opts, None, None, None)
        .await;

    assert!(
        result.is_ok(),
        "convert_video (recode) should succeed; got: {result:?}"
    );

    let out_probe = match probe_color(&out_mp4) {
        Ok(s) => s,
        Err(()) => return, // self-skip — ffprobe unavailable
    };
    eprintln!("recoded output tags:\n{out_probe}");

    assert!(
        out_probe.contains("color_range=pc"),
        "recoded output lost color_range=pc — the decoder->encoder color \
         metadata propagation is missing. ffprobe reported:\n{out_probe}"
    );
    assert!(
        out_probe.contains("color_space=bt470bg"),
        "recoded output lost color_space=bt470bg — the decoder->encoder color \
         metadata propagation is missing. ffprobe reported:\n{out_probe}"
    );

    println!(
        "FIX VERIFIED: recode preserved color_range=pc and color_space=bt470bg \
         from decoder -> encoder."
    );
}
