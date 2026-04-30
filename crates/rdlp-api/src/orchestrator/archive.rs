//! Download archive for tracking completed downloads.
//!
//! Records completed downloads as `{extractor} {id}` lines in a text file.
//! On subsequent runs, already-present entries are skipped. Compatible with
//! yt-dlp's `--download-archive` format.
//!
//! # Concurrency
//!
//! Read and write paths take an advisory file lock via [`fs4::fs_std::FileExt`]
//! so concurrent rdlp processes (multiple terminals, automated pipelines, the
//! desktop app and a CLI run side-by-side) cannot interleave their writes and
//! corrupt entries. The lock is exclusive on writes and shared on reads, and
//! is released when the file handle drops.

use fs4::fs_std::FileExt;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Load archive entries from a file into a `HashSet`.
///
/// Returns an empty set if the file does not exist. Blank lines and lines
/// starting with `#` are ignored. Takes a shared lock for the duration of
/// the read so a concurrent writer cannot append a half-line under us.
pub fn load_archive(path: &Path) -> HashSet<String> {
    // Safe: sync helper invoked only via tokio::task::spawn_blocking (see orchestrator/mod.rs::load_archive_if_configured).
    #[allow(clippy::disallowed_methods)]
    let Ok(file) = std::fs::File::open(path) else {
        return HashSet::new();
    };

    // Shared lock: many readers may proceed concurrently; an exclusive
    // writer (record_in_archive) blocks until we release. If locking
    // fails (rare; e.g. NFS without lock support) fall back to the
    // unlocked read — better than a hard error here, since the archive
    // is best-effort tracking, not security-critical state.
    let _lock = file.lock_shared().ok();

    BufReader::new(&file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|line| line.trim().to_owned())
        .collect()
    // `_lock` (when Some) drops here — the underlying `file` drop releases
    // the OS-level lock.
}

/// Check whether a video is already recorded in the archive.
pub fn is_in_archive(archive: &HashSet<String>, extractor: &str, id: &str) -> bool {
    archive.contains(&format!("{extractor} {id}"))
}

/// Append a completed download entry to the archive file.
///
/// Creates the file (and parent directories) if they don't exist. Holds an
/// exclusive advisory lock for the duration of the append so concurrent
/// `record_in_archive` calls (across processes or threads) cannot interleave
/// writes and produce a torn line.
pub fn record_in_archive(path: &Path, extractor: &str, id: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        // Safe: sync helper invoked only via tokio::task::spawn_blocking (see orchestrator/mod.rs::record_in_archive).
        #[allow(clippy::disallowed_methods)]
        std::fs::create_dir_all(parent)?;
    }

    // Windows note: opening with `.append(true)` alone produces a handle with
    // only FILE_APPEND_DATA access. LockFileEx (what fs4 calls underneath)
    // requires GENERIC_READ or GENERIC_WRITE on the handle and fails with
    // ERROR_ACCESS_DENIED otherwise. Adding `.read(true)` widens the desired
    // access mask without changing the append semantic — the kernel still
    // adjusts the file offset to end-of-file on every write thanks to
    // `.append(true)`.
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)?;

    // Exclusive lock — blocks until any concurrent writer releases. NFS
    // / non-POSIX filesystems may return an error; in that environment
    // we surface it because skipping the lock would silently allow
    // interleaved writes. Filesystems that don't support advisory locks
    // are not a supported deployment target for the download archive.
    file.lock_exclusive()?;

    let result = writeln!(file, "{extractor} {id}");

    // Explicit unlock (also released on drop, but this makes the
    // ordering with the write flush obvious).
    let _ = FileExt::unlock(&file);

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn load_empty_set_when_file_missing() {
        let archive = load_archive(Path::new("/nonexistent/archive.txt"));
        assert!(archive.is_empty());
    }

    #[test]
    fn load_skips_blank_and_comment_lines() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "# rdlp download archive").unwrap();
        writeln!(tmp).unwrap();
        writeln!(tmp, "SiteA abc123").unwrap();
        writeln!(tmp, "  # another comment  ").unwrap();
        writeln!(tmp, "SiteB xyz789").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert_eq!(archive.len(), 2);
        assert!(archive.contains("SiteA abc123"));
        assert!(archive.contains("SiteB xyz789"));
    }

    #[test]
    fn is_in_archive_checks_extractor_and_id() {
        let mut archive = HashSet::new();
        archive.insert("SiteA abc123".to_string());

        assert!(is_in_archive(&archive, "SiteA", "abc123"));
        assert!(!is_in_archive(&archive, "SiteA", "other"));
        assert!(!is_in_archive(&archive, "SiteB", "abc123"));
    }

    #[test]
    fn record_appends_entry() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        record_in_archive(&path, "SiteA", "abc123").unwrap();
        record_in_archive(&path, "SiteB", "xyz789").unwrap();

        let archive = load_archive(&path);
        assert_eq!(archive.len(), 2);
        assert!(is_in_archive(&archive, "SiteA", "abc123"));
        assert!(is_in_archive(&archive, "SiteB", "xyz789"));
    }

    #[test]
    fn record_creates_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("archive.txt");

        record_in_archive(&path, "SiteA", "id1").unwrap();

        let archive = load_archive(&path);
        assert!(is_in_archive(&archive, "SiteA", "id1"));
    }

    /// Regression: verify that two writers contending for the same archive
    /// produce a complete, well-formed file rather than interleaved bytes.
    /// This is the failing test (against the pre-flock implementation) that
    /// the bug-fix-requires-failing-test rule asks for.
    ///
    /// We launch many threads that each write a long marker line. With
    /// `O_APPEND` alone, glibc is allowed to split `write()` into multiple
    /// system calls under load — historically this is the source of torn
    /// lines. With `flock`, every line lands atomically.
    #[test]
    fn record_appends_are_atomic_under_thread_contention() {
        use std::sync::Arc;
        use std::thread;

        const THREADS: usize = 32;
        const PER_THREAD: usize = 50;

        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("archive.txt"));
        // Long-ish per-line payload increases the chance that an unprotected
        // write would tear. Using `extractor` as a fixed prefix and `id` as
        // a marker makes interleavings detectable: every recorded line MUST
        // be `SiteA <numeric-id>` exactly.

        let mut handles = Vec::with_capacity(THREADS);
        for t in 0..THREADS {
            let path = path.clone();
            handles.push(thread::spawn(move || {
                for i in 0..PER_THREAD {
                    let id = format!("{t:02}-{i:04}-padding-padding-padding-padding");
                    record_in_archive(&path, "SiteA", &id).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let archive = load_archive(&path);
        assert_eq!(
            archive.len(),
            THREADS * PER_THREAD,
            "expected {} unique entries, got {} (interleaved write detected)",
            THREADS * PER_THREAD,
            archive.len()
        );

        // Every entry MUST start with `SiteA ` and be followed by a
        // well-formed marker. A torn write would leave entries that don't
        // match this shape.
        for entry in &archive {
            let suffix = entry
                .strip_prefix("SiteA ")
                .unwrap_or_else(|| panic!("malformed entry: {entry:?}"));
            assert!(
                suffix.contains("-padding-padding-padding-padding"),
                "torn line detected: {entry:?}"
            );
        }
    }
}
