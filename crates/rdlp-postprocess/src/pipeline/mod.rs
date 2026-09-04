//! Channel-based post-processing pipeline.
//!
//! Each stage is a `tokio::spawn` task connected by bounded `mpsc` channels.
//! [`FileTracker`] owns all file lifecycle decisions — no stage ever deletes
//! files directly. [`TempRegistry`] handles crash cleanup.
//!
//! # Lint allowances
//!
//! - `clippy::indexing_slicing`: `txs[0]` and `txs[i + 1]` are accessed only
//!   within iteration over `stages`, where `txs` has `stages.len() + 1` elements.
//! - `clippy::expect_used`: `rxs_iter.next().expect(…)` at the end of the
//!   channel-construction loop is guaranteed by construction — `rxs` contains
//!   `stages.len() + 1` elements.
//! - `clippy::literal_string_with_formatting_args`: `{i}` in the expect string
//!   is documentation context, not a formatting argument.

#![allow(
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::literal_string_with_formatting_args,
    clippy::unnecessary_literal_bound,  // `fn name() -> &str` on a trait method returning literals
    clippy::option_if_let_else,         // map_or_else refactors reduce readability here
)]

pub mod registry;
pub mod sidecar;
pub mod stages;
pub mod tracker;

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use rdlp_core::PostProcessCallbackFactory;
use rdlp_types::InfoDict;
use rdlp_types::PostProcess;

pub use registry::TempRegistry;
pub use sidecar::{DiscoveredSidecar, SidecarOwnership};
pub use tracker::FileTracker;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can be produced by pipeline stages.
#[derive(Debug, Error, Clone)]
pub enum PipelineError {
    /// The pipeline was cancelled via its `CancellationToken`.
    ///
    /// Surfaced when `Pipeline::run`'s final receiver closes AND the token
    /// has been cancelled — the cascade of dropped `out_tx`s in `spawn_chain`
    /// stage tasks reaches the final receiver, and the post-loop check sees
    /// `token.is_cancelled() == true`.
    ///
    /// This is the **only** typed pipeline error. A stage failure is not one:
    /// since #632 the runner returns the stage's own `anyhow::Error` verbatim,
    /// context chain intact, rather than restringifying it into a variant. So
    /// call sites (notably the orchestrator at
    /// `crates/rdlp-api/src/orchestrator/postprocess.rs`) distinguish
    /// cancellation via `anyhow::Error::downcast_ref` — matching this variant
    /// means "abandoned on user cancel", anything else means a real failure —
    /// and propagate it as `OrchestratorError::UserCancelled` rather than the
    /// warn-and-fallback path used for stage failures. A former `StageFailure`
    /// variant was deleted in #632: nothing could construct it any more, and a
    /// variant that only test doubles produce is how the original bug hid.
    #[error("pipeline cancelled by token")]
    Cancelled,
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
    /// Enable verbose `FFmpeg` logging in stages.
    pub verbose: bool,
    /// Factory for creating per-stage progress callbacks.
    pub callback_factory: Option<PostProcessCallbackFactory>,
    /// Non-fatal warnings accumulated by stages during processing.
    pub warnings: Vec<String>,
    /// Encoding tool tag set by the primary content-creating stage (recode,
    /// audio extract, normalize). Pass-through stages (remux, metadata,
    /// thumbnail) propagate this to the output file's `encoding_tool`
    /// metadata instead of stamping their own name. `None` means no
    /// prior stage set it — pass-through stages use their own name.
    pub encoding_tool: Option<String>,
    /// Job-scoped cancellation token. Fatal `FFmpeg` stages clone this into their
    /// blocking work so an in-progress encode aborts promptly.
    pub cancel: CancellationToken,
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

/// Per-run pipeline options. Bundles the boolean flags so call sites are
/// self-documenting and the flags can't be positionally transposed.
#[derive(Debug, Clone, Copy)]
pub struct PipelineRunOptions {
    /// Borrow caller-supplied inputs (never delete them) — for a pre-existing
    /// local file rather than a file rdlp downloaded (#414).
    pub keep_inputs: bool,
    /// The input is an HLS download (drives `RemuxStage`).
    pub is_hls: bool,
    /// Verbose logging.
    pub verbose: bool,
}

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
    ///
    /// `opts.keep_inputs = true` constructs a borrowing [`FileTracker`] that never
    /// deletes the input files (success or cancel). Use this for user-supplied
    /// source files (`process_local_file`). `opts.keep_inputs = false` (the default
    /// everywhere else) gives the pipeline full ownership — inputs are deleted
    /// after successful processing.
    ///
    /// # Errors
    ///
    /// Returns the failing stage's own error, with its `anyhow` context chain
    /// intact, if a fatal stage fails (merge, audio extract, normalize, remux,
    /// or recode). Returns [`PipelineError::Cancelled`] — the one typed
    /// variant, matchable via `downcast_ref` — when the token was cancelled.
    /// Non-fatal stages (subtitle, metadata, thumbnail, fixup) log warnings
    /// and continue; one that returns `Err` anyway loses the message and so
    /// ends the run, surfacing its own cause.
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        info: InfoDict,
        files: Vec<std::path::PathBuf>,
        opts: PipelineRunOptions,
        config: Arc<PostProcess>,
        original_stem: String,
        callback_factory: Option<PostProcessCallbackFactory>,
        cancel: Option<CancellationToken>,
    ) -> anyhow::Result<Vec<std::path::PathBuf>> {
        let _permit =
            self.concurrency.acquire().await.map_err(|_| {
                anyhow::anyhow!("pipeline concurrency semaphore closed unexpectedly")
            })?;

        let tracker = if opts.keep_inputs {
            FileTracker::new_borrowing(files, Arc::clone(&self.temp_registry))
        } else {
            FileTracker::new(files, Arc::clone(&self.temp_registry))
        };

        // The error channel is owned by the stage *runner*, never by the
        // message. A stage propagating with `?` — which is every stage — has
        // no way to send on a channel that was moved into the message it just
        // consumed, so the previous `msg.error_tx` contract was one no stage
        // could satisfy and none did: every fatal failure reached the user as
        // "pipeline terminated with no output and no error" (#632). Capacity
        // 1 with `try_send`: the first fatal error wins and later ones are
        // dropped, which is the same first-error-wins semantics the oneshot
        // intended.
        let (error_tx, mut error_rx) = mpsc::channel::<anyhow::Error>(1);

        // `None` callers get a fresh token that nobody else holds — zero-cost.
        // The isolation guarantee is load-bearing: a `None` caller can never
        // observe a spurious cancel because the locally-created token is held
        // exclusively by this function's scope (cloned only into the per-stage
        // tasks below; never escapes outward via channels or shared state).
        // A future refactor that hands the local token to a sibling supervisor
        // task would silently break this property — keep it scope-local.
        let token = cancel.unwrap_or_default();

        let msg = PipelineMessage {
            info,
            tracker,
            config,
            original_stem,
            is_hls: opts.is_hls,
            verbose: opts.verbose,
            callback_factory,
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: token.clone(),
        };

        // Build channel chain: one mpsc(1) between consecutive stages.
        // first_tx → stage_0 → stage_1 → ... → stage_N → final_rx
        let mut final_rx = self.spawn_chain(msg, &token, error_tx);

        // Await the final message.
        let Some(final_msg) = final_rx.recv().await else {
            // If the cascade ended because the token was cancelled, surface that
            // distinct from "pipeline terminated with no output" (which is a bug
            // scenario — every stage cascaded None without error).
            if token.is_cancelled() {
                return Err(PipelineError::Cancelled.into());
            }
            // Pipeline was interrupted — recover the fatal stage's error.
            // Every sender is dropped by now (the cascade that produced the
            // `None` above is what drops them), so `recv` resolves immediately
            // rather than waiting. The error is returned as-is, preserving the
            // `anyhow` chain for `classify_pipeline_err`'s `{e:#}` flatten.
            return match error_rx.recv().await {
                Some(err) => Err(err),
                // Genuinely no error: no stage produced one. Since both the
                // fatal and non-fatal `Err` arms now send, the only way to
                // reach this is a stage task that died without returning —
                // i.e. panicked. Not a stand-in for a lost error any more.
                None => Err(anyhow::anyhow!(
                    "pipeline terminated with no output and no error"
                )),
            };
        };

        // FileTracker::cleanup performs N × std::fs::remove_file (temp_files)
        // and commits. It no longer renames survivors — the returned files are
        // temp-named (*.rdlp-tmp-{uuid}.*); the orchestrator does the single
        // final rename to the clean name (#406 Option X). Run on a blocking
        // worker so the blocking remove_file syscalls don't stall the runtime.
        let current_files = tokio::task::spawn_blocking(move || {
            let mut tracker = final_msg.tracker;
            tracker.cleanup();
            // `cleanup()` set `committed = true`, disarming the cancel-cleanup
            // `Drop`. `FileTracker` now implements `Drop`, so the surviving
            // temp-named files can't be moved out of the field directly — take them,
            // leaving the dropped tracker empty (and already committed).
            std::mem::take(&mut tracker.current_files)
        })
        .await
        .map_err(|e| anyhow::anyhow!("pipeline cleanup task join failed: {e}"))?;
        Ok(current_files)
    }

    /// Run the pipeline concurrently for multiple videos.
    ///
    /// Concurrency is bounded by the semaphore configured in [`Self::new`].
    ///
    /// Takes `self: Arc<Self>` so that the pipeline can be shared across tasks.
    pub async fn run_batch(
        self: Arc<Self>,
        inputs: Vec<BatchInput>,
        config: Arc<PostProcess>,
        verbose: bool,
        callback_factory: Option<PostProcessCallbackFactory>,
        cancel: Option<CancellationToken>,
    ) -> Vec<anyhow::Result<Vec<std::path::PathBuf>>> {
        let mut handles = Vec::with_capacity(inputs.len());
        for input in inputs {
            let pipeline = Arc::clone(&self);
            let config = Arc::clone(&config);
            let factory = callback_factory.clone();
            let cancel_clone = cancel.clone();
            let handle = tokio::spawn(async move {
                pipeline
                    .run(
                        input.info,
                        input.files,
                        PipelineRunOptions {
                            keep_inputs: false,
                            is_hls: input.is_hls,
                            verbose,
                        },
                        config,
                        input.original_stem,
                        factory,
                        cancel_clone,
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
    /// The chain is: `initial_msg` → [`stage_0`] → [`stage_1`] → ... → `final_rx`
    ///
    /// Each stage task reads from `in_rx`, may process or pass-through,
    /// then sends to `out_tx`. When a fatal stage fails, it drops `out_tx`
    /// which cascades None through all downstream stages.
    ///
    /// `error_tx` is taken **by value on purpose** — clippy's
    /// `needless_pass_by_value` suggestion to borrow it is wrong here, and
    /// following it would hang the pipeline. The caller awaits
    /// `error_rx.recv()`, which only resolves to `None` once *every* sender is
    /// dropped; consuming the sender here means the last non-task handle dies
    /// with this call, so a run that produced no error terminates instead of
    /// waiting forever on a handle `run` still owns. Passing `&Sender` would
    /// leave that handle alive in `run`'s frame and make the no-error path
    /// block indefinitely. Consuming it makes that structural rather than
    /// resting on a `drop(error_tx)` line a later edit could delete.
    #[allow(clippy::needless_pass_by_value)]
    fn spawn_chain(
        &self,
        initial_msg: PipelineMessage,
        token: &CancellationToken,
        error_tx: mpsc::Sender<anyhow::Error>,
    ) -> mpsc::Receiver<PipelineMessage> {
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
            let stage_token = token.clone();
            let stage_error_tx = error_tx.clone();

            tokio::spawn(async move {
                let mut in_rx = in_rx_i;
                // Cancellation granularity: this select! fires BETWEEN stages.
                // A stage already executing inside `process()` (e.g. FFmpeg
                // work in MergeStage / RecodeStage) will not observe the
                // token until it returns — `spawn_blocking` workers cannot
                // be interrupted, and FFmpeg's `AVIOInterruptCB` is IO-layer
                // only (does not interrupt codec encode/decode loops). Mid-
                // stage interruption is a separate, deferred follow-up.
                //
                // `biased;` ensures cancel observation wins ties — without
                // it, tokio::select!'s pseudo-random arm selection could
                // process one more message after the cancel fired. Same
                // pattern used in `crates/rdlp-downloader/src/fragments.rs`
                // for the per-fragment fetch race.
                let msg = tokio::select! {
                    biased;
                    () = stage_token.cancelled() => {
                        drop(out_tx); // cascade None downstream — mirrors fatal-error path
                        return;
                    }
                    result = in_rx.recv() => match result {
                        Some(m) => m,
                        None => return, // upstream cascade (existing behavior)
                    },
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
                        // `{e:#}` (not `{e}`): the flatten preserves the whole
                        // `anyhow` context chain. `{e}` printed only the
                        // outermost `.context(...)` — "recode stage failed" —
                        // and dropped the cause that names the actual problem
                        // ("avi cannot represent hevc video"). See #632, and
                        // `classify_pipeline_err`, which already flattens.
                        log::error!("Pipeline: fatal stage '{stage_name}' failed: {e:#}");
                        // The runner sends, so this cannot be forgotten by a
                        // stage using `?`. Capacity 1: a full channel means an
                        // earlier fatal error already won, and that one is the
                        // one worth reporting.
                        let _ = stage_error_tx.try_send(e);
                        drop(out_tx); // cascade None downstream
                    }
                    Err(e) => {
                        // Non-fatal stages are contracted to return Ok(msg) on
                        // failure so the pipeline passes through. One that
                        // returns Err anyway has consumed the message, so the
                        // run cannot continue — it ends here regardless of the
                        // stage's "non-fatal" status. Send the cause for the
                        // same reason the fatal arm does: otherwise this path
                        // reproduces #632 exactly, terminating the pipeline
                        // with "no output and no error" while holding the real
                        // error in hand. No shipped non-fatal stage does this
                        // today; `NonFatalErrStage` keeps it honest.
                        log::warn!(
                            "Pipeline: non-fatal stage '{stage_name}' returned Err (message lost): {e:#}"
                        );
                        let _ = stage_error_tx.try_send(e);
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
#[allow(clippy::disallowed_methods)]
mod tests;
