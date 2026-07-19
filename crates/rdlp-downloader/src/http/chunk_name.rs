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
//!    adaptive path calls no cleanup function at all.
//!
//! Cleanup of a failed download no longer scans a directory at all: the
//! writer registers each chunk path with a [`super::chunk_ledger::ChunkLedger`]
//! at construction time, and cleanup deletes exactly those registered paths.
//! An earlier version of this module scanned the temp directory for names
//! matching this grammar and deleted every match — but `DOWNLOAD_ID_COUNTER`
//! (`parallel.rs`) is a per-process counter starting at 0, so two concurrent
//! rdlp processes downloading the same filename into the same directory can
//! both compute `download_id == 0`, and a pattern-sweep in one process would
//! then delete the other's live chunk files. This is the same defect class
//! as #558 (`cleanup_leftover_segments`'s pattern-matched directory sweep),
//! just reached from a different starting point; see
//! `scripts/check-no-dir-sweep-delete.sh` for the standing gate against it.
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

use std::path::{Path, PathBuf};

/// Origin of a chunk within its download attempt: a fresh transfer or one
/// resumed from a previous, interrupted attempt.
///
/// Modeled as an enum (not a raw `&str` marker) because the set of markers is
/// closed and a typo in a string literal would silently create an
/// unclaimable chunk file.
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
    /// `.{download_id}.part`, wide enough to claim any foreign `.partN` file
    /// sharing that download id. Taking `&Path` and delegating to
    /// [`Path::file_name`] makes that state unrepresentable instead of
    /// validating a `String` after the fact.
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
    /// test). This is deliberate, not an oversight: resume detection
    /// (rdlp-api, #559) must recognize whatever file it claims by name, and
    /// a stricter parse could leave a chunk file written with leading zeros
    /// by some other rdlp version unrecognized.
    #[must_use]
    pub fn claims(&self, file_name: &str) -> Option<u64> {
        Self::claims_with_prefix(&self.prefix(), file_name)
    }

    /// Shared matching logic behind [`Self::claims`], taking an
    /// already-built `prefix` so a caller checking many file names against
    /// the same set (e.g. resume detection scanning a directory read-only)
    /// can compute it once instead of once per entry.
    fn claims_with_prefix(prefix: &str, file_name: &str) -> Option<u64> {
        let rest = file_name.strip_prefix(prefix)?;
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest.parse().ok()
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
}
