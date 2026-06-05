//! Per-video file lifecycle tracker.
//!
//! `FileTracker` is the single source of truth for file state during
//! post-processing. Only `FileTracker` decides when files are promoted,
//! demoted to temps, or deleted. No stage ever calls `remove_file` directly.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;

use crate::pipeline::registry::TempRegistry;

/// The marker substring embedded in every temp file name handed out by
/// [`FileTracker::temp_path`] (`{stem}.rdlp-tmp-{uuid}.{ext}`).
const TEMP_MARKER: &str = ".rdlp-tmp-";

/// Returns whether `path`'s file name contains the temp marker
/// ([`TEMP_MARKER`]) that [`FileTracker::temp_path`] embeds. Used by `Drop`
/// to detect (and warn about) deletion of a file that is NOT temp-namespaced —
/// the cancel-cleanup contract expects every deletable path to be temp-named.
fn is_temp_named(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains(TEMP_MARKER))
}

/// Tracks current and temporary file paths for a single post-processing job.
///
/// After each stage produces output, the stage calls [`replace`] to promote
/// the output to `current_files` and move the old files to `temp_files`.
/// When the pipeline completes successfully, [`cleanup`] deletes all temp files.
///
/// On `Drop` without a prior [`commit`] (or [`cleanup`], which commits as its
/// last step), `FileTracker` performs graceful-cancel cleanup: it deletes every
/// uncommitted path it issued via [`temp_path`] plus any `current_files`, and
/// releases each from [`TempRegistry`]. This prevents partial recode output and
/// intermediate files from leaking when a job is cancelled mid-pipeline (#335).
/// [`TempRegistry`] remains the crash/stale backstop for the case where the
/// process dies before `Drop` runs.
pub struct FileTracker {
    /// The live files currently being processed.
    pub(crate) current_files: Vec<PathBuf>,
    /// Files that have been superseded by a later stage's output.
    pub(crate) temp_files: Vec<PathBuf>,
    /// Every path handed out by [`temp_path`] — the uncommitted-output set
    /// deleted on cancel-drop.
    issued: Vec<PathBuf>,
    /// Disarms the `Drop` cleanup once the job succeeds (`commit`/`cleanup`).
    committed: bool,
    temp_registry: Arc<TempRegistry>,
}

impl FileTracker {
    /// Create a new tracker with the given initial files.
    ///
    /// # Precondition
    ///
    /// Callers MUST pass temp-namespaced (`*.rdlp-tmp-*`) paths as the initial
    /// `files`. On a cancel/uncommitted [`Drop`], every path in `current_files`
    /// is deleted; a non-temp path (e.g. a user's original file) would be
    /// deleted too. Such deletions are still performed (the cancel intent is to
    /// clear the working set), but a [`log::warn!`] is emitted naming the path,
    /// since it signals a caller-contract violation. The orchestrator satisfies
    /// this precondition by UUID-renaming inputs to `*.rdlp-tmp-*` before
    /// constructing the tracker.
    pub const fn new(files: Vec<PathBuf>, temp_registry: Arc<TempRegistry>) -> Self {
        Self {
            current_files: files,
            temp_files: Vec::new(),
            issued: Vec::new(),
            committed: false,
            temp_registry,
        }
    }

    /// Promote `new_files` to `current_files`, moving the old current files
    /// into `temp_files`.
    pub fn replace(&mut self, new_files: Vec<PathBuf>) {
        let old = std::mem::replace(&mut self.current_files, new_files);
        self.temp_files.extend(old);
    }

    /// Mark a path as a temp file without changing `current_files`.
    pub fn mark_temp(&mut self, path: PathBuf) {
        self.temp_files.push(path);
    }

    /// Generate a unique temp path adjacent to `base`:
    /// `{base_stem}.rdlp-tmp-{uuid}.{ext}`.
    ///
    /// The path is immediately registered with [`TempRegistry`] so that a
    /// crash before the stage completes will still clean it up.
    #[must_use]
    pub fn temp_path(&mut self, base: &Path, ext: &str) -> PathBuf {
        let dir = base.parent().unwrap_or_else(|| Path::new("."));
        let raw_stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
        // Strip any existing .rdlp-tmp-{uuid} suffixes to prevent stacking
        let stem = raw_stem.split(TEMP_MARKER).next().unwrap_or(raw_stem);
        let id = Uuid::new_v4().simple().to_string();
        let filename = format!("{stem}{TEMP_MARKER}{id}.{ext}");
        let path = dir.join(filename);
        self.temp_registry.register(&path);
        // Record the issued path so an uncommitted Drop deletes it (#335).
        self.issued.push(path.clone());
        path
    }

    /// Disarm the cancel-cleanup `Drop`, marking the job as successfully
    /// committed so its output files are kept.
    pub const fn commit(&mut self) {
        self.committed = true;
    }

    /// Return the first current file (convenience for single-file stages).
    ///
    /// # Panics
    ///
    /// Panics if `current_files` is empty.
    #[must_use]
    pub fn primary(&self) -> PathBuf {
        self.current_files[0].clone()
    }

    /// Delete all temp files and unregister them from [`TempRegistry`].
    ///
    /// Called on successful pipeline completion. On crash, the
    /// [`TempRegistry`] shutdown hook handles cleanup instead.
    // Safe: invoked via tokio::task::spawn_blocking from pipeline/mod.rs:182.
    #[allow(clippy::disallowed_methods)]
    pub fn cleanup(&mut self) {
        let to_delete: Vec<PathBuf> = self.temp_files.drain(..).collect();
        for path in to_delete {
            // Release from registry first (under lock), then delete.
            self.temp_registry.release(&path);
            if path.exists()
                && let Err(e) = std::fs::remove_file(&path)
            {
                log::warn!("FileTracker: failed to delete temp {}: {e}", path.display());
            }
        }

        // Rename current files from UUID temp names back to clean names
        for file in &mut self.current_files {
            if let Some(name) = file.file_name().and_then(|n| n.to_str())
                && let Some(idx) = name.find(".rdlp-tmp-")
            {
                let clean_stem = &name[..idx];
                let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
                let clean_name = format!("{clean_stem}.{ext}");
                let clean_path = file.with_file_name(&clean_name);
                match std::fs::rename(&file, &clean_path) {
                    Ok(()) => {
                        self.temp_registry.release(file);
                        *file = clean_path;
                    }
                    Err(e) => {
                        log::warn!(
                            "FileTracker: failed to rename {} → {}: {e}",
                            file.display(),
                            clean_name,
                        );
                    }
                }
            }
        }

        // Successful completion: disarm the cancel-cleanup Drop.
        self.committed = true;
    }
}

impl Drop for FileTracker {
    /// Graceful-cancel cleanup: if the job did not [`commit`](FileTracker::commit)
    /// (or [`cleanup`](FileTracker::cleanup), which commits last), delete every
    /// uncommitted issued path and any `current_files`, releasing each from the
    /// [`TempRegistry`] (#335).
    // Safe: synchronous remove_file in Drop is unavoidable — Drop cannot be
    // async, and this mirrors the existing justification on `cleanup()` and the
    // registry's own shutdown hook.
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Collect into an owned Vec to avoid borrowing self while mutating it.
        let to_delete: Vec<PathBuf> = self
            .issued
            .iter()
            .chain(self.current_files.iter())
            .cloned()
            .collect();
        for path in to_delete {
            // Visible guard (#339): every deletable path SHOULD be
            // temp-namespaced. If a caller violated the `new()` precondition by
            // passing a non-temp file, warn (but still delete — the cancel
            // intent is to clear the working set). Drop never panics.
            if !is_temp_named(&path) {
                log::warn!(
                    "FileTracker::Drop deleting non-temp-named file {} — unexpected; \
                     current_files should be temp-namespaced",
                    path.display(),
                );
            }
            // The issued and current sets may overlap (mark_temp re-files
            // issued paths); the exists() guard + idempotent release make this
            // safe to run per path.
            if path.exists()
                && let Err(e) = std::fs::remove_file(&path)
            {
                log::warn!(
                    "FileTracker: failed to delete uncommitted {} on cancel: {e}",
                    path.display(),
                );
            }
            self.temp_registry.release(&path);
        }
    }
}

#[cfg(test)]
// Safe: test fixtures — no async runtime in #[test] fns.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn test_registry() -> Arc<TempRegistry> {
        Arc::new(TempRegistry::new())
    }

    #[test]
    fn is_temp_named_detects_marker() {
        assert!(is_temp_named(Path::new("video.rdlp-tmp-abc123.mkv")));
        assert!(!is_temp_named(Path::new("video.mkv")));
        assert!(!is_temp_named(Path::new("/home/u/My Video.mp4")));
        // marker only in a parent dir component must NOT match (file_name-only check)
        assert!(!is_temp_named(Path::new("/home/u/.rdlp-tmp-abc/video.mkv")));
    }

    #[test]
    fn test_new_tracker() {
        let reg = test_registry();
        let tracker = FileTracker::new(vec![PathBuf::from("a.mp4")], reg);
        assert_eq!(tracker.current_files, vec![PathBuf::from("a.mp4")]);
        assert!(tracker.temp_files.is_empty());
    }

    #[test]
    fn test_replace_moves_old_to_temps() {
        let reg = test_registry();
        let mut tracker = FileTracker::new(vec![PathBuf::from("a.mp4")], reg);
        tracker.replace(vec![PathBuf::from("b.mp4")]);
        assert_eq!(tracker.current_files, vec![PathBuf::from("b.mp4")]);
        assert_eq!(tracker.temp_files, vec![PathBuf::from("a.mp4")]);
    }

    #[test]
    fn test_temp_path_generates_unique_uuid_paths() {
        let reg = test_registry();
        let mut tracker = FileTracker::new(vec![], reg);
        let p1 = tracker.temp_path(&PathBuf::from("/tmp/video.mp4"), "mp4");
        let p2 = tracker.temp_path(&PathBuf::from("/tmp/video.mp4"), "mp4");
        assert_ne!(p1, p2);
        assert!(p1.to_str().unwrap().contains("rdlp-tmp-"));
        assert_eq!(p1.extension().unwrap(), "mp4");
    }

    #[test]
    fn test_temp_path_registers_with_registry() {
        // Use a real temp dir so `TempRegistry::register()` can create the
        // sidecar lock file. Hard-coded `/tmp/...` doesn't exist on Windows
        // and causes registration to skip silently.
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        let mut tracker = FileTracker::new(vec![], reg.clone());
        let path = tracker.temp_path(&dir.path().join("video.mp4"), "mp4");
        assert!(reg.contains(&path));
    }

    #[test]
    fn test_primary_returns_first_file() {
        let reg = test_registry();
        let tracker = FileTracker::new(vec![PathBuf::from("a.mp4"), PathBuf::from("b.m4a")], reg);
        assert_eq!(tracker.primary(), PathBuf::from("a.mp4"));
    }

    #[test]
    fn test_cleanup_deletes_temp_files() {
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        let temp = dir.path().join("old.mp4");
        fs::write(&temp, b"test").unwrap();
        let mut tracker = FileTracker::new(vec![temp.clone()], reg);
        tracker.replace(vec![dir.path().join("new.mp4")]);
        tracker.cleanup();
        assert!(!temp.exists());
    }

    #[test]
    fn drop_uncommitted_deletes_issued_temp_output() {
        let dir = TempDir::new().unwrap();
        let reg = Arc::new(TempRegistry::new());
        let input = dir.path().join("in.rdlp-tmp-aaa.mkv");
        fs::write(&input, b"x").unwrap();
        let partial = {
            let mut tracker = FileTracker::new(vec![input.clone()], reg.clone());
            let out = tracker.temp_path(&input, "mkv"); // issued, NOT replace()-promoted
            fs::write(&out, b"partial").unwrap();
            assert!(out.exists());
            out
            // tracker dropped here WITHOUT commit() → cancel semantics
        };
        assert!(
            !partial.exists(),
            "issued partial output must be deleted on uncommitted drop"
        );
        assert!(
            !input.exists(),
            "input intermediate (current_files) must be deleted too"
        );
        assert!(
            !reg.contains(&partial),
            "registry entry for partial must be released"
        );
    }

    #[test]
    fn drop_after_commit_keeps_files() {
        let dir = TempDir::new().unwrap();
        let reg = Arc::new(TempRegistry::new());
        let input = dir.path().join("in.rdlp-tmp-bbb.mkv");
        fs::write(&input, b"x").unwrap();
        let kept = {
            let mut tracker = FileTracker::new(vec![input.clone()], reg.clone());
            let out = tracker.temp_path(&input, "mkv");
            fs::write(&out, b"final").unwrap();
            tracker.commit(); // success path disarms Drop
            out
        };
        // `reg` is held alive past the tracker's drop, mirroring production
        // (`Pipeline` owns the `Arc<TempRegistry>` for its whole lifetime).
        // Without this keepalive, dropping the last `Arc` triggers
        // `TempRegistry`'s own sweep, which deletes the committed-but-still-
        // registered output and masks what this test is asserting.
        assert!(
            reg.contains(&kept),
            "committed output stays registered until pipeline-level cleanup"
        );
        assert!(kept.exists(), "committed tracker must NOT delete on drop");
    }
}
