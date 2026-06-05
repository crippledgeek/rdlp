//! End-to-end proof that cancelling a recode pipeline mid-flight aborts the
//! encode AND leaves zero `*.rdlp-tmp-*` artifacts behind (#334, #335).
//!
//! This stitches the full stack together: a real [`Pipeline`] containing a
//! [`RecodeStage`] configured to actually re-encode (so the blocking FFmpeg
//! loop runs), driven against a real H.264 fixture. Cancelling the job's
//! [`CancellationToken`] mid-encode must:
//!   1. surface as [`PipelineError::Cancelled`] (mirroring the orchestrator
//!      downcast at `crates/rdlp-api/src/orchestrator/postprocess.rs:172-177`),
//!      and
//!   2. trigger [`FileTracker`]'s RAII cancel-cleanup, so the partial recode
//!      temp output (`*.rdlp-tmp-*`) is deleted — no leftover artifacts.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! input fixture).
//!
//! ## Timing approach
//!
//! Spawn `run(...)` on a task, sleep ~250 ms, then `token.cancel()`. The
//! fixture is 30 s of `testsrc` re-encoded with libx264 — far longer than the
//! sub-second cancel window, so the encode loop is guaranteed to still be
//! running when the cancel fires. This exercises the *mid-flight* abort path
//! (the cooperative `check_cancelled` in the FFmpeg encode loop from Tasks
//! 4-7), not merely the pre-loop classification a pre-cancelled token would
//! hit. A pre-cancel fallback is unnecessary because the long fixture makes
//! the spawn+sleep+cancel timing robust.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, Pipeline, PipelineError, RecodeStage, TempRegistry};
use rdlp_types::{ContainerFormat, InfoDict, PostProcess};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a long (30 s) H.264 yuv420p fixture so the recode encode loop has
/// thousands of frames to process — guarantees the encode is still in flight
/// when the cancel fires ~250 ms in.
fn build_long_fixture(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("src.mp4");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=30:s=1280x720",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

/// Count files in `dir` whose name contains `.rdlp-tmp-` (leftover pipeline
/// temp artifacts) or ends in `.lock` (registry sidecars).
// Flat scan — pipeline temp files are expected in this dir, not subdirectories.
fn leftover_artifacts(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if name.contains(".rdlp-tmp-") || name.ends_with(".lock") {
            out.push(path);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_mid_recode_aborts_and_cleans_up() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = TempDir::new().expect("tempdir");
    let Ok(src) = build_long_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };

    // Move the fixture into a clean output dir so leftover-artifact scanning
    // only sees pipeline-produced files (the recode temp output lands adjacent
    // to its input).
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let input = work.join("video.mp4");
    std::fs::rename(&src, &input).unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpegRunner::new"));
    let stages: Vec<Arc<dyn PipelineStage>> = vec![Arc::new(RecodeStage::new(ffmpeg))];
    let pipeline = Arc::new(Pipeline::new(stages, Arc::new(TempRegistry::new()), 4));

    // Recode to MKV with an EXPLICIT encoder so the remux (stream-copy) fast
    // path is disabled and the real transcode loop runs (`video_encoder` set
    // forces `can_remux == false` in RecodeStage::process).
    let config = Arc::new(PostProcess {
        recode_video: Some(ContainerFormat::Mkv),
        video_encoder: Some("libx264".to_string()),
        ..PostProcess::default()
    });

    let info = InfoDict::new(
        "id".to_string(),
        "Cancel Test".to_string(),
        "TestExtractor".to_string(),
        "https://example.com/video".to_string(),
    );

    let token = CancellationToken::new();
    let token_for_run = token.clone();
    let pipeline_for_run = Arc::clone(&pipeline);

    let handle = tokio::spawn(async move {
        pipeline_for_run
            .run(
                info,
                vec![input],
                config,
                "video".to_string(),
                false,
                false,
                None,
                Some(token_for_run),
            )
            .await
    });

    // Let the encode loop get going, then cancel mid-flight.
    tokio::time::sleep(Duration::from_millis(250)).await;
    token.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
        .await
        .expect("pipeline must complete within 10s after cancel (timeout = cancel regressed/hung)")
        .expect("pipeline task join");

    // (1) The run must classify as Cancelled — mirror the orchestrator downcast.
    let err = result.expect_err("mid-recode cancel must surface as Err");
    assert!(
        matches!(
            err.downcast_ref::<PipelineError>(),
            Some(PipelineError::Cancelled)
        ),
        "expected PipelineError::Cancelled, got: {err:?}"
    );

    // (2) Zero leftover temp artifacts — the FileTracker RAII Drop must have
    // deleted the partial recode output and released its registry lock.
    let leftovers = leftover_artifacts(&work);
    assert!(
        leftovers.is_empty(),
        "cancel-cleanup must leave zero *.rdlp-tmp-*/.lock artifacts; found: {leftovers:?}"
    );
}
