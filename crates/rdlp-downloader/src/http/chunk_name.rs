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
use log::{debug, warn};
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
/// No `PartialEq`/`Eq`: `std::io::Error` implements neither (it may wrap an
/// arbitrary boxed source), and no caller needs whole-struct equality — every
/// test in this module asserts on `deleted`/`errors` directly, which is also
/// the more informative failure message on assertion failure.
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
    ///
    /// Takes the *output path*, not a pre-extracted filename string: every
    /// migrating call site otherwise duplicated its own
    /// `.file_name().to_string_lossy()` boilerplate, and an empty or
    /// filename-less string would silently build a set whose prefix is
    /// `.{download_id}.part`, wide enough to claim (and sweep would then
    /// delete) any foreign `.partN` file sharing that download id. Taking
    /// `&Path` and delegating to [`Path::file_name`] makes that state
    /// unrepresentable instead of validating a `String` after the fact.
    ///
    /// # Errors
    ///
    /// Returns an error if `output_path` has no filename component (e.g. it
    /// is empty, `.`, `..`, or root).
    pub fn for_attempt(
        output_path: impl AsRef<Path>,
        download_id: u64,
        kind: ChunkKind,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            filename: Self::filename_of(output_path.as_ref())?,
            download_id: Some(download_id),
            kind,
        })
    }

    /// Construct the descriptor for the legacy pre-Phase-2.5 grammar
    /// (`{filename}.part{chunk_id}`, no download-id segment). Always
    /// [`ChunkKind::Fresh`] — the legacy grammar predates resume support,
    /// so a legacy chunk set is read/delete-only against files nothing in
    /// the tree still writes. Does not touch the filesystem.
    ///
    /// See [`Self::for_attempt`] for why this takes a path rather than a
    /// pre-extracted filename string.
    ///
    /// # Errors
    ///
    /// Returns an error if `output_path` has no filename component (e.g. it
    /// is empty, `.`, `..`, or root).
    pub fn legacy(output_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Ok(Self {
            filename: Self::filename_of(output_path.as_ref())?,
            download_id: None,
            kind: ChunkKind::Fresh,
        })
    }

    /// Extract the filename component required by both constructors above.
    fn filename_of(output_path: &Path) -> anyhow::Result<String> {
        output_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "output path {} has no filename component",
                    output_path.display()
                )
            })
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
    ///
    /// **Not a strict inverse of [`Self::path_in`].** `path_in` never emits
    /// a leading-zero chunk id, but `claims` parses the digit run with
    /// ordinary decimal parsing, so e.g. `"part007"` also claims chunk id
    /// `7` (see the `claims_lenient_leading_zeros_alias_to_same_chunk_id`
    /// test). This is deliberate, not an oversight: [`Self::sweep`] must
    /// delete whatever file it claims by name, and a stricter parse could
    /// leave orphaned a chunk file written with leading zeros by some other
    /// rdlp version.
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
    /// Only regular files are ever deleted: an entry whose name matches this
    /// set's grammar but whose [`std::fs::FileType`] is not a plain file
    /// (most notably a directory) is never ours — nothing in this tree
    /// creates a chunk *directory* — so it is skipped rather than handed to
    /// `remove_file`, which would otherwise fail it into [`SweepReport`] as
    /// a spurious `IsADirectory`/`EISDIR` error for a path this sweep must
    /// not touch.
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
                debug!(
                    path:? = entry.path();
                    "Skipping non-UTF-8 directory entry during chunk sweep: {}",
                    name.to_string_lossy()
                );
                continue;
            };
            if Self::claims_with_prefix(&prefix, name).is_none() {
                continue;
            }

            match entry.file_type().await {
                Ok(file_type) if file_type.is_file() => {}
                Ok(_) => continue, // not ours: no code in this tree creates a chunk directory/symlink
                Err(e) => {
                    warn!(path:? = entry.path(); "Failed to stat directory entry during chunk sweep: {e}");
                    continue;
                }
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
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
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
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Resume).expect("valid filename");
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 3);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "Title.mp4.7.resume3");
        assert_eq!(set.claims(name), Some(3));
    }

    #[test]
    fn round_trip_legacy() {
        let set = ChunkSet::legacy("Title.mp4").expect("valid filename");
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 4);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "Title.mp4.part4");
        assert_eq!(set.claims(name), Some(4));
    }

    #[test]
    fn claims_rejects_different_download_id() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        assert_eq!(set.claims("Title.mp4.8.part3"), None);
    }

    #[test]
    fn claims_rejects_different_filename() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        assert_eq!(set.claims("Other.mp4.7.part3"), None);
    }

    #[test]
    fn claims_rejects_bare_marker_with_no_digits() {
        let set = ChunkSet::legacy("Title.mp4").expect("valid filename");
        assert_eq!(set.claims("Title.mp4.part"), None);
    }

    #[test]
    fn claims_rejects_non_numeric_suffix() {
        let set = ChunkSet::legacy("Title.mp4").expect("valid filename");
        assert_eq!(set.claims("Title.mp4.part1x"), None);
    }

    #[test]
    fn claims_rejects_foreign_file() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        assert_eq!(set.claims("Title.mp4"), None);
    }

    #[test]
    fn fresh_set_does_not_claim_resume_chunk() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        assert_eq!(set.claims("Title.mp4.7.resume3"), None);
    }

    #[test]
    fn resume_set_does_not_claim_fresh_chunk() {
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Resume).expect("valid filename");
        assert_eq!(set.claims("Title.mp4.7.part3"), None);
    }

    #[test]
    fn claims_unbounded_index_beyond_legacy_ten_chunk_cap() {
        // #559: the legacy scanner bounded chunk ids to 0..10. This set must
        // claim chunk ids well beyond that bound.
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        assert_eq!(set.claims("Title.mp4.7.part12"), Some(12));
        assert_eq!(set.claims("Title.mp4.7.part100"), Some(100));
    }

    #[test]
    fn claims_lenient_leading_zeros_alias_to_same_chunk_id() {
        // N2: `claims` is deliberately not a strict inverse of `path_in` —
        // see the doc comment on `claims`. Pinned here so the aliasing is a
        // recorded decision, not an accident a future refactor "fixes" away.
        let set = ChunkSet::legacy("Title.mp4").expect("valid filename");
        assert_eq!(set.claims("Title.mp4.part007"), Some(7));
        // `path_in` itself never produces the leading-zero form.
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        assert_eq!(
            set.path_in(dir, 7).file_name().unwrap().to_str().unwrap(),
            "Title.mp4.part7"
        );
    }

    #[test]
    fn for_attempt_rejects_empty_filename() {
        // N4: an empty filename would otherwise build a set whose prefix is
        // bare `.{download_id}.part`, claiming any foreign `.partN` file
        // sharing that download id.
        assert!(ChunkSet::for_attempt("", 7, ChunkKind::Fresh).is_err());
    }

    #[test]
    fn legacy_rejects_empty_filename() {
        assert!(ChunkSet::legacy("").is_err());
    }

    #[test]
    fn empty_filename_error_prevents_claiming_a_foreign_part_file() {
        // N4b: construction fails outright, so there is no `ChunkSet` in
        // existence that could claim a foreign `.part1` via the empty-prefix
        // hazard described above.
        assert!(ChunkSet::for_attempt("", 1, ChunkKind::Fresh).is_err());
        assert!(ChunkSet::legacy("").is_err());
    }

    #[tokio::test]
    async fn sweep_deletes_only_claimed_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");

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
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        let report = set
            .sweep(Path::new("/nonexistent/rdlp-chunk-name-test-dir"))
            .await
            .expect("missing dir sweeps clean");
        assert_eq!(report.deleted, 0);
        assert!(report.errors.is_empty());
    }

    #[tokio::test]
    async fn sweep_skips_directory_matching_chunk_grammar() {
        // N1: a directory named exactly like a claimed chunk (e.g. some
        // out-of-band tool, or a user, created `Title.mp4.7.part3/`) is not
        // ours — nothing in this tree ever creates a chunk *directory* — so
        // it must survive sweep untouched and must not surface as an error.
        let dir = tempfile::tempdir().expect("tempdir");
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        let claimed_dir = set.path_in(dir.path(), 3);
        tokio::fs::create_dir(&claimed_dir)
            .await
            .expect("create directory standing in for a chunk");

        let report = set.sweep(dir.path()).await.expect("sweep");

        assert_eq!(report.deleted, 0);
        assert!(report.errors.is_empty());
        assert!(
            tokio::fs::metadata(&claimed_dir)
                .await
                .expect("directory must survive sweep")
                .is_dir()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sweep_records_typed_error_for_undeletable_entry() {
        // A regular file inside a directory with its write bit cleared
        // cannot be unlinked (no write permission on the containing
        // directory), so it deterministically exercises the
        // `SweepReport::errors` path with a real `io::ErrorKind` rather
        // than a synthetic one. (A chunk-named *directory* no longer takes
        // this path at all — see `sweep_skips_directory_matching_chunk_grammar`
        // — so a permission failure on a regular file is the remaining way
        // to exercise this branch.)
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let set = ChunkSet::for_attempt("Title.mp4", 7, ChunkKind::Fresh).expect("valid filename");
        let undeletable = set.path_in(dir.path(), 0);
        tokio::fs::write(&undeletable, b"chunk")
            .await
            .expect("write chunk standing in for the undeletable file");

        let mut perms = tokio::fs::metadata(dir.path())
            .await
            .expect("stat dir")
            .permissions();
        perms.set_mode(0o555); // read + execute, no write: entries can't be unlinked
        tokio::fs::set_permissions(dir.path(), perms.clone())
            .await
            .expect("make directory read-only");

        let report = set.sweep(dir.path()).await.expect("sweep");

        perms.set_mode(0o755);
        tokio::fs::set_permissions(dir.path(), perms)
            .await
            .expect("restore directory permissions for cleanup");

        assert_eq!(report.deleted, 0);
        assert_eq!(report.errors.len(), 1);
        let (path, err) = &report.errors[0];
        assert_eq!(path, &undeletable);
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
