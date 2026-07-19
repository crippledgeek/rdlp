//! Shared fixtures for the `ThumbnailStage` / `SubtitleStage` integration
//! suites.
//!
//! `thumbnail_explicit_container_548.rs` (#548), `..._551.rs` (#551),
//! `thumbnail_borrowed_sidecar.rs` and `subtitle_borrowed_sidecar.rs` each need
//! the same three things: a real ffmpeg-built media fixture, a real image or
//! subtitle sidecar, and a `PipelineMessage` wrapping them. Those helpers were
//! copied per-file as the suites were added; this module is the single
//! definition.
//!
//! Real, decodable fixtures are mandatory across all three suites. #548 shipped
//! tests that passed against UNPATCHED code because their fixtures were
//! nonexistent paths: a missing input makes the pre-existing auto-remux failure
//! warning coincidentally contain the fixture's own extension and the word
//! "thumbnail", so the assertions matched without the fix being present.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    dead_code
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use rdlp_postprocess::PostProcess;
use rdlp_postprocess::pipeline::{FileTracker, PipelineMessage, TempRegistry};
use rdlp_types::InfoDict;

/// `ffmpeg` is present but the fixture could not be built — a broken muxer
/// name, a libx264-less build, or a permissions problem. This must FAIL the
/// suite, never skip it: a silent skip would leave every test vacuously green
/// with only an `eprintln` that `cargo test` swallows without `--nocapture`,
/// reproducing exactly the failure mode #548 hit.
pub const FIXTURE_FAILED: &str = "ffmpeg is available but building the fixture failed";

/// Whether the system `ffmpeg` CLI is usable. Used ONLY to build fixtures —
/// production code never spawns a subprocess (library-first rule).
pub fn ffmpeg_cli_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Build a 1-frame H.264 fixture, muxed explicitly as `muxer` (the output
/// extension alone is ambiguous for some containers, e.g. `.f4v`, so the muxer
/// name is passed via `-f`).
pub fn build_video_fixture(path: &Path, muxer: &str) -> Result<(), ()> {
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=d=1:s=320x240",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-f",
        muxer,
        path.to_str().unwrap(),
    ])
}

/// Build a REAL JPEG. Placeholder bytes are not a substitute: an invalid image
/// makes the embed fail early and skip the sidecar cleanup entirely, which
/// hides sidecar-lifecycle bugs — the first investigation of the borrowed-input
/// data loss was misled by exactly that.
pub fn build_jpeg_fixture(path: &Path) -> Result<(), ()> {
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=d=1:s=64x64",
        "-frames:v",
        "1",
        path.to_str().unwrap(),
    ])
}

fn run_ffmpeg(args: &[&str]) -> Result<(), ()> {
    let ok = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(()) }
}

/// The axes the thumbnail suites vary when building a `PipelineMessage`.
/// Grouped into one parameter object so [`make_msg`] stays at three arguments.
pub struct MsgOptions {
    /// Drives `RemuxStage`'s HLS auto-remux path.
    pub is_hls: bool,
    /// `true` selects [`FileTracker::new_borrowing`] (a user-owned local file,
    /// which must never be deleted) over [`FileTracker::new`] (a file rdlp
    /// downloaded and therefore owns).
    pub borrowing: bool,
    /// Stem used for thumbnail sidecar discovery.
    pub original_stem: &'static str,
}

impl Default for MsgOptions {
    fn default() -> Self {
        Self {
            is_hls: false,
            borrowing: false,
            original_stem: "video",
        }
    }
}

/// `MsgOptions` for a run over a media file with the given stem, choosing
/// whether the input is user-owned (`borrowing`) or rdlp-downloaded.
///
/// The sidecar suites all need exactly this pair, and BOTH values matter:
/// a gate of the form `!write_x && ownership.is_disposable()` short-circuits
/// on ownership, so a test that only ever passes `borrowing: true` cannot pin
/// the `write_x` half of it (caught in review — all three subtitle tests
/// stayed green with `!write_subtitles` deleted outright).
pub fn opts(stem: &'static str, borrowing: bool) -> MsgOptions {
    MsgOptions {
        borrowing,
        original_stem: stem,
        ..MsgOptions::default()
    }
}

/// Build a `PipelineMessage` around `files` for a single-stage test.
pub fn make_msg(files: Vec<PathBuf>, config: PostProcess, opts: MsgOptions) -> PipelineMessage {
    let reg = Arc::new(TempRegistry::new());
    let (error_tx, _) = oneshot::channel();
    let tracker = if opts.borrowing {
        FileTracker::new_borrowing(files, reg)
    } else {
        FileTracker::new(files, reg)
    };
    PipelineMessage {
        info: InfoDict::new(
            "id".to_string(),
            "Test Video".to_string(),
            "TestExtractor".to_string(),
            "https://example.com".to_string(),
        ),
        tracker,
        config: Arc::new(config),
        original_stem: opts.original_stem.to_string(),
        is_hls: opts.is_hls,
        verbose: false,
        callback_factory: None,
        error_tx: Some(error_tx),
        warnings: Vec::new(),
        encoding_tool: None,
        cancel: CancellationToken::new(),
    }
}
