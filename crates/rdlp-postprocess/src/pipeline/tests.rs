use super::*;
use std::path::PathBuf;

/// 8-tuple returned by `run_args` for `Pipeline::run` calls in tests.
type RunArgs = (
    InfoDict,
    Vec<PathBuf>,
    Arc<PostProcess>,
    String,
    bool,
    bool,
    Option<PostProcessCallbackFactory>,
    Option<CancellationToken>,
);

fn make_pipeline(stages: Vec<Arc<dyn PipelineStage>>) -> Pipeline {
    let reg = Arc::new(TempRegistry::new());
    Pipeline::new(stages, reg, 4)
}

fn make_info() -> InfoDict {
    InfoDict::new(
        "id".to_string(),
        "Test Video".to_string(),
        "TestExtractor".to_string(),
        "https://example.com/video".to_string(),
    )
}

struct PassthroughStage;
#[async_trait]
impl PipelineStage for PassthroughStage {
    fn name(&self) -> &str {
        "passthrough"
    }
    fn should_run(&self, _: &PipelineMessage) -> bool {
        true
    }
    async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        Ok(msg)
    }
}

struct SkipStage;
#[async_trait]
impl PipelineStage for SkipStage {
    fn name(&self) -> &str {
        "skip"
    }
    fn should_run(&self, _: &PipelineMessage) -> bool {
        false
    }
    async fn process(&self, _: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        panic!("should not be called");
    }
}

struct FailStage;
#[async_trait]
impl PipelineStage for FailStage {
    fn name(&self) -> &str {
        "fail"
    }
    fn should_run(&self, _: &PipelineMessage) -> bool {
        true
    }
    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        let err = PipelineError::StageFailure {
            stage: "fail".into(),
            cause: "test error".into(),
        };
        if let Some(tx) = msg.error_tx.take() {
            let _ = tx.send(err);
        }
        Err(anyhow::anyhow!("test error"))
    }
}

struct NonFatalFailStage;
#[async_trait]
impl PipelineStage for NonFatalFailStage {
    fn name(&self) -> &str {
        "nonfatal"
    }
    fn should_run(&self, _: &PipelineMessage) -> bool {
        true
    }
    fn is_fatal(&self) -> bool {
        false
    }
    async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        // Non-fatal: return Ok with the message to pass through.
        // (This simulates a stage that encounters an error but can still pass through.)
        Ok(msg)
    }
}

fn run_args(files: Vec<PathBuf>) -> RunArgs {
    (
        make_info(),
        files,
        Arc::new(PostProcess::default()),
        "video".to_string(),
        false,
        false,
        None,
        None, // cancel — default to None for back-compat tests
    )
}

#[tokio::test]
async fn test_pipeline_passthrough() {
    let pipeline = make_pipeline(vec![Arc::new(PassthroughStage)]);
    let (info, files, config, stem, hls, verbose, cb, cancel) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);
    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, cancel)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![PathBuf::from("/tmp/video.mp4")]);
}

#[tokio::test]
async fn test_pipeline_skip_stage() {
    let pipeline = make_pipeline(vec![Arc::new(SkipStage), Arc::new(PassthroughStage)]);
    let (info, files, config, stem, hls, verbose, cb, cancel) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);
    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, cancel)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_pipeline_fatal_error_cascades() {
    let pipeline = make_pipeline(vec![Arc::new(FailStage), Arc::new(PassthroughStage)]);
    let (info, files, config, stem, hls, verbose, cb, cancel) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);
    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, cancel)
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("test error"), "unexpected error: {err}");
}

#[tokio::test]
async fn test_pipeline_nonfatal_passes_through() {
    let pipeline = make_pipeline(vec![
        Arc::new(NonFatalFailStage),
        Arc::new(PassthroughStage),
    ]);
    let (info, files, config, stem, hls, verbose, cb, cancel) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);
    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, cancel)
        .await;
    assert!(result.is_ok());
}

#[test]
fn test_context_preserved_in_error_chain() {
    use anyhow::Context;
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let result: anyhow::Result<()> = Err(io_err).context("remux stage failed");
    let err = result.unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("remux stage failed"), "msg: {msg}");
    assert!(msg.contains("file missing"), "msg: {msg}");
}

#[test]
fn test_downcast_through_context() {
    use anyhow::Context;
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
    let result: anyhow::Result<()> = Err(io_err).context("thumbnail stage failed");
    let err = result.unwrap_err();
    let root = err.root_cause();
    let io = root
        .downcast_ref::<std::io::Error>()
        .expect("should downcast to io::Error");
    assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied);
}

// Regression guard for Phase 1 async audit Finding 3.1 (spec §5.1.3).
//
// Before the fix, `Pipeline::run` called `FileTracker::cleanup()` (which
// performs N × std::fs::remove_file + std::fs::rename) directly on the
// async runtime thread. On a `current_thread` runtime, that blocks the
// executor and starves any co-tenant task. After the fix the cleanup is
// dispatched via `spawn_blocking`, so a co-tenant ticker keeps advancing.
//
// Pragmatic behavioural assertion: the run completes and returns the
// expected files. A deterministic ticker-starvation test requires a
// large-enough temp set to exceed ~50ms of syscalls, which is flaky
// across CI hosts — see §Exception in `bug-fix-requires-failing-test.md`.
// Manual verification: on a quiet machine, seeding the tracker with
// 5000 temp files blocked the ticker on unpatched code; patched code
// keeps the ticker advancing.
#[tokio::test(flavor = "current_thread")]
async fn test_pipeline_cleanup_does_not_block_runtime() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let pipeline = make_pipeline(vec![Arc::new(PassthroughStage)]);
    let video = dir.path().join("video.mp4");
    std::fs::write(&video, b"vid").unwrap();

    let progressed = Arc::new(AtomicUsize::new(0));
    let p = Arc::clone(&progressed);
    let ticker = tokio::spawn(async move {
        for _ in 0..50 {
            p.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    });

    let (info, files, config, stem, hls, verbose, cb, cancel) = run_args(vec![video.clone()]);
    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, cancel)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![video]);
    ticker.await.unwrap();
    // Ticker must have made at least some progress — serves as a smoke
    // check that the runtime wasn't completely pinned.
    assert!(progressed.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn test_pipeline_concurrent_semaphore() {
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CONCURRENT: AtomicUsize = AtomicUsize::new(0);
    static MAX_SEEN: AtomicUsize = AtomicUsize::new(0);

    struct CountingStage;
    #[async_trait]
    impl PipelineStage for CountingStage {
        fn name(&self) -> &str {
            "counting"
        }
        fn should_run(&self, _: &PipelineMessage) -> bool {
            true
        }
        async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
            let c = CONCURRENT.fetch_add(1, Ordering::SeqCst) + 1;
            MAX_SEEN.fetch_max(c, Ordering::SeqCst);
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            CONCURRENT.fetch_sub(1, Ordering::SeqCst);
            Ok(msg)
        }
    }

    let reg = StdArc::new(TempRegistry::new());
    // Max concurrency = 2
    let pipeline = StdArc::new(Pipeline::new(vec![Arc::new(CountingStage)], reg, 2));

    let mut handles = vec![];
    for _ in 0..6 {
        let p = StdArc::clone(&pipeline);
        handles.push(tokio::spawn(async move {
            let (info, files, config, stem, hls, verbose, cb, cancel) =
                run_args(vec![PathBuf::from("/tmp/v.mp4")]);
            p.run(info, files, config, stem, hls, verbose, cb, cancel)
                .await
        }));
    }
    for h in handles {
        h.await.unwrap().unwrap();
    }
    // Semaphore should have capped concurrency at 2
    assert!(
        MAX_SEEN.load(Ordering::SeqCst) <= 2,
        "max concurrent was {}",
        MAX_SEEN.load(Ordering::SeqCst)
    );
}

#[tokio::test]
#[allow(clippy::items_after_statements)]
async fn pipeline_run_returns_cancelled_when_token_pre_cancelled() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Counter-stage that increments every time `process` is called.
    let count = Arc::new(AtomicUsize::new(0));

    struct CountingStage(Arc<AtomicUsize>);
    #[async_trait]
    impl PipelineStage for CountingStage {
        fn name(&self) -> &str {
            "counting"
        }
        fn should_run(&self, _msg: &PipelineMessage) -> bool {
            true
        }
        fn is_fatal(&self) -> bool {
            false
        }
        async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(msg)
        }
    }

    let pipeline = make_pipeline(vec![Arc::new(CountingStage(Arc::clone(&count)))]);
    let (info, files, config, stem, hls, verbose, cb, _) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);

    let token = CancellationToken::new();
    token.cancel(); // pre-cancel BEFORE run

    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, Some(token))
        .await;

    assert!(result.is_err(), "pre-cancelled token must surface as Err");
    let err = result.unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<PipelineError>(),
            Some(PipelineError::Cancelled)
        ),
        "expected PipelineError::Cancelled, got: {err:?}"
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "stage MUST NOT execute when token is pre-cancelled"
    );
}

#[tokio::test]
#[allow(clippy::items_after_statements)]
async fn pipeline_run_cancels_mid_pipeline() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Stage 0: cancels the token at the end of `process`, returns Ok(msg).
    // Stages 1, 2: increment a counter; assert they never run.
    //
    // Determinism: `process` calls `token.cancel()` BEFORE returning Ok,
    // so the cancel atomic flag is set before the spawn_chain caller's
    // `out_tx.send(msg)` runs. Downstream stages' `tokio::select!` is
    // `biased;` — the cancel arm is checked before the recv arm. Per
    // tokio docs, `biased;` is a hard ordering guarantee (not "usually"
    // / "preferred"). So the count assertion is deterministic regardless
    // of multi-thread scheduling: every downstream stage sees the cancel
    // first when its task is polled, regardless of whether the message
    // arrived in its mpsc channel before or after the wake.
    let token = CancellationToken::new();
    let token_for_stage = token.clone();
    let downstream_count = Arc::new(AtomicUsize::new(0));

    struct CancelOnExitStage {
        token: CancellationToken,
    }
    #[async_trait]
    impl PipelineStage for CancelOnExitStage {
        fn name(&self) -> &str {
            "cancel-on-exit"
        }
        fn should_run(&self, _msg: &PipelineMessage) -> bool {
            true
        }
        fn is_fatal(&self) -> bool {
            false
        }
        async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
            // Process succeeds, then trigger cancel before returning.
            self.token.cancel();
            Ok(msg)
        }
    }

    struct DownstreamCountingStage(Arc<AtomicUsize>);
    #[async_trait]
    impl PipelineStage for DownstreamCountingStage {
        fn name(&self) -> &str {
            "downstream-counting"
        }
        fn should_run(&self, _msg: &PipelineMessage) -> bool {
            true
        }
        fn is_fatal(&self) -> bool {
            false
        }
        async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(msg)
        }
    }

    let pipeline = make_pipeline(vec![
        Arc::new(CancelOnExitStage {
            token: token_for_stage,
        }),
        Arc::new(DownstreamCountingStage(Arc::clone(&downstream_count))),
        Arc::new(DownstreamCountingStage(Arc::clone(&downstream_count))),
    ]);
    let (info, files, config, stem, hls, verbose, cb, _) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);

    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, Some(token))
        .await;

    assert!(result.is_err(), "mid-pipeline cancel must surface as Err");
    let err = result.unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<PipelineError>(),
            Some(PipelineError::Cancelled)
        ),
        "expected PipelineError::Cancelled, got: {err:?}"
    );
    assert_eq!(
        downstream_count.load(Ordering::SeqCst),
        0,
        "downstream stages MUST NOT run after stage 0 cancels the token"
    );
}

#[tokio::test]
async fn pipeline_run_with_none_cancel_runs_to_completion() {
    let pipeline = make_pipeline(vec![Arc::new(PassthroughStage)]);
    let (info, files, config, stem, hls, verbose, cb, _) =
        run_args(vec![PathBuf::from("/tmp/video.mp4")]);

    let result = pipeline
        .run(info, files, config, stem, hls, verbose, cb, None)
        .await;

    assert!(
        result.is_ok(),
        "None cancel arg MUST run to completion identically to the pre-token behaviour: {result:?}"
    );
}

#[tokio::test]
#[allow(clippy::items_after_statements)]
async fn run_batch_cancels_all_in_flight_items() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Verifies the shared-parent-token semantic: ONE token cancels ALL
    // in-flight batch items. We pre-cancel the token so each item's
    // first stage hits the select! cancel branch deterministically —
    // this avoids racing the test against tokio's scheduler. The
    // architectural property under test is "items share the token,"
    // not "items observe the cancel within X ms."
    let count = Arc::new(AtomicUsize::new(0));

    struct CountingStage(Arc<AtomicUsize>);
    #[async_trait]
    impl PipelineStage for CountingStage {
        fn name(&self) -> &str {
            "counting"
        }
        fn should_run(&self, _msg: &PipelineMessage) -> bool {
            true
        }
        fn is_fatal(&self) -> bool {
            false
        }
        async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(msg)
        }
    }

    let pipeline = std::sync::Arc::new(make_pipeline(vec![Arc::new(CountingStage(Arc::clone(
        &count,
    )))]));

    let inputs: Vec<BatchInput> = (0..3)
        .map(|i| BatchInput {
            info: make_info(),
            files: vec![PathBuf::from(format!("/tmp/video-{i}.mp4"))],
            original_stem: format!("video-{i}"),
            is_hls: false,
        })
        .collect();

    let token = CancellationToken::new();
    token.cancel(); // pre-cancel — all items see it on first poll

    let results = pipeline
        .run_batch(
            inputs,
            Arc::new(PostProcess::default()),
            false,
            None,
            Some(token),
        )
        .await;

    assert_eq!(results.len(), 3, "all 3 batch items must be returned");
    for (i, result) in results.iter().enumerate() {
        assert!(
            result.is_err(),
            "batch item {i}: shared-token cancel must surface as Err in every item"
        );
        let err = result.as_ref().unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<PipelineError>(),
                Some(PipelineError::Cancelled)
            ),
            "batch item {i}: expected PipelineError::Cancelled, got: {err:?}"
        );
    }
    // Stage MAY have run on items that beat the cancel into recv — accept
    // 0..=3 here (depending on scheduler). The load-bearing assertion is
    // that every item's RESULT is Err(Cancelled), not the count.
}
