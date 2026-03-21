//! Crash-safe temp file registry.
//!
//! `TempRegistry` tracks all pipeline temp files globally. On shutdown, it
//! deletes any remaining registered files. On startup, `cleanup_stale()`
//! removes old `*.rdlp-tmp-*` files left by a prior crash.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Global registry of pipeline temp files for crash-safe cleanup.
pub struct TempRegistry {
    active: Mutex<HashSet<PathBuf>>,
}

impl TempRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: Mutex::new(HashSet::new()),
        }
    }

    /// Register a temp file path. Called by [`crate::pipeline::tracker::FileTracker::temp_path`].
    pub fn register(&self, path: &Path) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.to_path_buf());
    }

    /// Unregister a temp file path. Called before deleting a file so that
    /// `cleanup_all` does not double-delete it.
    pub fn unregister(&self, path: &Path) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(path);
    }

    /// Check whether a path is currently registered.
    pub fn contains(&self, path: &Path) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(path)
    }

    /// Drain the registry atomically and delete all registered temp files.
    ///
    /// The set is drained under lock to prevent double-delete races with
    /// concurrent `unregister` / file-deletion calls.
    pub fn cleanup_all(&self) {
        let paths: Vec<PathBuf> = {
            let mut guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain().collect()
        };
        for path in paths {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("TempRegistry: failed to delete {}: {e}", path.display());
                } else {
                    log::debug!("TempRegistry: cleaned up {}", path.display());
                }
            }
        }
    }

    /// Scan `dir` for stale `*.rdlp-tmp-*` files older than 1 hour and delete
    /// them. Called on app startup to remove files left by a prior crash.
    pub fn cleanup_stale(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let one_hour = std::time::Duration::from_secs(3600);
        let now = std::time::SystemTime::now();

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            // Match files with `.rdlp-tmp-` in the stem (before the final ext)
            if !name.contains(".rdlp-tmp-") {
                continue;
            }
            let age = match entry.metadata().and_then(|m| m.modified()) {
                Ok(mtime) => now.duration_since(mtime).unwrap_or_default(),
                Err(_) => continue,
            };
            if age >= one_hour {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!(
                        "TempRegistry: failed to remove stale temp {}: {e}",
                        path.display()
                    );
                } else {
                    log::info!("TempRegistry: removed stale temp {}", path.display());
                }
            }
        }
    }
}

impl Default for TempRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_register_and_unregister() {
        let reg = TempRegistry::new();
        let path = PathBuf::from("/tmp/test-rdlp-tmp-abc.mp4");
        reg.register(&path);
        assert!(reg.contains(&path));
        reg.unregister(&path);
        assert!(!reg.contains(&path));
    }

    #[test]
    fn test_cleanup_all_deletes_files() {
        let dir = TempDir::new().unwrap();
        let reg = TempRegistry::new();
        let path = dir.path().join("test.rdlp-tmp-123.mp4");
        fs::write(&path, b"test").unwrap();
        reg.register(&path);
        reg.cleanup_all();
        assert!(!path.exists());
        assert!(!reg.contains(&path));
    }

    #[test]
    fn test_cleanup_stale_removes_old_temps() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("video.rdlp-tmp-abc123.mp4");
        fs::write(&path, b"test").unwrap();
        // Set modification time to 2 hours ago
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(two_hours_ago))
            .unwrap();
        TempRegistry::cleanup_stale(dir.path());
        assert!(!path.exists());
    }

    #[test]
    fn test_cleanup_stale_preserves_non_temp_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("my-video.mp4");
        fs::write(&path, b"test").unwrap();
        TempRegistry::cleanup_stale(dir.path());
        assert!(path.exists());
    }
}
