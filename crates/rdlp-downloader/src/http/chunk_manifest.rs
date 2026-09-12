//! On-disk record of which parallel-download chunks one attempt has
//! completed, and at what length (#675).
//!
//! Before this module existed, resume recovery (`rdlp-api`'s
//! `detect_chunk_files`) trusted any non-empty chunk file at the next
//! expected id: a chunk truncated by an interrupted write was merged at its
//! truncated length, and a gap silently ended the recoverable prefix with no
//! record of what was lost beyond it. [`ChunkManifest`] closes that by
//! recording, as each chunk completes, the exact byte length the writer
//! produced — so recovery can tell "this chunk is done" from "this chunk
//! exists" the same way `rdlp-downloader`'s DASH resume state
//! (`dash::state`) already distinguishes a completed segment index from an
//! on-disk file.
//!
//! [`ChunkManifestTracker`] is the write-side half: a cheaply-cloneable
//! handle threaded through the same `try_unfold`/`buffer_unordered` closures
//! [`super::chunk_ledger::ChunkLedger`] already threads through, so
//! registering a completed chunk costs one more call at the same call site.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::{debug, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::chunk_name::ChunkKind;

/// Current schema version of the on-disk chunk manifest. Bump on
/// incompatible field changes; [`ChunkManifest::load_matching`] rejects a
/// mismatch rather than guessing at a migration.
pub const CHUNK_MANIFEST_VERSION: u32 = 1;

/// Persisted record of one download attempt's completed chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkManifest {
    /// Schema version. A mismatch forces the manifest to be treated as
    /// absent (unverifiable) rather than parsed under stale assumptions.
    pub schema_version: u32,
    /// The attempt this manifest belongs to — matched against the
    /// `download_id` a recovering scan is currently probing, so a manifest
    /// left behind by an unrelated attempt is never mistaken for this one's.
    pub download_id: u64,
    /// Fresh vs. Resume — matched the same way as `download_id`.
    pub kind: ChunkKind,
    /// The plan's total size for this attempt, in bytes.
    pub total_size: u64,
    /// `chunk_id -> exact byte length written`, recorded only once that
    /// chunk's download has fully succeeded.
    pub completed: BTreeMap<u64, u64>,
}

impl ChunkManifest {
    /// Construct a fresh, empty manifest for one download attempt.
    ///
    /// Deliberately does NOT take a byte offset: nothing on the recovery
    /// side ever reads one (the file's own length verification is
    /// per-chunk, not offset-relative), and an earlier version of this type
    /// carried one anyway "for parity with `DashDownloadState`" — a
    /// duplicated field with no reader is dead weight, not parity.
    #[must_use]
    pub const fn new(download_id: u64, kind: ChunkKind, total_size: u64) -> Self {
        Self {
            schema_version: CHUNK_MANIFEST_VERSION,
            download_id,
            kind,
            total_size,
            completed: BTreeMap::new(),
        }
    }

    /// Record that `chunk_id` finished at exactly `len` bytes. Idempotent —
    /// a retried save after a transient failure re-records the same value.
    pub fn record_completed(&mut self, chunk_id: u64, len: u64) {
        self.completed.insert(chunk_id, len);
    }

    /// The recorded length for `chunk_id`, if this manifest ever recorded it
    /// as complete.
    #[must_use]
    pub fn recorded_len(&self, chunk_id: u64) -> Option<u64> {
        self.completed.get(&chunk_id).copied()
    }

    /// Persist this manifest to `path` (write-temp-in-same-dir + rename via
    /// `crate::atomic::atomic_write_json`).
    ///
    /// # Errors
    /// Returns the underlying I/O error from the write or rename.
    pub async fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::atomic::atomic_write_json(path, self.clone()).await
    }

    /// Load a manifest from `path`, accepting it only if it matches
    /// `download_id` and `kind`, carries the current schema version, and its
    /// highest recorded chunk id is within [`crate::chunking::CHUNK_SCAN_CEILING`].
    ///
    /// The ceiling check is a security boundary, not a correctness nicety:
    /// this file is attacker-influenceable by anyone who can write to the
    /// download's output directory (no secret gates it), and its
    /// `completed` keys drive recovery's scan range directly. A single
    /// entry near `u64::MAX` would otherwise make that scan effectively
    /// unbounded — no real plan can produce an id anywhere near this
    /// ceiling (see `CHUNK_SCAN_CEILING`'s doc), so rejecting one here is
    /// pure loss-prevention with no cost to a legitimate manifest.
    ///
    /// Returns `None` for: missing file, parse failure, schema mismatch,
    /// `download_id` mismatch, `kind` mismatch, or an oversized max chunk id
    /// — every one of these means the manifest cannot be trusted to
    /// describe the chunk set being probed, so the caller must treat the
    /// set as unverifiable rather than guess at a partial match.
    #[must_use]
    pub async fn load_matching(path: &Path, download_id: u64, kind: ChunkKind) -> Option<Self> {
        let manifest: Self = crate::atomic::read_json_sidecar(path).await?;
        if manifest.schema_version != CHUNK_MANIFEST_VERSION
            || manifest.download_id != download_id
            || manifest.kind != kind
        {
            return None;
        }
        if let Some(&max_id) = manifest.completed.keys().next_back()
            && max_id > crate::chunking::CHUNK_SCAN_CEILING
        {
            debug!(
                max_id,
                ceiling = crate::chunking::CHUNK_SCAN_CEILING;
                "Rejecting chunk manifest at {}: max recorded chunk id {max_id} exceeds the \
                 scan ceiling of {}",
                path.display(),
                crate::chunking::CHUNK_SCAN_CEILING
            );
            return None;
        }
        Some(manifest)
    }
}

/// Write-side handle to a [`ChunkManifest`] under construction, threaded
/// through the same concurrently-polled chunk-download futures
/// [`super::chunk_ledger::ChunkLedger`] is threaded through — but kept a
/// separate type rather than folded into `ChunkLedger` itself, because the
/// two track different moments and need different lock discipline:
/// `ChunkLedger::register` runs BEFORE a chunk's download starts (so a
/// mid-write failure is still tracked for cleanup) and only ever needs a
/// synchronous `Vec::push` — no I/O, so a `std::sync::Mutex` held for a
/// single statement is correct and cheap. `record_and_save` runs AFTER a
/// chunk succeeds, and must persist to disk before another completion can
/// observe a consistent manifest, so it needs an async lock held across
/// that write (see below) — a fundamentally different obligation than
/// registering an about-to-exist path.
///
/// Uses a `tokio::sync::Mutex`, held across the save itself (mirroring
/// `DashDownloadState`'s save-under-lock pattern): a std `Mutex` released
/// between the record and the save let two concurrent completions race —
/// snapshot A (with chunk 3) and snapshot B (with chunks 3+4) could reach
/// `save` in either order, and A landing last silently dropped chunk 4 from
/// the file on disk even though `record_completed` itself never lost it.
/// Holding the async lock across record-then-save serializes the pair, so
/// whichever completion's save runs last is provably a superset of every
/// completion recorded before its own lock acquisition.
#[derive(Debug, Clone)]
pub(super) struct ChunkManifestTracker {
    manifest: Arc<Mutex<ChunkManifest>>,
    /// `None` when the owning [`super::chunk_name::ChunkSet`] has no
    /// manifest path (the legacy grammar) — recording becomes a no-op
    /// rather than a call site needing to branch on set kind.
    path: Option<PathBuf>,
}

impl ChunkManifestTracker {
    pub(super) fn new(manifest: ChunkManifest, path: Option<PathBuf>) -> Self {
        Self {
            manifest: Arc::new(Mutex::new(manifest)),
            path,
        }
    }

    /// Record `chunk_id` as complete at `len` bytes and persist the updated
    /// manifest. Save failures are logged and otherwise swallowed — a
    /// missed manifest write degrades resume-recovery precision for this
    /// attempt, it does not fail the download in progress, matching
    /// `note_sidecar_save`'s severity for the HLS/DASH sidecars.
    pub(super) async fn record_and_save(&self, chunk_id: u64, len: u64) {
        let Some(path) = &self.path else { return };
        // Held across the save: see the type-level doc for why releasing
        // this between record and save is the bug.
        let mut guard = self.manifest.lock().await;
        guard.record_completed(chunk_id, len);
        if let Err(e) = guard.save(path).await {
            warn!(path:? = path; "Failed to save chunk manifest: {e}");
        }
    }

    /// Remove the manifest file. Called once an attempt no longer needs it
    /// — either its chunks were merged (success) or discarded (failure) —
    /// so a stale manifest never outlives the chunk files it describes.
    /// `NotFound` is not an error: the desired end state either way is "no
    /// manifest file present".
    pub(super) async fn delete(&self) {
        let Some(path) = &self.path else { return };
        let _ = tokio::fs::remove_file(path).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the on-disk `kind` literal: `#[serde(rename_all)]` is absent, so
    /// serde's default enum-tagging serializes the Rust variant NAME
    /// verbatim. A future rename of `ChunkKind::Fresh`/`Resume` would
    /// silently change this string and orphan every manifest already on
    /// disk (`load_matching`'s `kind` comparison would then reject them as
    /// a mismatch) — this test makes that rename loud instead.
    #[test]
    fn on_disk_kind_literal_is_pinned() {
        let fresh = serde_json::to_string(&ChunkKind::Fresh).expect("serialize");
        let resume = serde_json::to_string(&ChunkKind::Resume).expect("serialize");
        assert_eq!(fresh, "\"Fresh\"");
        assert_eq!(resume, "\"Resume\"");
    }

    #[tokio::test]
    async fn save_then_load_matching_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Title.mp4.7.chunks.json");
        let mut manifest = ChunkManifest::new(7, ChunkKind::Fresh, 1_000_000);
        manifest.record_completed(0, 65_536);
        manifest.record_completed(1, 65_536);
        manifest.save(&path).await.expect("save");

        let loaded = ChunkManifest::load_matching(&path, 7, ChunkKind::Fresh)
            .await
            .expect("must load a matching manifest");
        assert_eq!(loaded.recorded_len(0), Some(65_536));
        assert_eq!(loaded.recorded_len(1), Some(65_536));
        assert_eq!(loaded.recorded_len(2), None, "chunk 2 was never recorded");
    }

    #[tokio::test]
    async fn load_matching_rejects_download_id_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.json");
        ChunkManifest::new(7, ChunkKind::Fresh, 100)
            .save(&path)
            .await
            .expect("save");

        assert!(
            ChunkManifest::load_matching(&path, 8, ChunkKind::Fresh)
                .await
                .is_none(),
            "a manifest for a different download_id must not be trusted"
        );
    }

    #[tokio::test]
    async fn load_matching_rejects_kind_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.json");
        ChunkManifest::new(7, ChunkKind::Fresh, 100)
            .save(&path)
            .await
            .expect("save");

        assert!(
            ChunkManifest::load_matching(&path, 7, ChunkKind::Resume)
                .await
                .is_none(),
            "a Fresh manifest must not be trusted for a Resume probe"
        );
    }

    #[tokio::test]
    async fn load_matching_rejects_schema_version_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.json");
        let mut manifest = ChunkManifest::new(7, ChunkKind::Fresh, 100);
        manifest.schema_version = CHUNK_MANIFEST_VERSION + 1;
        manifest.save(&path).await.expect("save");

        assert!(
            ChunkManifest::load_matching(&path, 7, ChunkKind::Fresh)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn load_matching_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.json");
        assert!(
            ChunkManifest::load_matching(&path, 7, ChunkKind::Fresh)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn tracker_with_no_path_is_a_no_op() {
        let tracker = ChunkManifestTracker::new(ChunkManifest::new(0, ChunkKind::Fresh, 0), None);
        // Must not panic and must not attempt any I/O.
        tracker.record_and_save(0, 10).await;
        tracker.delete().await;
    }

    #[tokio::test]
    async fn tracker_delete_removes_the_manifest_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.json");
        let tracker = ChunkManifestTracker::new(
            ChunkManifest::new(0, ChunkKind::Fresh, 10),
            Some(path.clone()),
        );
        tracker.record_and_save(0, 10).await;
        assert!(tokio::fs::metadata(&path).await.is_ok(), "manifest saved");

        tracker.delete().await;
        assert!(
            tokio::fs::metadata(&path).await.is_err(),
            "manifest must be gone after delete"
        );
    }

    /// Spec review finding 4: `record_and_save` used to snapshot under a std
    /// `Mutex` and save outside it, so two concurrent completions could race
    /// and land an older snapshot last, dropping a recorded chunk from the
    /// file on disk. Holding the (now async) lock across record-then-save
    /// serializes them; every one of N concurrent completions must survive
    /// to the final on-disk manifest.
    #[tokio::test]
    async fn concurrent_record_and_save_calls_all_survive_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.json");
        let tracker = ChunkManifestTracker::new(
            ChunkManifest::new(0, ChunkKind::Fresh, 0),
            Some(path.clone()),
        );

        const N: u64 = 32;
        let mut tasks = tokio::task::JoinSet::new();
        for chunk_id in 0..N {
            let tracker = tracker.clone();
            tasks.spawn(async move { tracker.record_and_save(chunk_id, chunk_id + 1).await });
        }
        tasks.join_all().await;

        let loaded = ChunkManifest::load_matching(&path, 0, ChunkKind::Fresh)
            .await
            .expect("manifest must have been saved at least once");
        for chunk_id in 0..N {
            assert_eq!(
                loaded.recorded_len(chunk_id),
                Some(chunk_id + 1),
                "chunk {chunk_id} must survive concurrent saves, not be dropped by a race"
            );
        }
    }
}
