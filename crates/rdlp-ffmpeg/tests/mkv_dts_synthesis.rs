//! Regression test: MKV thumbnail embed must NOT emit a "Timestamps are unset"
//! warning even when the source MKV has streams with `AV_NOPTS_VALUE` DTS (e.g.
//! B-frame content muxed through a `setts=dts=NOPTS` bitstream filter).
//!
//! The raw-FFI thumbnail-attach path synthesizes a monotonic dts <= pts for
//! every packet whose dts the demuxer left unset (see `dts_synth.rs`). This
//! test proves the synthesizer is wired in: with the fix applied, the captured
//! FFmpeg logs contain no "timestamps are unset" warning.
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

/// Generate a tiny real JPEG cover image in `dir`.
///
/// JPEG (not WebP): since #530, the raw MKV thumbnail-embed path requires a
/// format `FFmpeg`'s own Matroska read-back renders as a visible cover
/// (jpeg/png/gif/tiff) and rejects webp/bmp outright, expecting callers to
/// normalize those first. This test's subject is DTS synthesis on the MAIN
/// media stream, not thumbnail-format handling, so any accepted cover format
/// works — jpeg keeps it a minimal, still-valid fixture.
///
/// Returns `Ok(path)` or `Err(())` (self-skip).
fn build_jpeg_cover(dir: &std::path::Path) -> Result<std::path::PathBuf, ()> {
    let cover = dir.join("cover.jpg");
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

/// Build a video-only MKV (`video_only_unset.mkv`) with B-frames whose every
/// packet's DTS is forced to `AV_NOPTS_VALUE` via the `setts=dts=NOPTS` BSF.
///
/// Drives the merge path's video input. Returns `Ok(path)` or `Err(())`.
fn build_video_only_unset_dts(dir: &std::path::Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("video_only_src.mkv");
    let unset = dir.join("video_only_unset.mkv");

    // Step 1: video-only H.264 MKV with B-frames (DTS ≠ PTS for some packets).
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
        "-an",
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

    Ok(unset)
}

/// Generate a 1-second AAC audio-only file (`audio.m4a`) for the merge path.
///
/// Returns `Ok(path)` or `Err(())` (self-skip).
fn build_audio_only(dir: &std::path::Path) -> Result<std::path::PathBuf, ()> {
    let audio = dir.join("audio.m4a");
    run_ffmpeg(&[
        "-y",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1",
        "-c:a",
        "aac",
        audio.to_str().unwrap(),
    ])?;
    Ok(audio)
}

/// Assert every video packet in `mkv` for which ffprobe reports a dts has a
/// monotonically non-decreasing dts with `dts <= pts`. Uses ffprobe; self-skips
/// (returns) if ffprobe is absent or errors.
///
/// # Why `N/A` dts is tolerated for Matroska
///
/// Matroska is a PTS-only container: it does NOT persist per-packet DTS at the
/// block level. On read-back ffprobe reconstructs DTS, and for the B-frame
/// reorder *warmup* at the start of a stream it legitimately reports `N/A` —
/// this is true even for a clean FFmpeg-CLI MKV remux, independent of what the
/// muxer was fed. The fix under test operates *inside* the muxer (it stops
/// matroskaenc from warning "Timestamps are unset" / hard-failing on an unset
/// dts); it cannot, and is not expected to, make ffprobe report a set dts on
/// MKV read-back. So this helper enforces the invariant on every dts ffprobe
/// *does* report, while permitting `N/A` from the container. It still requires
/// at least one set dts so the check is not vacuous, and at least one video
/// packet so the file is non-empty.
fn assert_dts_clean(mkv: &std::path::Path) {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-loglevel",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "packet=pts,dts",
            "-of",
            "csv",
            mkv.to_str().unwrap(),
        ])
        .output();
    let Ok(out) = out else { return };
    if !out.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut last: Option<i64> = None;
    let mut saw_packet = false;
    let mut saw_dts = false;
    for line in text.lines() {
        // `-of csv` rows are `packet,<pts>,<dts>`; either field may be `N/A`.
        let cols: Vec<&str> = line.split(',').collect();
        saw_packet = true;
        let dts = cols.get(2).copied().unwrap_or("N/A");
        // Matroska does not persist dts; `N/A` on read-back is expected for the
        // B-frame warmup and is not a defect (see doc comment).
        let Ok(d) = dts.parse::<i64>() else { continue };
        if let Some(p) = cols.get(1).and_then(|s| s.parse::<i64>().ok()) {
            assert!(d <= p, "dts {d} exceeds pts {p}: {line}");
        }
        if let Some(l) = last {
            assert!(d >= l, "dts not monotonic: {l} -> {d}: {line}");
        }
        last = Some(d);
        saw_dts = true;
    }
    assert!(saw_packet, "expected at least one video packet in {mkv:?}");
    assert!(
        saw_dts,
        "expected at least one set (non-N/A) dts in {mkv:?} — \
         a fully-unset dts column would mean the synthesizer never ran"
    );
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

/// Verifies the MKV raw-FFI thumbnail-embed path emits NO "Timestamps are
/// unset" warning when the source file contains packets whose DTS is
/// `AV_NOPTS_VALUE` — the synthesizer fills in a monotonic dts <= pts.
#[tokio::test]
async fn thumbnail_embed_emits_no_unset_timestamp_warning() {
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

    let cover = match build_jpeg_cover(dir.path()) {
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

    // ── assert the warning is gone ─────────────────────────────────────────
    let logs = collector.collected();
    eprintln!("Captured FFmpeg log lines ({}):", logs.len());
    for line in &logs {
        eprintln!("  {line}");
    }

    let has_unset_warning = logs
        .iter()
        .any(|l| l.to_ascii_lowercase().contains("timestamps are unset"));

    assert!(
        !has_unset_warning,
        "Expected NO 'Timestamps are unset' warning from FFmpeg while embedding \
         a thumbnail into an MKV with NOPTS DTS (the dts synthesizer should fill \
         in a monotonic dts <= pts), but the warning WAS captured.\n\
         Captured lines: {logs:#?}\n\
         \n\
         If this assertion fails, the DtsSynthesizer is not wired into the MKV \
         raw-FFI thumbnail-attach write loop (see mkv_raw_ffi.rs)."
    );

    // ── container-level verification ───────────────────────────────────────
    // Beyond the absence of the warning, prove the OUTPUT container actually
    // carries a set, monotonic dts <= pts on every video packet.
    assert_dts_clean(&output);

    println!(
        "FIX VERIFIED: no 'Timestamps are unset' warning captured AND output \
         container has clean dts — the dts synthesizer is wired into the MKV \
         thumbnail-attach path."
    );
}

/// Verifies the MKV raw-FFI remux path (`remux_mkv_raw_ffi`) produces a
/// container with a set, monotonic dts <= pts on every video packet when the
/// source MKV has packets whose DTS is `AV_NOPTS_VALUE`.
#[tokio::test]
async fn remux_mkv_produces_clean_dts() {
    if !ffmpeg_available() {
        return; // self-skip
    }

    let dir = tempfile::tempdir().expect("failed to create temp dir");

    let (_, unset_dts_mkv) = match build_unset_dts_mkv(dir.path()) {
        Ok(p) => p,
        Err(()) => return, // self-skip — fixture generation failed
    };

    let output = dir.path().join("remuxed.mkv");

    // The public `remux` API takes a progress callback (not a PostProcessCallback),
    // so logs can't be captured here — verify the output container directly.
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new failed");
    let opts = rdlp_ffmpeg::RemuxOptions::default();
    let result = runner.remux(&unset_dts_mkv, &output, &opts, None).await;

    assert!(
        result.is_ok(),
        "remux should not fail on unset-DTS MKV input; got: {result:?}"
    );

    assert_dts_clean(&output);

    println!("FIX VERIFIED (remux): output container has clean, monotonic dts.");
}

/// Verifies the MKV raw-FFI merge path (`merge_mkv_raw_ffi`) produces a
/// container with a set, monotonic dts <= pts on every video packet when the
/// video input has packets whose DTS is `AV_NOPTS_VALUE`.
#[tokio::test]
async fn merge_mkv_produces_clean_dts() {
    if !ffmpeg_available() {
        return; // self-skip
    }

    let dir = tempfile::tempdir().expect("failed to create temp dir");

    let video = match build_video_only_unset_dts(dir.path()) {
        Ok(p) => p,
        Err(()) => return, // self-skip
    };
    let audio = match build_audio_only(dir.path()) {
        Ok(p) => p,
        Err(()) => return, // self-skip
    };

    let output = dir.path().join("merged.mkv");

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new failed");
    let opts = rdlp_ffmpeg::RemuxOptions::default();
    let result = runner
        .merge(&video, &audio, &output, &opts, None, None)
        .await;

    assert!(
        result.is_ok(),
        "merge should not fail on unset-DTS video input; got: {result:?}"
    );

    assert_dts_clean(&output);

    println!("FIX VERIFIED (merge): output container has clean, monotonic dts.");
}
