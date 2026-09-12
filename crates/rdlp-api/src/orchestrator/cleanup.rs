//! Cancel-path cleanup for orchestrator-owned sidecars.
//!
//! On a post-processing cancel the pipeline's `FileTracker` is the SOLE owner
//! of source-file deletion: it alone knows `keep_inputs` (#414), so it alone
//! may delete the files it was given. `Pipeline::run` joins every spawned
//! stage task before returning `PipelineError::Cancelled` (#560), so by the
//! time a caller observes `OrchestratorError::UserCancelled` every tracker
//! that run created has already been dropped — including one still doing
//! `FFmpeg` work when the cancel fired, which, before #560, `run` did not
//! wait for. This module deletes only what the pipeline never sees:
//! the downloaded thumbnail and the `.rdlp_state.json` session-state file.
//! Called on the PP-cancel path only — download-time cancel intentionally
//! retains resume state (see the #404 design spec).

use std::path::Path;

use super::Orchestrator;
use super::session_state::{self, SessionState};
use super::thumbnail::OwnedThumbnail;

/// The cancelled job's identity — just enough to locate its session-state
/// sidecar. Grouped so the function stays at two parameters as more
/// orchestrator-owned artifacts are added (per `limit-function-arguments`).
pub(super) struct CancelledJob<'a> {
    pub(super) output_dir: &'a Path,
    pub(super) output_to_stdout: bool,
    pub(super) title: &'a str,
}

/// Idempotently delete the orchestrator-owned artifacts of a cancelled job:
/// the session-state file and the downloaded thumbnail. Source files are
/// deliberately NOT handled here — see the module doc.
///
/// The session state and the thumbnail are `NotFound`-tolerant — they delete
/// unconditionally and absorb "already gone", which is atomic.
///
/// Every delete is best-effort — failures are logged, never propagated, so
/// cleanup cannot turn a cancel into a failure.
pub(super) async fn cleanup_cancelled_artifacts(
    job: &CancelledJob<'_>,
    thumbnail: Option<OwnedThumbnail>,
) {
    // Session state — skip in stdout mode (none is ever written there).
    if !job.output_to_stdout {
        let sanitized = Orchestrator::sanitize_filename(job.title);
        let state_path = session_state::single_video_state_path(job.output_dir, &sanitized);
        // SessionState::delete is itself NotFound-tolerant + best-effort, so no
        // exists()-guard is needed here.
        SessionState::delete(&state_path).await;
    }

    // Downloaded thumbnail. The token is proof rdlp created this file; a
    // borrowed run cannot produce one, so this path cannot reach a user file.
    if let Some(thumb) = thumbnail
        && let Err(e) = thumb.delete().await
    {
        // Best-effort: cleanup must not turn a cancel into a failure (#404).
        log::warn!("cleanup_cancelled_artifacts: failed to delete thumbnail: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn deletes_session_state_and_thumbnail() {
        let dir = TempDir::new().unwrap();
        let out = dir.path();
        let title = "My Video";
        let sanitized = Orchestrator::sanitize_filename(title);
        let state = out.join(format!("{sanitized}.rdlp_state.json"));
        let thumb = out.join("My Video.webp");
        tokio::fs::write(&state, b"x").await.unwrap();
        tokio::fs::write(&thumb, b"x").await.unwrap();

        cleanup_cancelled_artifacts(
            &CancelledJob {
                output_dir: out,
                output_to_stdout: false,
                title,
            },
            Some(OwnedThumbnail::for_test(thumb.clone())),
        )
        .await;

        assert!(!state.exists(), "session state must be deleted");
        assert!(!thumb.exists(), "thumbnail must be deleted");
    }

    /// The single-owner invariant (#560): a source file must survive
    /// `cleanup_cancelled_artifacts` regardless of `keep_inputs` — the
    /// function has no way to reach it at all, because only the pipeline's
    /// `FileTracker` (which knows `keep_inputs`, #414) may delete a source.
    #[tokio::test]
    async fn never_touches_a_source_file() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("borrowed-or-owned.mp4");
        tokio::fs::write(&source, b"x").await.unwrap();

        cleanup_cancelled_artifacts(
            &CancelledJob {
                output_dir: dir.path(),
                output_to_stdout: false,
                title: "T",
            },
            None,
        )
        .await;

        assert!(
            source.exists(),
            "cleanup_cancelled_artifacts must never delete a source file itself (#560)"
        );
    }

    #[tokio::test]
    async fn idempotent_when_absent() {
        let dir = TempDir::new().unwrap();
        // Nothing exists — must not panic or error.
        cleanup_cancelled_artifacts(
            &CancelledJob {
                output_dir: dir.path(),
                output_to_stdout: false,
                title: "Gone",
            },
            Some(OwnedThumbnail::for_test(dir.path().join("missing.webp"))),
        )
        .await;
    }

    #[tokio::test]
    async fn stdout_mode_skips_session_state() {
        let dir = TempDir::new().unwrap();
        let title = "Z";
        let sanitized = Orchestrator::sanitize_filename(title);
        let state = dir.path().join(format!("{sanitized}.rdlp_state.json"));
        tokio::fs::write(&state, b"x").await.unwrap();

        cleanup_cancelled_artifacts(
            &CancelledJob {
                output_dir: dir.path(),
                output_to_stdout: true,
                title,
            },
            None,
        )
        .await;

        assert!(state.exists(), "stdout mode must NOT delete session state");
    }

    #[tokio::test]
    async fn deletes_thumbnail_when_state_absent() {
        // The primary PP-cancel state: FileTracker::Drop already removed the
        // source download, but the thumbnail is still on disk. The thumbnail
        // must be deleted even though the state file is absent.
        let dir = TempDir::new().unwrap();
        let out = dir.path();
        let title = "Partial";
        let thumb = out.join("Partial.webp");
        tokio::fs::write(&thumb, b"x").await.unwrap();

        cleanup_cancelled_artifacts(
            &CancelledJob {
                output_dir: out,
                output_to_stdout: false,
                title,
            },
            Some(OwnedThumbnail::for_test(thumb.clone())),
        )
        .await;

        assert!(
            !thumb.exists(),
            "thumbnail must be deleted even when state absent"
        );
    }

    /// The keep-by-default invariant: a token that never reaches cleanup must
    /// leave the file alone. This is what makes the absence of a `Drop` impl
    /// safe — dropping the token is inert.
    #[tokio::test]
    async fn dropped_token_does_not_delete_the_thumbnail() {
        let dir = TempDir::new().unwrap();
        let thumb = dir.path().join("Kept.jpg");
        tokio::fs::write(&thumb, b"x").await.unwrap();

        let token = OwnedThumbnail::for_test(thumb.clone());
        drop(token);

        assert!(
            thumb.exists(),
            "dropping the token must not delete the thumbnail (keep-by-default)"
        );
    }

    /// `NotFound` is success: the postcondition ("no thumbnail here") holds.
    #[tokio::test]
    async fn deleting_an_absent_thumbnail_is_ok() {
        let dir = TempDir::new().unwrap();
        let token = OwnedThumbnail::for_test(dir.path().join("never-existed.jpg"));

        assert!(
            token.delete().await.is_ok(),
            "a missing thumbnail must not be reported as an error"
        );
    }

    /// The error-propagating arm. Only `NotFound` is absorbed; every other
    /// failure must surface so the cancel path can log it. Without this test,
    /// mutating `other => other` into `_ => Ok(())` — silently swallowing every
    /// real I/O failure — leaves the whole suite green.
    ///
    /// Also pins the path into the error text: the token has no accessor by
    /// design, so `delete` is the only place that can supply it, and that one
    /// log line is all an operator gets for a cleanup failure.
    #[tokio::test]
    async fn deleting_an_undeletable_thumbnail_reports_the_error() {
        let dir = TempDir::new().unwrap();
        // `remove_file` on a directory fails with a non-NotFound error.
        let not_a_file = dir.path().join("a-directory.jpg");
        tokio::fs::create_dir(&not_a_file).await.unwrap();

        let err = OwnedThumbnail::for_test(not_a_file.clone())
            .delete()
            .await
            .expect_err("a non-NotFound failure must not be absorbed");

        assert_ne!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "the original ErrorKind must survive so callers can still match on it"
        );
        assert!(
            err.to_string().contains("a-directory.jpg"),
            "the error text must name the thumbnail: {err}"
        );
        assert!(
            not_a_file.exists(),
            "a failed delete must leave the path alone"
        );
    }

    /// The best-effort contract at the seam: a thumbnail that fails to delete
    /// must not abort the rest of the cleanup. Without this, mutating the
    /// thumbnail block to `let _ = thumb.delete().await;` — or to an early
    /// `return` — leaves every other test green while session-state cleanup
    /// silently stops running.
    #[tokio::test]
    async fn a_failing_thumbnail_delete_does_not_abort_session_state_cleanup() {
        let dir = TempDir::new().unwrap();
        let out = dir.path();
        let title = "Resilient";

        // Undeletable thumbnail: remove_file on a directory fails.
        let undeletable = out.join("Resilient.jpg");
        tokio::fs::create_dir(&undeletable).await.unwrap();

        let state = out.join(format!(
            "{}.rdlp_state.json",
            Orchestrator::sanitize_filename(title)
        ));
        tokio::fs::write(&state, b"x").await.unwrap();

        cleanup_cancelled_artifacts(
            &CancelledJob {
                output_dir: out,
                output_to_stdout: false,
                title,
            },
            Some(OwnedThumbnail::for_test(undeletable.clone())),
        )
        .await;

        assert!(
            !state.exists(),
            "session-state cleanup must still run after a thumbnail delete fails"
        );
        assert!(
            undeletable.exists(),
            "the undeletable thumbnail itself must be left alone"
        );
    }
}
