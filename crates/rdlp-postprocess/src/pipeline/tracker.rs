//! Per-video file lifecycle tracker.
//!
//! `FileTracker` is the single source of truth for file state during
//! post-processing. Only `FileTracker` decides when files are promoted,
//! demoted to temps, or deleted. No stage ever calls `remove_file` directly.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;

use crate::pipeline::registry::TempRegistry;

/// Tracks current and temporary file paths for a single post-processing job.
///
/// After each stage produces output, the stage calls [`replace`] to promote
/// the output to `current_files` and move the old files to `temp_files`.
/// When the pipeline completes successfully, [`cleanup`] deletes all temp files.
///
/// Crash cleanup is handled by [`TempRegistry`] — `FileTracker` does NOT
/// implement `Drop` with file deletion.
pub struct FileTracker {
    /// The live files currently being processed.
    pub(crate) current_files: Vec<PathBuf>,
    /// Files that have been superseded by a later stage's output.
    pub(crate) temp_files: Vec<PathBuf>,
    temp_registry: Arc<TempRegistry>,
}

impl FileTracker {
    /// Create a new tracker with the given initial files.
    pub fn new(files: Vec<PathBuf>, temp_registry: Arc<TempRegistry>) -> Self {
        Self {
            current_files: files,
            temp_files: Vec::new(),
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
    pub fn temp_path(&self, base: &Path, ext: &str) -> PathBuf {
        let dir = base.parent().unwrap_or(Path::new("."));
        let raw_stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
        // Strip any existing .rdlp-tmp-{uuid} suffixes to prevent stacking
        let stem = raw_stem.split(".rdlp-tmp-").next().unwrap_or(raw_stem);
        let id = Uuid::new_v4().simple().to_string();
        let filename = format!("{stem}.rdlp-tmp-{id}.{ext}");
        let path = dir.join(filename);
        self.temp_registry.register(&path);
        path
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn test_registry() -> Arc<TempRegistry> {
        Arc::new(TempRegistry::new())
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
        let tracker = FileTracker::new(vec![], reg);
        let p1 = tracker.temp_path(&PathBuf::from("/tmp/video.mp4"), "mp4");
        let p2 = tracker.temp_path(&PathBuf::from("/tmp/video.mp4"), "mp4");
        assert_ne!(p1, p2);
        assert!(p1.to_str().unwrap().contains("rdlp-tmp-"));
        assert_eq!(p1.extension().unwrap(), "mp4");
    }

    #[test]
    fn test_temp_path_registers_with_registry() {
        let reg = test_registry();
        let tracker = FileTracker::new(vec![], reg.clone());
        let path = tracker.temp_path(&PathBuf::from("/tmp/video.mp4"), "mp4");
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
}
