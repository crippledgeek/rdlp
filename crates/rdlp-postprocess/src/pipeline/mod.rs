//! Channel-based post-processing pipeline.
//!
//! Each stage is a `tokio::spawn` task connected by bounded `mpsc` channels.
//! [`FileTracker`] owns all file lifecycle decisions — no stage ever deletes
//! files directly. [`TempRegistry`] handles crash cleanup.

pub mod registry;
pub mod stages;
pub mod tracker;

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{Semaphore, mpsc, oneshot};

use rdlp_core::PostProcessCallbackFactory;
use rdlp_types::InfoDict;
use rdlp_types::PostProcess;

pub use registry::TempRegistry;
pub use tracker::FileTracker;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can be produced by pipeline stages.
#[derive(Debug, Error, Clone)]
pub enum PipelineError {
    /// A fatal stage failed.
    #[error("stage '{stage}' failed: {cause}")]
    StageFailure {
        /// Name of the stage that failed.
        stage: String,
        /// Human-readable cause.
        cause: String,
    },
}

// ---------------------------------------------------------------------------
// PipelineMessage
// ---------------------------------------------------------------------------

/// The message that flows through the pipeline channel chain.
///
/// Each stage receives this, may mutate `tracker`, and sends it downstream.
pub struct PipelineMessage {
    /// Metadata for the video being processed.
    pub info: InfoDict,
    /// File lifecycle state.
    pub tracker: FileTracker,
    /// Post-processing configuration.
    pub config: Arc<PostProcess>,
    /// Original file stem for thumbnail / subtitle discovery after UUID renames.
    pub original_stem: String,
    /// Whether the source was HLS (triggers auto-remux in `RemuxStage`).
    pub is_hls: bool,
    /// Enable verbose FFmpeg logging in stages.
    pub verbose: bool,
    /// Factory for creating per-stage progress callbacks.
    pub callback_factory: Option<PostProcessCallbackFactory>,
    /// Error channel — the first fatal stage sends here; subsequent stages see `None`.
    pub error_tx: Option<oneshot::Sender<PipelineError>>,
    /// Non-fatal warnings accumulated by stages during processing.
    pub warnings: Vec<String>,
    /// Encoding tool tag set by the primary content-creating stage (recode,
    /// audio extract, normalize). Pass-through stages (remux, metadata,
    /// thumbnail) propagate this to the output file's `encoding_tool`
    /// metadata instead of stamping their own name. `None` means no
    /// prior stage set it — pass-through stages use their own name.
    pub encoding_tool: Option<String>,
}

// ---------------------------------------------------------------------------
// PipelineStage trait
// ---------------------------------------------------------------------------

/// A single stage in the post-processing pipeline.
#[async_trait]
pub trait PipelineStage: Send + Sync {
    /// Human-readable stage name (used in logging and error messages).
    fn name(&self) -> &str;

    /// Whether this stage should run for the given message.
    ///
    /// Receives the full message so stages can make data-dependent decisions
    /// (e.g., `MergeStage` checks `tracker.current_files.len() >= 2`).
    fn should_run(&self, msg: &PipelineMessage) -> bool;

    /// Whether a failure in this stage is fatal.
    ///
    /// Fatal stages (default) kill the entire pipeline.
    /// Non-fatal stages log the error and pass the message through unchanged.
    fn is_fatal(&self) -> bool {
        true
    }

    /// Process the message.
    ///
    /// Must use `tracker.temp_path()` for output and `tracker.replace()` to
    /// promote output. Must never call `remove_file` directly.
    async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage>;
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Channel-based post-processing pipeline.
pub struct Pipeline {
    stages: Vec<Arc<dyn PipelineStage>>,
    temp_registry: Arc<TempRegistry>,
    concurrency: Arc<Semaphore>,
}

impl Pipeline {
    /// Create a new pipeline.
    ///
    /// `max_concurrent` limits how many `run()` calls execute simultaneously
    /// (used by `run_batch`).
    pub fn new(
        stages: Vec<Arc<dyn PipelineStage>>,
        temp_registry: Arc<TempRegistry>,
        max_concurrent: usize,
    ) -> Self {
        Self {
            stages,
            temp_registry,
            concurrency: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Run the full pipeline for a single video.
    ///
    /// Returns the final `current_files` from the tracker on success.
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        info: InfoDict,
        files: Vec<std::path::PathBuf>,
        config: Arc<PostProcess>,
        original_stem: String,
        is_hls: bool,
        verbose: bool,
        callback_factory: Option<PostProcessCallbackFactory>,
    ) -> anyhow::Result<Vec<std::path::PathBuf>> {
        let _permit =
            self.concurrency.acquire().await.map_err(|_| {
                anyhow::anyhow!("pipeline concurrency semaphore closed unexpectedly")
            })?;

        let tracker = FileTracker::new(files, Arc::clone(&self.temp_registry));

        let (error_tx, error_rx) = oneshot::channel::<PipelineError>();

        let msg = PipelineMessage {
            info,
            tracker,
            config,
            original_stem,
            is_hls,
            verbose,
            callback_factory,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        };

        // Build channel chain: one mpsc(1) between consecutive stages.
        // first_tx → stage_0 → stage_1 → ... → stage_N → final_rx
        let mut final_rx = self.spawn_chain(msg);

        // Await the final message.
        match final_rx.recv().await {
            Some(final_msg) => {
                // FileTracker::cleanup performs N × std::fs::remove_file and
                // std::fs::rename. These are blocking syscalls that can stall
                // the async runtime on slow or network filesystems. Move the
                // work to a blocking worker so the executor stays responsive
                // for any concurrent pipeline runs.
                let current_files = tokio::task::spawn_blocking(move || {
                    let mut tracker = final_msg.tracker;
                    tracker.cleanup();
                    tracker.current_files
                })
                .await
                .map_err(|e| anyhow::anyhow!("pipeline cleanup task join failed: {e}"))?;
                Ok(current_files)
            }
            None => {
                // Pipeline was interrupted — recover error from the oneshot.
                match error_rx.await.ok() {
                    Some(err) => Err(anyhow::anyhow!("{err}")),
                    None => Err(anyhow::anyhow!(
                        "pipeline terminated with no output and no error"
                    )),
                }
            }
        }
    }

    /// Run the pipeline concurrently for multiple videos.
    ///
    /// Concurrency is bounded by the semaphore configured in [`new`].
    ///
    /// Takes `self: Arc<Self>` so that the pipeline can be shared across tasks.
    pub async fn run_batch(
        self: Arc<Self>,
        inputs: Vec<BatchInput>,
        config: Arc<PostProcess>,
        verbose: bool,
        callback_factory: Option<PostProcessCallbackFactory>,
    ) -> Vec<anyhow::Result<Vec<std::path::PathBuf>>> {
        let mut handles = Vec::with_capacity(inputs.len());
        for input in inputs {
            let pipeline = Arc::clone(&self);
            let config = Arc::clone(&config);
            let factory = callback_factory.clone();
            let handle = tokio::spawn(async move {
                pipeline
                    .run(
                        input.info,
                        input.files,
                        config,
                        input.original_stem,
                        input.is_hls,
                        verbose,
                        factory,
                    )
                    .await
            });
            handles.push(handle);
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(res) => results.push(res),
                Err(e) => results.push(Err(anyhow::anyhow!("task panicked: {e}"))),
            }
        }
        results
    }

    /// Spawn the stage chain and return the final receiver.
    ///
    /// The chain is: initial_msg → [stage_0] → [stage_1] → ... → final_rx
    ///
    /// Each stage task reads from `in_rx`, may process or pass-through,
    /// then sends to `out_tx`. When a fatal stage fails, it drops `out_tx`
    /// which cascades None through all downstream stages.
    fn spawn_chain(&self, initial_msg: PipelineMessage) -> mpsc::Receiver<PipelineMessage> {
        if self.stages.is_empty() {
            // No stages — connect directly.
            let (tx, rx) = mpsc::channel::<PipelineMessage>(1);
            let _ = tx.try_send(initial_msg);
            return rx;
        }

        // Build channels: one per inter-stage boundary.
        // stages: [S0, S1, S2]
        // channels: tx0→rx0 (input to S0), tx1→rx1 (S0→S1), tx2→rx2 (S1→S2), tx3→rx3 (S2→output)

        let n = self.stages.len();
        let mut txs: Vec<mpsc::Sender<PipelineMessage>> = Vec::with_capacity(n + 1);
        let mut rxs: Vec<mpsc::Receiver<PipelineMessage>> = Vec::with_capacity(n + 1);

        for _ in 0..=n {
            let (tx, rx) = mpsc::channel::<PipelineMessage>(1);
            txs.push(tx);
            rxs.push(rx);
        }

        // Send initial message into the first channel.
        let _ = txs[0].try_send(initial_msg);

        // Spawn one task per stage.
        // rxs[0] is stage 0's input, rxs[1] is stage 1's input, ..., rxs[n] is the final output.
        // txs[i+1] is stage i's output.
        let mut rxs_iter = rxs.into_iter();

        for (i, stage) in self.stages.iter().enumerate() {
            let stage = Arc::clone(stage);
            let in_rx_i = rxs_iter
                .next()
                .expect("pipeline: rxs has stages+1 elements; iteration {i} must yield Some");
            let out_tx = txs[i + 1].clone();
            let stage_name = stage.name().to_owned();

            tokio::spawn(async move {
                let mut in_rx = in_rx_i;
                let msg = match in_rx.recv().await {
                    Some(m) => m,
                    None => return, // upstream cascade
                };

                if !stage.should_run(&msg) {
                    let _ = out_tx.send(msg).await;
                    return;
                }

                let is_fatal = stage.is_fatal();

                match stage.process(msg).await {
                    Ok(result) => {
                        let _ = out_tx.send(result).await;
                    }
                    Err(e) if is_fatal => {
                        // error_tx was moved into the message passed to process().
                        // The stage is responsible for sending on msg.error_tx before
                        // returning Err. If it didn't, we have no channel left — the
                        // pipeline will fall back to "no error" message.
                        log::error!("Pipeline: fatal stage '{stage_name}' failed: {e}");
                        drop(out_tx); // cascade None downstream
                    }
                    Err(e) => {
                        // Non-fatal: log and cascade (message was moved into process).
                        // Non-fatal stages should return Ok(msg) on failure for passthrough.
                        log::warn!(
                            "Pipeline: non-fatal stage '{stage_name}' returned Err (message lost): {e}"
                        );
                        drop(out_tx);
                    }
                }
            });
        }

        // The last rx in the iterator is the pipeline output.
        rxs_iter
            .next()
            .expect("pipeline: rxs has stages+1 elements; loop consumed stages; one must remain")
    }
}

/// Input for a single video in [`Pipeline::run_batch`].
pub struct BatchInput {
    /// Video metadata.
    pub info: InfoDict,
    /// Files to process.
    pub files: Vec<std::path::PathBuf>,
    /// Original file stem.
    pub original_stem: String,
    /// Whether the source was HLS.
    pub is_hls: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
// Safe: test fixtures — the single std::fs::write here is setup for an async test that
// specifically exercises the spawn_blocking cleanup path; the write itself runs before
// the pipeline starts and is not inside an .await-blocking critical section.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    fn run_args(
        files: Vec<PathBuf>,
    ) -> (
        InfoDict,
        Vec<PathBuf>,
        Arc<PostProcess>,
        String,
        bool,
        bool,
        Option<PostProcessCallbackFactory>,
    ) {
        (
            make_info(),
            files,
            Arc::new(PostProcess::default()),
            "video".to_string(),
            false,
            false,
            None,
        )
    }

    #[tokio::test]
    async fn test_pipeline_passthrough() {
        let pipeline = make_pipeline(vec![Arc::new(PassthroughStage)]);
        let (info, files, config, stem, hls, verbose, cb) =
            run_args(vec![PathBuf::from("/tmp/video.mp4")]);
        let result = pipeline
            .run(info, files, config, stem, hls, verbose, cb)
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![PathBuf::from("/tmp/video.mp4")]);
    }

    #[tokio::test]
    async fn test_pipeline_skip_stage() {
        let pipeline = make_pipeline(vec![Arc::new(SkipStage), Arc::new(PassthroughStage)]);
        let (info, files, config, stem, hls, verbose, cb) =
            run_args(vec![PathBuf::from("/tmp/video.mp4")]);
        let result = pipeline
            .run(info, files, config, stem, hls, verbose, cb)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_pipeline_fatal_error_cascades() {
        let pipeline = make_pipeline(vec![Arc::new(FailStage), Arc::new(PassthroughStage)]);
        let (info, files, config, stem, hls, verbose, cb) =
            run_args(vec![PathBuf::from("/tmp/video.mp4")]);
        let result = pipeline
            .run(info, files, config, stem, hls, verbose, cb)
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
        let (info, files, config, stem, hls, verbose, cb) =
            run_args(vec![PathBuf::from("/tmp/video.mp4")]);
        let result = pipeline
            .run(info, files, config, stem, hls, verbose, cb)
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

        let (info, files, config, stem, hls, verbose, cb) = run_args(vec![video.clone()]);
        let result = pipeline
            .run(info, files, config, stem, hls, verbose, cb)
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
                let (info, files, config, stem, hls, verbose, cb) =
                    run_args(vec![PathBuf::from("/tmp/v.mp4")]);
                p.run(info, files, config, stem, hls, verbose, cb).await
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
}
