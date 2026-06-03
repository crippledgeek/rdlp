//! Regression test: MKV thumbnail embed emits "Timestamps are unset" warning
//! when the source MKV has streams with `AV_NOPTS_VALUE` DTS (e.g. B-frame
//! content muxed through a `setts=dts=NOPTS` bitstream filter).
//!
//! This test documents the bug **before** the fix is applied (failing-first
//! discipline).  It asserts that the warning IS present today so that the
//! fix task can prove the warning disappears.
//!
//! # Self-skip contract
//!
//! If the system `ffmpeg` binary is absent or any fixture-generation command
//! fails, the test prints a diagnostic and returns early (self-skip).  It does
//! NOT fail — the absence of the binary is an environment limitation, not a
//! code defect.

// expect/unwrap intentional in test code — panics surface failures clearly.
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// process::Command is used only to build fixtures via the system ffmpeg binary.
// This is acceptable in integration tests (not in library code).
#![allow(clippy::disallowed_methods)]

use std::process::Command;
use std::sync::{Arc, Mutex};

use rdlp_ffmpeg::FFmpegRunner;
use rdlp_types::Progress;

// ── Fixture helpers ────────────────────────────────────────────────────────

/// Returns `false` and prints a skip message if `ffmpeg` is not on PATH.
fn ffmpeg_available() -> bool {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(out) if out.status.success() => true,
        _ => {
            eprintln!(
                "[SKIP] system `ffmpeg` binary not found or not executable; \
                 skipping mkv_dts_synthesis test"
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

/// Build the fixture MKV files in `dir`:
///
/// 1. `src.mkv`       — normal H.264 with B-frames (so DTS ≠ PTS for some packets)
/// 2. `unset_dts.mkv` — same content, but every packet's DTS is forced to
///    `AV_NOPTS_VALUE` via the `setts=dts=NOPTS` bitstream filter
///
/// Returns `Ok(paths)` or `Err(())` (self-skip).
fn build_unset_dts_mkv(
    dir: &std::path::Path,
) -> Result<(std::path::PathBuf, std::path::PathBuf), ()> {
    let src = dir.join("src.mkv");
    let unset = dir.join("unset_dts.mkv");

    // Step 1: generate a 1-second H.264 MKV with B-frames.
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
        "-bf",
        "3",
        "-pix_fmt",
        "yuv420p",
        src.to_str().unwrap(),
    ])?;

    // Step 2: re-mux, forcing every packet's DTS to NOPTS via the setts BSF.
    run_ffmpeg(&[
        "-y",
        "-loglevel",
        "error",
        "-i",
        src.to_str().unwrap(),
        "-c",
        "copy",
        "-bsf:v",
        "setts=dts=NOPTS",
        "-f",
        "matroska",
        unset.to_str().unwrap(),
    ])?;

    Ok((src, unset))
}

/// Generate a tiny real WebP cover image in `dir`.
///
/// Returns `Ok(path)` or `Err(())` (self-skip).
fn build_webp_cover(dir: &std::path::Path) -> Result<std::path::PathBuf, ()> {
    let cover = dir.join("cover.webp");
    run_ffmpeg(&[
        "-y",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=red:s=8x8:d=1",
        "-frames:v",
        "1",
        cover.to_str().unwrap(),
    ])?;
    Ok(cover)
}

// ── Callback helper ────────────────────────────────────────────────────────

/// Minimal [`PostProcessCallback`] that collects all `on_log` messages.
struct LogCollector {
    lines: Mutex<Vec<String>>,
}

impl LogCollector {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            lines: Mutex::new(Vec::new()),
        })
    }

    fn collected(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

impl rdlp_core::PostProcessCallback for LogCollector {
    fn on_progress(&self, _progress: Progress) {}
    fn on_log(&self, message: &str) {
        self.lines.lock().unwrap().push(message.to_string());
    }
}

// ── The test ───────────────────────────────────────────────────────────────

/// Reproduces the "Timestamps are unset" warning that our MKV raw-FFI
/// thumbnail-embed path emits when the source file contains packets whose
/// DTS is `AV_NOPTS_VALUE`.
///
/// This test is expected to **pass** (the warning IS present) until the
/// fix in the companion task synthesises DTS from PTS — at which point it
/// will fail and must be updated or removed as part of the fix PR.
#[tokio::test]
async fn repro_thumbnail_embed_emits_unset_timestamp_warning_today() {
    // ── environment guard ──────────────────────────────────────────────────
    if !ffmpeg_available() {
        return; // self-skip
    }

    // ── fixture setup ──────────────────────────────────────────────────────
    let dir = tempfile::tempdir().expect("failed to create temp dir");

    let (_, unset_dts_mkv) = match build_unset_dts_mkv(dir.path()) {
        Ok(p) => p,
        Err(()) => return, // self-skip — fixture generation failed
    };

    let cover = match build_webp_cover(dir.path()) {
        Ok(p) => p,
        Err(()) => return, // self-skip
    };

    let output = dir.path().join("output_with_thumbnail.mkv");

    // ── run embed_thumbnail with a log-collecting callback ─────────────────
    // Passing a callback activates LogCaptureGuard inside embed_thumbnail_sync.
    // After the FFmpeg work completes, forward_captured_logs drains the capture
    // buffer into cb.on_log().
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new failed");
    let collector = LogCollector::new();
    let cb: Arc<dyn rdlp_core::PostProcessCallback> = collector.clone();

    let result = runner
        .embed_thumbnail(&unset_dts_mkv, &cover, &output, "mkv", Some(cb), None)
        .await;

    // The embed itself should succeed even with NOPTS DTS — FFmpeg still muxes
    // the file; it just logs a warning.
    assert!(
        result.is_ok(),
        "embed_thumbnail should not fail on unset-DTS input; got: {result:?}"
    );

    // ── assert the bug is present ──────────────────────────────────────────
    let logs = collector.collected();
    eprintln!("Captured FFmpeg log lines ({}):", logs.len());
    for line in &logs {
        eprintln!("  {line}");
    }

    let has_unset_warning = logs
        .iter()
        .any(|l| l.to_ascii_lowercase().contains("timestamps are unset"));

    assert!(
        has_unset_warning,
        "Expected to capture a 'Timestamps are unset' warning from FFmpeg \
         while embedding a thumbnail into an MKV with NOPTS DTS, but no such \
         warning was found.\n\
         Captured lines: {logs:#?}\n\
         \n\
         If this assertion fails, either:\n\
         (a) the fix has already been applied (update/remove this test), or\n\
         (b) the fixture does not actually produce NOPTS DTS (check setts BSF)."
    );

    println!(
        "BUG REPRODUCED: 'Timestamps are unset' warning captured — \
         the fix task should make this assertion FAIL."
    );
}
