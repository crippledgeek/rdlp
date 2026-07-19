//! Single authoritative grammar for parallel-download chunk file names.
//!
//! Before this module existed, the chunk-name grammar was duplicated as five
//! independent `format!` strings across `rdlp-downloader` and `rdlp-api`, and
//! the divergence already produced a live data leak (refs #568, #559): the
//! `rdlp-api` cleaner bounded its directory scan to `0..10` chunks, silently
//! leaking every chunk beyond that when a download used more connections.
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
#[derive(Debug, Clone, Default)]
pub struct SweepReport {
    /// Number of chunk files this set claimed and successfully deleted.
    pub deleted: usize,
    /// Per-file errors encountered while deleting a claimed chunk. A
    /// `NotFound` on removal is not recorded here (the file is already gone,
    /// which is the desired end state); every other I/O error is.
    pub errors: Vec<String>,
}

/// A named, addressable set of parallel-download chunk files sharing one
/// output filename, one download attempt (or the legacy "no id" attempt),
/// and one [`ChunkKind`].
///
/// `download_id: None` denotes the legacy pre-Phase-2.5 grammar
/// (`{filename}.part{chunk_id}`, no download id segment).
#[derive(Debug, Clone)]
pub struct ChunkSet {
    filename: String,
    download_id: Option<u64>,
    kind: ChunkKind,
}

impl ChunkSet {
    /// Construct a chunk set descriptor. Does not touch the filesystem.
    pub fn new(filename: impl Into<String>, download_id: Option<u64>, kind: ChunkKind) -> Self {
        Self {
            filename: filename.into(),
            download_id,
            kind,
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
        let rest = file_name.strip_prefix(self.prefix().as_str())?;
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
    /// cleanup of the rest.
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

        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("iterating directory {}", dir.display()))?
        {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if self.claims(&name).is_none() {
                continue;
            }

            let path = entry.path();
            match tokio::fs::remove_file(&path).await {
                Ok(()) => report.deleted += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => report.errors.push(format!("{}: {e}", path.display())),
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_new_style_fresh() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 3);
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "Title.mp4.7.part3"
        );
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(set.claims(name), Some(3));
    }

    #[tokio::test]
    async fn round_trip_new_style_resume() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Resume);
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 3);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "Title.mp4.7.resume3");
        assert_eq!(set.claims(name), Some(3));
    }

    #[tokio::test]
    async fn round_trip_legacy() {
        let set = ChunkSet::new("Title.mp4", None, ChunkKind::Fresh);
        let dir = Path::new("/tmp/does-not-need-to-exist-for-this-test");
        let path = set.path_in(dir, 4);
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "Title.mp4.part4");
        assert_eq!(set.claims(name), Some(4));
    }

    #[tokio::test]
    async fn claims_rejects_different_download_id() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.8.part3"), None);
    }

    #[tokio::test]
    async fn claims_rejects_different_filename() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        assert_eq!(set.claims("Other.mp4.7.part3"), None);
    }

    #[tokio::test]
    async fn claims_rejects_bare_marker_with_no_digits() {
        let set = ChunkSet::new("Title.mp4", None, ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.part"), None);
    }

    #[tokio::test]
    async fn claims_rejects_non_numeric_suffix() {
        let set = ChunkSet::new("Title.mp4", None, ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.part1x"), None);
    }

    #[tokio::test]
    async fn claims_rejects_foreign_file() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4"), None);
    }

    #[tokio::test]
    async fn fresh_set_does_not_claim_resume_chunk() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.7.resume3"), None);
    }

    #[tokio::test]
    async fn resume_set_does_not_claim_fresh_chunk() {
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Resume);
        assert_eq!(set.claims("Title.mp4.7.part3"), None);
    }

    #[tokio::test]
    async fn claims_unbounded_index_beyond_legacy_ten_chunk_cap() {
        // #559: the legacy scanner bounded chunk ids to 0..10. This set must
        // claim chunk ids well beyond that bound.
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        assert_eq!(set.claims("Title.mp4.7.part12"), Some(12));
        assert_eq!(set.claims("Title.mp4.7.part100"), Some(100));
    }

    #[tokio::test]
    async fn sweep_deletes_only_claimed_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);

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
        let set = ChunkSet::new("Title.mp4", Some(7), ChunkKind::Fresh);
        let report = set
            .sweep(Path::new("/nonexistent/rdlp-chunk-name-test-dir"))
            .await
            .expect("missing dir sweeps clean");
        assert_eq!(report.deleted, 0);
        assert!(report.errors.is_empty());
    }
}
