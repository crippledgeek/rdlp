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
//! Spawn `run(...)`, wait until the encode is observably in flight, then
//! `token.cancel()`. "In flight" is established by the recode stage's own
//! progress callback: the fraction is emitted from inside the FFmpeg packet
//! loop, so a non-zero value means frames are being processed and the
//! cooperative `check_cancelled` these tests target is reachable — not the
//! pre-loop classification a pre-cancelled token would hit.
//!
//! This replaces a fixed 250 ms sleep. The sleep was sound for
//! `cancel_mid_recode_aborts_and_cleans_up`, whose only precondition is that a
//! 30 s encode is still running. It was NOT sound for
//! `cancel_after_remux_deletes_real_named_source`, which additionally requires
//! `RemuxStage` to have *completed* before the cancel — otherwise `video.ts` is
//! still the live input rather than a superseded temp file, and the #404
//! deletion assertion fails. Under contention 250 ms did not cover the ts->mp4
//! remux, and that test failed in exactly that way during an unrelated gate run
//! on 2026-09-08 while a parallel build saturated the box.
//!
//! Stage ordering makes the progress signal sufficient for both: `spawn_chain`
//! joins stages by a channel and only forwards after `process` returns, so no
//! recode progress can be emitted while the remux is still running.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use rdlp_core::{PostProcessCallback, PostProcessCallbackFactory};
use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{
    FFmpegRunner, Pipeline, PipelineError, PipelineRunOptions, RecodeStage, RemuxStage,
    TempRegistry,
};
use rdlp_types::{ContainerFormat, InfoDict, PostProcess, Progress};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a long (30 s) H.264 yuv420p fixture so the recode encode loop has
/// thousands of frames to process — the encode is still running long after the
/// first progress report the cancel waits on.
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

/// Build a long (30 s) H.264 **MPEG-TS** fixture. The `.ts` extension forces
/// `RemuxStage` (is_hls) to remux ts → mp4, which `replace()`s the original
/// into `temp_files` — the exact #404 condition.
fn build_long_ts_fixture(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("src.ts");
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
            "-f",
            "mpegts",
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

/// Upper bound on how long to wait for the encode to reach its loop.
///
/// Bounds a hang, not the expected wait. Generous on purpose: a value tuned to
/// observed timings would reintroduce the wall-clock assumption this exists to
/// remove.
const ENCODE_IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(60);

/// Signals the first non-zero progress reported by the stage it was built for.
struct RecodeBeacon {
    /// `None` for every stage but the recode one, so only that stage can
    /// signal. `RemuxStage` reports progress too, and its progress means the
    /// remux has *started* — the opposite of the precondition
    /// `cancel_after_remux_deletes_real_named_source` needs.
    tx: Option<tokio::sync::watch::Sender<bool>>,
}

impl PostProcessCallback for RecodeBeacon {
    fn on_progress(&self, progress: Progress) {
        // Strict `>`: the first video packet is at pts 0 and reports 0.0, which
        // does not yet prove the loop advanced.
        if progress > Progress::ZERO
            && let Some(tx) = &self.tx
        {
            let _ = tx.send(true);
        }
    }

    fn on_log(&self, message: &str) {
        // Supplying a factory at all installs FFmpeg's global log forwarder, so
        // without this every FFmpeg line would be swallowed by the trait's
        // no-op default — exactly when a failing run needs them most.
        eprintln!("{message}");
    }
}

/// A callback factory paired with a receiver that resolves once `stage_name`
/// reports progress.
///
/// This is the production progress path, not a proxy for it: the fraction is
/// emitted from inside the FFmpeg packet loop, after the cooperative
/// `check_cancelled` these tests target.
///
/// `stage_name` is passed in from `RecodeStage::name()` rather than written as
/// a literal here. A literal that drifted from `name()` would not fail at the
/// point of breakage: both tests would block for the full timeout before
/// reporting anything.
///
/// Two earlier conditions were tried and rejected against a real run. A fixed
/// sleep cannot know whether a preceding stage finished. Watching the recode
/// output file grow looks equivalent and is not: Matroska buffers clusters, so
/// the file's length stays flat and then jumps near the end — waiting on growth
/// let the whole 30 s encode complete before the cancel fired, and both tests
/// failed with `Ok` where they expected `Cancelled`.
fn recode_progress_beacon(
    stage_name: String,
) -> (
    PostProcessCallbackFactory,
    tokio::sync::watch::Receiver<bool>,
) {
    let (tx, rx) = tokio::sync::watch::channel(false);
    let factory: PostProcessCallbackFactory = Arc::new(move |stage: &str| {
        Arc::new(RecodeBeacon {
            tx: (stage == stage_name).then(|| tx.clone()),
        }) as Arc<dyn PostProcessCallback>
    });
    (factory, rx)
}

/// Wait until the recode stage is observably encoding.
async fn wait_for_encode_in_flight(rx: &mut tokio::sync::watch::Receiver<bool>, stage_name: &str) {
    tokio::time::timeout(ENCODE_IN_FLIGHT_TIMEOUT, rx.wait_for(|started| *started))
        .await
        .unwrap_or_else(|_| {
            // States the observation, not a cause: several distinct failures
            // produce it, and naming only one sends the next reader the wrong
            // way.
            panic!(
                "no progress from stage {stage_name:?} within \
                 {ENCODE_IN_FLIGHT_TIMEOUT:?} — either the stage name drifted from the \
                 test's filter, the recode took a stream-copy path, or the encode stalled"
            )
        })
        .expect("progress channel closed before the recode stage reported anything");
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
    // Name sourced from the stage itself, so a rename cannot silently
    // desynchronise the beacon's filter from the stage it is waiting on.
    let recode = RecodeStage::new(ffmpeg);
    // Owned: `PipelineStage::name` borrows from `self`, which is moved into the
    // stage vector below.
    let recode_stage_name = recode.name().to_owned();
    let stages: Vec<Arc<dyn PipelineStage>> = vec![Arc::new(recode)];
    let pipeline = Arc::new(Pipeline::new(stages, Arc::new(TempRegistry::new()), 4));

    // Recode to MKV with an EXPLICIT encoder so the remux (stream-copy) fast
    // path is disabled and the real transcode loop runs (`video_encoder` set
    // forces `can_remux == false` in RecodeStage::process).
    let config = Arc::new(PostProcess {
        recode_video: Some(ContainerFormat::Mkv),
        video_encoder: Some(rdlp_types::VideoEncoderName::from_static("libx264")),
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
    let (factory, mut progress) = recode_progress_beacon(recode_stage_name.clone());

    let handle = tokio::spawn(async move {
        pipeline_for_run
            .run(
                info,
                vec![input],
                PipelineRunOptions {
                    keep_inputs: false,
                    is_hls: false,
                    verbose: false,
                },
                config,
                "video".to_string(),
                Some(factory),
                Some(token_for_run),
            )
            .await
    });

    // Cancel only once the encode is observably mid-loop.
    wait_for_encode_in_flight(&mut progress, &recode_stage_name).await;
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

/// #404 guard: cancelling mid-recode after a Remux stage has superseded the
/// original download must delete the **real-named** source file (which lives in
/// `temp_files`), not just the `*.rdlp-tmp-*` intermediates.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_after_remux_deletes_real_named_source() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = TempDir::new().expect("tempdir");
    let Ok(src) = build_long_ts_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let input = work.join("video.ts"); // real-named HLS download
    std::fs::rename(&src, &input).unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpegRunner::new"));
    let recode = RecodeStage::new(Arc::clone(&ffmpeg));
    // Owned: `PipelineStage::name` borrows from `self`, which is moved below.
    let recode_stage_name = recode.name().to_owned();
    let stages: Vec<Arc<dyn PipelineStage>> = vec![
        Arc::new(RemuxStage::new(ffmpeg)), // ts->mp4: video.ts -> temp_files
        Arc::new(recode),                  // slow encode -> cancel target
    ];
    let pipeline = Arc::new(Pipeline::new(stages, Arc::new(TempRegistry::new()), 4));

    let config = Arc::new(PostProcess {
        recode_video: Some(ContainerFormat::Mkv),
        video_encoder: Some(rdlp_types::VideoEncoderName::from_static("libx264")),
        ..PostProcess::default()
    });
    let info = InfoDict::new(
        "id".to_string(),
        "Cancel Src".to_string(),
        "TestExtractor".to_string(),
        "https://example.com/video".to_string(),
    );

    let token = CancellationToken::new();
    let token_for_run = token.clone();
    let pipeline_for_run = Arc::clone(&pipeline);
    let input_for_run = input.clone();
    let (factory, mut progress) = recode_progress_beacon(recode_stage_name.clone());
    let handle = tokio::spawn(async move {
        pipeline_for_run
            .run(
                info,
                vec![input_for_run],
                PipelineRunOptions {
                    keep_inputs: false,
                    is_hls: true,
                    verbose: false,
                },
                config,
                "video".to_string(),
                Some(factory),
                Some(token_for_run),
            )
            .await
    });

    // Recode progress additionally proves RemuxStage finished: stages run in
    // order, so the recode loop cannot be reporting until the ts->mp4 remux has
    // completed and `video.ts` has become a superseded temp file. That is the
    // precondition the #404 assertion below depends on, and the one a fixed
    // sleep could not establish.
    wait_for_encode_in_flight(&mut progress, &recode_stage_name).await;
    token.cancel();

    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("pipeline must complete within 10s after cancel")
        .expect("pipeline task join");

    let err = result.expect_err("mid-recode cancel must surface as Err");
    assert!(
        matches!(
            err.downcast_ref::<PipelineError>(),
            Some(PipelineError::Cancelled)
        ),
        "expected PipelineError::Cancelled, got: {err:?}"
    );
    assert!(
        !input.exists(),
        "real-named source in temp_files must be deleted on cancel (#404)"
    );
}
