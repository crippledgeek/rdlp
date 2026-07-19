//! Owned-path tracking for parallel-download chunk files.
//!
//! Replaces the pattern-sweep deletion [`super::chunk_name::ChunkSet`] used to
//! perform (`ChunkSet::sweep`, removed): that scanned the temp directory for
//! names matching this download's grammar and deleted every match. The
//! grammar alone cannot distinguish "a chunk this run wrote" from "a chunk a
//! concurrent process with the same `download_id` wrote" — `DOWNLOAD_ID_COUNTER`
//! (`parallel.rs`) is a per-process atomic starting at 0, so two concurrent
//! rdlp processes downloading the same filename into the same directory both
//! use `download_id == 0`, and a pattern sweep in process A would delete
//! process B's live chunk files. This is the #558 defect class
//! (`cleanup_leftover_segments`'s directory-pattern sweep) reached from a
//! different starting point; see `scripts/check-no-dir-sweep-delete.sh`.
//!
//! [`ChunkLedger`] closes that hole by construction: nothing is ever deleted
//! that wasn't first registered by this run, at the moment this run computed
//! the path — so cleanup can only ever remove paths this attempt is
//! authoritatively known to have created (or be about to create).

use log::{debug, warn};
use std::path::PathBuf;
use std::sync::Mutex;

/// The set of chunk file paths one download attempt has created (or is about
/// to create), carrying deletion authority over exactly those paths.
///
/// Analogous in spirit to `rdlp-api`'s `OwnedThumbnail` (#556): possession of
/// a registration is what authorizes deletion, so designation and permission
/// travel together instead of a cleanup path re-deriving "which paths are
/// mine" by re-scanning the filesystem.
///
/// Uses a `std::sync::Mutex` rather than `tokio::sync::Mutex`: the adaptive
/// downloader registers a path once per chunk from inside a `try_unfold`
/// closure shared across concurrently-polled futures, so registration must
/// be callable without threading `&mut self` through the pipeline — but each
/// critical section is a single `Vec::push`, never held across an `.await`,
/// so the synchronous primitive is both sufficient and cheaper.
#[derive(Debug, Default)]
pub(super) struct ChunkLedger {
    paths: Mutex<Vec<PathBuf>>,
}

impl ChunkLedger {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Register a chunk path this run is about to write.
    ///
    /// Must be called BEFORE the chunk's download starts, not after it
    /// succeeds — a chunk that fails mid-write still leaves a partial file
    /// on disk, and that file must still be reachable by cleanup.
    pub(super) fn register(&self, path: PathBuf) {
        self.paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(path);
    }

    /// Delete every path registered so far, consuming the registration list.
    ///
    /// A `NotFound` on removal is not an error (the file is already gone,
    /// which is the desired end state — e.g. a chunk already drained into
    /// the merged output); any other I/O error is logged at `warn!` but does
    /// not stop cleanup of the remaining paths, matching the severity and
    /// resilience the prior directory sweep provided.
    pub(super) async fn cleanup(&self) {
        let paths = std::mem::take(
            &mut *self
                .paths
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let total = paths.len();
        let mut deleted = 0usize;
        for path in &paths {
            match tokio::fs::remove_file(path).await {
                Ok(()) => deleted += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!(path:? = path; "Failed to delete chunk file: {e}"),
            }
        }
        debug!(deleted = deleted, total = total; "Cleaned up owned chunk files after failed download");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cleanup_deletes_only_registered_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = ChunkLedger::new();

        let owned_a = dir.path().join("owned_a");
        let owned_b = dir.path().join("owned_b");
        tokio::fs::write(&owned_a, b"a").await.expect("write a");
        tokio::fs::write(&owned_b, b"b").await.expect("write b");
        ledger.register(owned_a.clone());
        ledger.register(owned_b.clone());

        // A foreign file this ledger never registered, even though nothing
        // here distinguishes its *name* from an owned chunk's — the point of
        // this type is that name alone is irrelevant to deletion.
        let foreign = dir.path().join("foreign");
        tokio::fs::write(&foreign, b"f")
            .await
            .expect("write foreign");

        ledger.cleanup().await;

        assert!(tokio::fs::metadata(&owned_a).await.is_err());
        assert!(tokio::fs::metadata(&owned_b).await.is_err());
        assert!(
            tokio::fs::metadata(&foreign).await.is_ok(),
            "an unregistered file must survive cleanup regardless of its name"
        );
    }

    #[tokio::test]
    async fn cleanup_of_already_missing_registered_path_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = ChunkLedger::new();
        // Registered but never actually written (e.g. the write failed before
        // any bytes landed) — cleanup must tolerate this silently.
        ledger.register(dir.path().join("never-written"));

        ledger.cleanup().await;
    }

    #[tokio::test]
    async fn cleanup_with_no_registrations_is_a_no_op() {
        ChunkLedger::new().cleanup().await;
    }

    /// This is the whole point of replacing `ChunkSet::sweep` with a ledger.
    ///
    /// `DOWNLOAD_ID_COUNTER` (`parallel.rs`) is a per-*process* atomic
    /// starting at 0, so two concurrent rdlp processes downloading the same
    /// filename into the same directory can both compute `download_id == 0`
    /// — at which point their chunk files are named identically
    /// (`ChunkSet::path_in` depends only on filename + `download_id` + kind +
    /// `chunk_id`). The removed `ChunkSet::sweep` deleted every directory
    /// entry matching that shared name grammar, so process A's sweep would
    /// also delete process B's live, in-progress chunk file of the same
    /// name. A `ChunkLedger` cannot make that mistake: it deletes only the
    /// paths THIS run explicitly registered, so a file this run never
    /// registered survives even when its name is indistinguishable from an
    /// owned chunk's.
    #[tokio::test]
    async fn cleanup_survives_a_foreign_file_matching_this_run_exact_grammar_and_download_id() {
        use super::super::chunk_name::{ChunkKind, ChunkSet};

        let dir = tempfile::tempdir().expect("tempdir");
        // Same filename, same download_id, same kind for both "this run"
        // and the "foreign" file below — standing in for a second, real,
        // concurrent rdlp process that landed on the same per-process
        // download id.
        let shared_set =
            ChunkSet::for_attempt("Title.mp4", 0, ChunkKind::Fresh).expect("valid filename");

        let this_run = ChunkLedger::new();
        let owned_chunk = shared_set.path_in(dir.path(), 0);
        tokio::fs::write(&owned_chunk, b"mine")
            .await
            .expect("write this run's chunk 0");
        this_run.register(owned_chunk.clone());

        // The foreign process's chunk 1: identical grammar (filename,
        // download_id, kind), never registered with THIS run's ledger.
        let foreign_chunk = shared_set.path_in(dir.path(), 1);
        tokio::fs::write(&foreign_chunk, b"theirs")
            .await
            .expect("write the foreign process's chunk 1");

        this_run.cleanup().await;

        assert!(
            tokio::fs::metadata(&owned_chunk).await.is_err(),
            "this run's own registered chunk must be deleted"
        );
        assert!(
            tokio::fs::metadata(&foreign_chunk).await.is_ok(),
            "a same-grammar, same-download-id chunk this run never \
             registered (standing in for a concurrent process's live chunk) \
             must survive cleanup — the removed ChunkSet::sweep could not \
             guarantee this"
        );
    }
}
