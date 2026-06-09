//! Cancel-path cleanup for orchestrator-owned sidecars.
//!
//! On a post-processing cancel the pipeline's `FileTracker` reclaims its own
//! files (the source download via `temp_files`), but the orchestrator owns two
//! artifacts the pipeline never sees: the downloaded thumbnail and the
//! `.rdlp_state.json` session-state file. This module deletes them. Called on
//! the PP-cancel path only — download-time cancel intentionally retains resume
//! state (see the #404 design spec).

use std::path::{Path, PathBuf};

use super::Orchestrator;
use super::session_state::{self, SessionState};

/// Idempotently delete the orchestrator-owned artifacts of a cancelled job:
/// the session-state file, the downloaded thumbnail, and (defensively) any
/// source download files. Every delete is `exists()`-guarded and best-effort —
/// failures are logged, never propagated, so cleanup cannot turn a cancel into
/// a failure.
// Wired into the PP-cancel arm in Task 4 (#404). `expect` (not `allow`) so the
// lint self-clears — it becomes an error the moment the caller is added.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) async fn cleanup_cancelled_artifacts(
    output_dir: &Path,
    output_to_stdout: bool,
    title: &str,
    source_files: &[PathBuf],
    thumbnail: Option<&Path>,
) {
    // Session state — skip in stdout mode (none is ever written there).
    if !output_to_stdout {
        let sanitized = Orchestrator::sanitize_filename(title);
        let state_path = session_state::single_video_state_path(output_dir, &sanitized);
        // SessionState::delete is itself NotFound-tolerant + best-effort, so no
        // exists()-guard is needed here (unlike the thumbnail/source blocks below).
        SessionState::delete(&state_path).await;
    }

    // Downloaded thumbnail (exact captured path).
    if let Some(thumb) = thumbnail
        && thumb.exists()
        && let Err(e) = tokio::fs::remove_file(thumb).await
    {
        log::warn!(
            "cleanup_cancelled_artifacts: failed to delete thumbnail {}: {e}",
            thumb.display()
        );
    }

    // Source download files — defense-in-depth; normally already removed by the
    // pipeline's FileTracker::Drop (temp_files).
    for file in source_files {
        if file.exists()
            && let Err(e) = tokio::fs::remove_file(file).await
        {
            log::warn!(
                "cleanup_cancelled_artifacts: failed to delete source {}: {e}",
                file.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn deletes_session_state_thumbnail_and_source() {
        let dir = TempDir::new().unwrap();
        let out = dir.path();
        let title = "My Video";
        let sanitized = Orchestrator::sanitize_filename(title);
        let state = out.join(format!("{sanitized}.rdlp_state.json"));
        let thumb = out.join("My Video.webp");
        let source = out.join("My Video [id].mp4");
        for p in [&state, &thumb, &source] {
            tokio::fs::write(p, b"x").await.unwrap();
        }

        cleanup_cancelled_artifacts(
            out,
            false,
            title,
            std::slice::from_ref(&source),
            Some(thumb.as_path()),
        )
        .await;

        assert!(!state.exists(), "session state must be deleted");
        assert!(!thumb.exists(), "thumbnail must be deleted");
        assert!(!source.exists(), "source download must be deleted");
    }

    #[tokio::test]
    async fn idempotent_when_absent() {
        let dir = TempDir::new().unwrap();
        // Nothing exists — must not panic or error.
        cleanup_cancelled_artifacts(
            dir.path(),
            false,
            "Gone",
            &[dir.path().join("missing.mp4")],
            Some(&dir.path().join("missing.webp")),
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

        cleanup_cancelled_artifacts(dir.path(), true, title, &[], None).await;

        assert!(state.exists(), "stdout mode must NOT delete session state");
    }

    #[tokio::test]
    async fn deletes_thumbnail_when_source_and_state_absent() {
        // The primary PP-cancel state: FileTracker::Drop already removed the
        // source download, but the thumbnail is still on disk. The thumbnail
        // must be deleted even though the state file and source are absent.
        let dir = TempDir::new().unwrap();
        let out = dir.path();
        let title = "Partial";
        let thumb = out.join("Partial.webp");
        tokio::fs::write(&thumb, b"x").await.unwrap();
        let absent_source = out.join("Partial [id].mp4"); // never created

        cleanup_cancelled_artifacts(
            out,
            false,
            title,
            std::slice::from_ref(&absent_source),
            Some(thumb.as_path()),
        )
        .await;

        assert!(
            !thumb.exists(),
            "thumbnail must be deleted even when source/state absent"
        );
    }
}
