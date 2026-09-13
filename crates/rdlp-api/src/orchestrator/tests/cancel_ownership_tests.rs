//! Single-owner cancel cleanup (#560).
//!
//! `cleanup_cancelled_artifacts` used to delete `source_files` itself,
//! duplicating `FileTracker::Drop` under weaker rules (no borrowed-input
//! exclusion). Neither of its two callers ever passed it a borrowed path, so
//! that loop was never reachable with one — these tests are standing
//! contract guards, not regression pins. They drive the REAL cancel path
//! through `run_postprocessing` — pre-cancelling the token before the
//! pipeline has done any work — and check who actually decides the source's
//! fate: the pipeline's `FileTracker`, which respects `keep_inputs` (#414),
//! not the orchestrator. See `pipeline/tests.rs`'s
//! `cancel_mid_slow_stage_reclaims_owned_source_before_run_returns` for the
//! regression pin proper — the race the pre-#560 `Pipeline::run` had, where
//! `Cancelled` could be returned before an in-flight stage had actually
//! dropped its tracker.

use super::*;
use crate::orchestrator::errors::OrchestratorError;
use crate::orchestrator::test_support::test_info_with_formats;
use tempfile::TempDir;

fn test_info() -> InfoDict {
    test_info_with_formats(Vec::new())
}

/// A borrowed (user-owned) input must survive a post-processing cancel.
///
/// Not a regression guard (neither caller of `cleanup_cancelled_artifacts`
/// ever passed it a borrowed path, so the pre-#560 loop was never reachable
/// with one — see the module doc). This is a standing contract guard: it
/// pins that the REAL cancel path (pipeline → `FileTracker::Drop`, joined by
/// `Pipeline::run` before it returns `Cancelled`, #560) is what decides
/// ownership, and it already refuses to delete a borrowed input (#414).
#[tokio::test]
async fn borrowed_input_survives_a_postprocessing_cancel() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("users-own-clip.mp4");
    tokio::fs::write(&source, b"not really a video")
        .await
        .unwrap();

    let orchestrator = create_test_orchestrator();
    assert!(
        orchestrator.pipeline.is_ready(),
        "precondition: this test needs a real FFmpeg pipeline on the test \
         machine, or a Ready-but-cancelled run degrades to Ok(files) instead \
         of exercising the cancel path at all"
    );
    // Cancel before the pipeline can do any work — the earliest possible
    // cancel point, and the one Phase 1 identified as the only gap worth
    // checking: does ownership (the `FileTracker`) exist before this fires?
    orchestrator.cancel_token.cancel();

    let result = orchestrator
        .run_postprocessing(&test_info(), vec![source.clone()], false, true)
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::UserCancelled)),
        "expected UserCancelled, got {result:?}"
    );
    assert!(
        source.exists(),
        "a borrowed (keep_inputs=true) source must survive a cancel"
    );
}

/// The mirror case: an OWNED input is reclaimed on cancel by the pipeline's
/// `FileTracker`, with no help from the orchestrator's cleanup step. Proves
/// the source is not simply never reachable in either direction.
#[tokio::test]
async fn owned_input_is_reclaimed_by_the_pipeline_on_cancel() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("rdlp-downloaded.mp4");
    tokio::fs::write(&source, b"not really a video")
        .await
        .unwrap();

    let orchestrator = create_test_orchestrator();
    assert!(
        orchestrator.pipeline.is_ready(),
        "precondition: this test needs a real FFmpeg pipeline on the test \
         machine, or a Ready-but-cancelled run degrades to Ok(files) instead \
         of exercising the cancel path at all"
    );
    orchestrator.cancel_token.cancel();

    let result = orchestrator
        .run_postprocessing(&test_info(), vec![source.clone()], false, false)
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::UserCancelled)),
        "expected UserCancelled, got {result:?}"
    );
    assert!(
        !source.exists(),
        "an owned (keep_inputs=false) source must be reclaimed on cancel"
    );
}
