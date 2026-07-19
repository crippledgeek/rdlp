//! Single authoritative grammar for parallel-download chunk file names.
//!
//! Before this module existed, the chunk-name grammar was duplicated as five
//! independent `format!` strings across `rdlp-downloader` and `rdlp-api`, and
//! the divergence already produced two live defects this module closes
//! (refs #568, #559):
//!
//! 1. **Kind-blind cleanup** (`parallel.rs::cleanup_chunk_files`) hardcodes
//!    the `.part{chunk_id}` marker while the chunk writer interpolates a
//!    `suffix` that is `"resume"` on the resume path — so a failed *resumed*
//!    download's cleanup pass silently deletes nothing. This is #568's live
//!    leak and the reason chunk kind ([`ChunkKind`]) is part of the grammar,
//!    not an afterthought.
//! 2. **Range-bounded cleanup**: the same function bounds deletion by
//!    `total_chunks`, a count the adaptive controller doesn't have — it
//!    generates chunk ids lazily via `stream::try_unfold` (`parallel.rs`) —
//!    so an adaptive download can exceed any a-priori bound, and the
//!    adaptive path calls no cleanup function at all. [`ChunkSet::sweep`]
//!    replaces the bounded loop with a directory scan, so the final chunk
//!    count need never be known up front.
//!
//! (The `rdlp-api` legacy-chunk cleaner's `0..10` scan bound, by contrast,
//! never leaked in practice: the legacy grammar predates power-of-two
//! chunking, when `concurrent_fragments` was capped at 10 — see
//! `orchestrator/resume.rs`. It's included here only because [`ChunkSet`]
//! now also owns that grammar's parsing.)
//!
//! [`ChunkSet`] is the one place a chunk file name is built or parsed. Two
//! grammars are represented:
//!
//! - **new-style** (`download_id: Some(_)`): `{filename}.{download_id}.{marker}{chunk_id}`
//! - **legacy** (`download_id: None`, pre-Phase-2.5): `{filename}.{marker}{chunk_id}`
//!
//! `filename` is the full output file name including extension (e.g.
//! `Title.mp4`), so a real chunk looks like `Title.mp4.7.resume3`.

use anyhow::Context;
use log::warn;
use std::path::{Path, PathBuf};

/// Origin of a chunk within its download attempt: a fresh transfer or one
/// resumed from a previous, interrupted attempt.
///
/// Modeled as an enum (not a raw `&str` marker) because the set of markers is
/// closed and a typo in a string literal would silently create an
/// unclaimable — and therefore unsweepable — chunk file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// A chunk written by a brand-new download attempt.
    Fresh,
    /// A chunk written while resuming a previously interrupted attempt.
    Resume,
}

impl ChunkKind {
    /// The literal marker embedded in the chunk file name for this kind.
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Fresh => "part",
            Self::Resume => "resume",
        }
    }
}

/// Report of a [`ChunkSet::sweep`] pass over a directory.
///
/// `std::io::Error` implements neither `Clone` nor `PartialEq` (it may wrap
/// an arbitrary boxed source), so this type's equality is hand-written
/// below rather than derived: two reports are equal when they deleted the
/// same count and their errors match pairwise by path and
/// [`std::io::ErrorKind`] (the part of an `io::Error` that is meaningfully
/// comparable).
#[derive(Debug, Default)]
#[must_use]
pub struct SweepReport {
    /// Number of chunk files this set claimed and successfully deleted.
    pub deleted: usize,
    /// Per-file errors encountered while deleting a claimed chunk, paired
    /// with the path that failed. A `NotFound` on removal is not recorded
    /// here (the file is already gone, which is the desired end state);
    /// every other I/O error is, kept as a typed [`std::io::Error`] so a
    /// caller can distinguish e.g. `PermissionDenied` from a transient
    /// failure rather than pattern-matching a formatted string.
    pub errors: Vec<(PathBuf, std::io::Error)>,
}

impl PartialEq for SweepReport {
    fn eq(&self, other: &Self) -> bool {
        self.deleted == other.deleted
            && self.errors.len() == other.errors.len()
            && self.errors.iter().zip(&other.errors).all(
                |((path, err), (other_path, other_err))| {
                    path == other_path && err.kind() == other_err.kind()
                },
            )
    }
}

impl Eq for SweepReport {}

/// A named, addressable set of parallel-download chunk files sharing one
/// output filename, one download attempt (or the legacy "no id" attempt),
/// and one [`ChunkKind`].
///
/// `download_id: None` denotes the legacy pre-Phase-2.5 grammar
/// (`{filename}.part{chunk_id}`, no download id segment). The legacy
/// grammar is Fresh-only (see [`ChunkSet::legacy`]); a legacy *resumed*
/// chunk set has no producer anywhere in the tree, so it is deliberately
/// unreachable through this type's public constructors rather than merely
/// undocumented.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkSet {
    filename: String,
    download_id: Option<u64>,
    kind: ChunkKind,
}

impl ChunkSet {
    /// Construct the descriptor for a chunk belonging to a specific,
    /// numbered download attempt. Does not touch the filesystem.
    pub fn for_attempt(filename: impl Into<String>, download_id: u64, kind: ChunkKind) -> Self {
        Self {
            filename: filename.into(),
            download_id: Some(download_id),
            kind,
        }
    }

    /// Construct the descriptor for the legacy pre-Phase-2.5 grammar
    /// (`{filename}.part{chunk_id}`, no download-id segment). Always
    /// [`ChunkKind::Fresh`] — the legacy grammar predates resume support,
    /// so a legacy chunk set is read/delete-only against files nothing in
    /// the tree still writes. Does not touch the filesystem.
    pub fn legacy(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            download_id: None,
            kind: ChunkKind::Fresh,
        }
    }

    /// The shared prefix every chunk file name in this set begins with,
    /// immediately followed by the chunk id's decimal digits.
    fn prefix(&self) -> String {
        self.download_id.map_or_else(
            || format!("{}.{}", self.filename, self.kind.marker()),
            |id| format!("{}.{id}.{}", self.filename, self.kind.marker()),
        )
    }

    /// Build the path for `chunk_id` within `dir`.
    #[must_use]
    pub fn path_in(&self, dir: &Path, chunk_id: u64) -> PathBuf {
        dir.join(format!("{}{chunk_id}", self.prefix()))
    }

    /// If `file_name` belongs to this set, return its chunk id.
    ///
    /// A match requires the exact prefix for this set's filename,
    /// download id, and kind, followed by one or more ASCII digits and
    /// nothing else — so a foreign file, a different download id, a
    /// different [`ChunkKind`], or a non-numeric/empty suffix are all
    /// rejected.
    #[must_use]
    pub fn claims(&self, file_name: &str) -> Option<u64> {
        Self::claims_with_prefix(&self.prefix(), file_name)
    }

    /// Shared matching logic behind [`Self::claims`], taking an
    /// already-built `prefix` so [`Self::sweep`] can compute it once per
    /// directory scan instead of once per entry.
    fn claims_with_prefix(prefix: &str, file_name: &str) -> Option<u64> {
        let rest = file_name.strip_prefix(prefix)?;
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest.parse().ok()
    }

    /// Scan `dir` and delete every entry this set [`claims`](Self::claims).
    ///
    /// This is a directory scan rather than an index loop over an assumed
    /// chunk-id range: the adaptive downloader generates chunk ids lazily,
    /// so the final count is unknown at cleanup time, and a bounded loop is
    /// exactly the defect (#559) this module exists to close.
    ///
    /// A missing directory is not an error (nothing to sweep); any other
    /// I/O error reading the directory is surfaced. Per-file deletion
    /// errors are collected in the returned [`SweepReport`] rather than
    /// aborting the sweep, so one locked/foreign-owned file doesn't stop
    /// cleanup of the rest — but each is also logged at `warn!` level here,
    /// matching the severity the pre-existing bounded cleanup used, so a
    /// failed delete stays operator-visible even though it no longer aborts
    /// the sweep.
    ///
    /// `std::fs::remove_file` (and its `tokio::fs` equivalent used here)
    /// removes a symlink itself rather than following it, so a hostile
    /// symlink named to match this set's grammar cannot be used to delete
    /// an arbitrary target through this pattern-based sweep.
    ///
    /// # Errors
    ///
    /// Returns an error if `dir` exists but cannot be read (permissions,
    /// I/O failure) or if iterating its entries fails partway through.
    /// A missing `dir` is not an error.
    pub async fn sweep(&self, dir: &Path) -> anyhow::Result<SweepReport> {
        let mut report = SweepReport::default();

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(e) => {
                return Err(e).with_context(|| format!("reading directory {}", dir.display()));
            }
        };

        let prefix = self.prefix();
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("iterating directory {}", dir.display()))?
        {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if Self::claims_with_prefix(&prefix, name).is_none() {
                continue;
            }

            let path = entry.path();
            match tokio::fs::remove_file(&path).await {
                Ok(()) => report.deleted += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    warn!(path:? = path; "Failed to delete chunk file: {e}");
                    report.errors.push((path, e));
                }
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_new_style_fresh() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 3);
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "Title.mp4.7.part3"
        );
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(set.claims(name), Some(3));
    }

    #[test]
    fn round_trip_new_style_resume() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Resume);
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 3);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "Title.mp4.7.resume3");
        assert_eq!(set.claims(name), Some(3));
    }

    #[test]
    fn round_trip_legacy() {
        let set = ChunkSet::legacy("Title.mp4");
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 4);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "Title.mp4.part4");
        assert_eq!(set.claims(name), Some(4));
    }

    #[test]
    fn claims_rejects_different_download_id() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.8.part3"), None);
    }

    #[test]
    fn claims_rejects_different_filename() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        assert_eq!(set.claims("Other.mp4.7.part3"), None);
    }

    #[test]
    fn claims_rejects_bare_marker_with_no_digits() {
        let set = ChunkSet::legacy("Title.mp4");
        assert_eq!(set.claims("Title.mp4.part"), None);
    }

    #[test]
    fn claims_rejects_non_numeric_suffix() {
        let set = ChunkSet::legacy("Title.mp4");
        assert_eq!(set.claims("Title.mp4.part1x"), None);
    }

    #[test]
    fn claims_rejects_foreign_file() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4"), None);
    }

    #[test]
    fn fresh_set_does_not_claim_resume_chunk() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.7.resume3"), None);
    }

    #[test]
    fn resume_set_does_not_claim_fresh_chunk() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Resume);
        assert_eq!(set.claims("Title.mp4.7.part3"), None);
    }

    #[test]
    fn claims_unbounded_index_beyond_legacy_ten_chunk_cap() {
        // #559: the legacy scanner bounded chunk ids to 0..10. This set must
        // claim chunk ids well beyond that bound.
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.7.part12"), Some(12));
        assert_eq!(set.claims("Title.mp4.7.part100"), Some(100));
    }

    #[tokio::test]
    async fn sweep_deletes_only_claimed_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);

        // 12 chunks belonging to this set — well past the legacy 0..10 bound.
        for chunk_id in 0..12u64 {
            let path = set.path_in(dir.path(), chunk_id);
            tokio::fs::write(&path, b"chunk")
                .await
                .expect("write chunk");
        }

        // 2 foreign files that must survive the sweep untouched.
        let foreign_a = dir.path().join("Title.mp4");
        let foreign_b = dir.path().join("Other.mp4.7.part0");
        tokio::fs::write(&foreign_a, b"final")
            .await
            .expect("write foreign a");
        tokio::fs::write(&foreign_b, b"other")
            .await
            .expect("write foreign b");

        let report = set.sweep(dir.path()).await.expect("sweep");
        assert_eq!(report.deleted, 12);
        assert!(report.errors.is_empty());

        let mut remaining = Vec::new();
        let mut entries = tokio::fs::read_dir(dir.path()).await.expect("read_dir");
        while let Some(entry) = entries.next_entry().await.expect("next_entry") {
            remaining.push(entry.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(remaining.len(), 2);
        assert!(remaining.contains(&"Title.mp4".to_owned()));
        assert!(remaining.contains(&"Other.mp4.7.part0".to_owned()));
    }

    #[tokio::test]
    async fn sweep_missing_directory_is_not_an_error() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        let report = set
            .sweep(Path::new("/nonexistent/rdlp-chunk-name-test-dir"))
            .await
            .expect("missing dir sweeps clean");
        assert_eq!(report.deleted, 0);
        assert!(report.errors.is_empty());
    }

    #[tokio::test]
    async fn sweep_records_typed_error_for_undeletable_entry() {
        // A directory that matches this set's grammar can never be removed
        // by `remove_file`, so it deterministically exercises the
        // `SweepReport::errors` path with a real `io::ErrorKind` rather
        // than a synthetic one.
        let dir = tempfile::tempdir().expect("tempdir");
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh);
        let undeletable = set.path_in(dir.path(), 0);
        tokio::fs::create_dir(&undeletable)
            .await
            .expect("create dir standing in for the chunk file");

        let report = set.sweep(dir.path()).await.expect("sweep");

        assert_eq!(report.deleted, 0);
        assert_eq!(report.errors.len(), 1);
        let (path, err) = &report.errors[0];
        assert_eq!(path, &undeletable);
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
