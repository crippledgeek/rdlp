//! End-to-end proof that an empty `--video-encoder` override reaching
//! [`RecodeStage::process`] is treated as "no override", not as a request
//! for an encoder literally named `""` (Item 17 of PR-3's `#618` re-review).
//!
//! Before the fix, the CLI pre-flight validator
//! (`rdlp_ffmpeg::resolve_recode_encoder`, called from `rdlp-cli/src/config.rs`)
//! already filtered an empty string as "no override", while `RecodeStage`
//! discriminated on `Some(_)` directly and took the override branch —
//! producing a blank-name error ("video encoder '' is not available in this
//! `FFmpeg` build") the CLI validator itself would never have surfaced. This
//! test drives the real [`Pipeline`] end to end so the fix is proven at the
//! `process()` entry point, not only at the `build_convert_options` helper
//! (covered by a synchronous unit test alongside `RecodeStage` itself).
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! input fixture, mirroring `tests/cancel_e2e.rs`'s convention) — CLI-spawn
//! use is restricted to `tests/` by `scripts/check-no-cli.sh`, which forbids
//! `std::process::Command` anywhere under `crates/*/src/` (production code
//! enforces pure libav-only `FFmpeg` usage via `ffmpeg-the-third`).
//!
//! Route note: with the fix applied, `RecodeStage::can_remux` currently
//! returns unconditionally `true` for [`ContainerFormat::Mkv`] (tracked as
//! **#630**), so this scenario reaches `Ok` via the remux path, not the
//! transcode path — it still proves the fix (pre-fix, it failed on the
//! transcode branch, before remux short-circuiting was even reached). Once
//! `#630` resolves `can_remux` to something conditional, this test's route
//! through the code may change; it should not be read as coverage of the
//! transcode branch specifically.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::process::Command;
use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::{FileTracker, PipelineMessage, PipelineStage, TempRegistry};
use rdlp_postprocess::{FFmpegRunner, PostProcess, RecodeStage};
use rdlp_types::{ContainerFormat, InfoDict};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Build a tiny 1 s H.264 mp4 fixture — only the fixture build spawns the
/// `ffmpeg` CLI; the recode itself goes through the real `FFmpegRunner`.
fn build_fixture(dir: &std::path::Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("src.mp4");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=320x240",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&src)
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(src) } else { Err(()) }
}

#[tokio::test]
async fn empty_video_encoder_override_is_treated_as_no_override_in_process() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg CLI unavailable to build fixture");
        return;
    }

    let dir = TempDir::new().expect("tempdir");
    let src = build_fixture(dir.path()).expect("fixture build must succeed");

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = RecodeStage::new(ffmpeg);

    let config = PostProcess {
        recode_video: Some(ContainerFormat::Mkv),
        video_encoder: Some(String::new()),
        ..PostProcess::default()
    };
    let reg = Arc::new(TempRegistry::new());
    let msg = PipelineMessage {
        info: InfoDict::new(
            "id".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        ),
        tracker: FileTracker::new(vec![src], reg),
        config: Arc::new(config),
        original_stem: "test".to_string(),
        is_hls: false,
        verbose: false,
        callback_factory: None,
        warnings: Vec::new(),
        encoding_tool: None,
        cancel: tokio_util::sync::CancellationToken::new(),
    };

    let result = stage.process(msg).await;
    assert!(
        result.is_ok(),
        "an empty --video-encoder override reaching RecodeStage::process must be \
         treated as \"no override\" (matching the CLI pre-flight validator), not \
         produce a blank-encoder-name error: {:?}",
        result.err()
    );
}
