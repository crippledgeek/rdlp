//! Crash-safe temp file registry.
//!
//! `TempRegistry` tracks all pipeline temp files globally. On `Drop`, it
//! deletes any remaining registered paths (orphaned temps from a crash or
//! early exit). On startup, `cleanup_stale()` removes files left by a prior
//! crash.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Global registry of pipeline temp files for crash-safe cleanup.
///
/// Share as `Arc<TempRegistry>` across stages.
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

    /// Register a temp file path. Called by `FileTracker::temp_path()`.
    pub fn register(&self, path: &Path) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.to_path_buf());
    }

    /// Release a path — file has been moved to its final location and no
    /// longer needs crash cleanup.
    pub fn release(&self, path: &Path) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(path);
    }

    /// Check whether a path is currently registered.
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(path)
    }

    /// Drain the registry atomically and delete all registered temp files.
    ///
    /// Called on explicit shutdown (e.g. `RunEvent::ExitRequested`, SIGTERM).
    /// The set is drained under the lock; file deletion happens outside the
    /// lock to avoid holding the lock across I/O. Double-delete is impossible
    /// because the set is drained atomically.
    // Safe: invoked from sync CLI/Tauri shutdown paths, not from an async runtime worker.
    #[allow(clippy::disallowed_methods)]
    pub fn cleanup_all(&self) {
        let paths: Vec<PathBuf> = self
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        for path in paths {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!(
                        "TempRegistry: cleanup_all failed for {}: {e}",
                        path.display()
                    );
                } else {
                    log::debug!("TempRegistry: cleanup_all removed {}", path.display());
                }
            }
        }
    }

    /// Scan `dir` for stale `*.rdlp-tmp-*` files older than 1 hour and delete
    /// them. Called on app startup to remove files left by a prior crash.
    // Safe: CLI invokes via spawn_blocking; Tauri invokes from sync startup/shutdown; no async runtime active on this thread.
    #[allow(clippy::disallowed_methods)]
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
            if !name.contains(".rdlp-tmp-") {
                continue;
            }
            let age = match entry.metadata().and_then(|m| m.modified()) {
                Ok(mtime) => now.duration_since(mtime).unwrap_or_default(),
                Err(e) => {
                    log::debug!(
                        "TempRegistry: stat failed for {} ({e}); skipping stale-prune \
                         (file may persist past 1h threshold)",
                        path.display()
                    );
                    continue;
                }
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

impl Drop for TempRegistry {
    // Safe: Drop runs synchronously on the thread that owns the value; no async runtime context.
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        // Drain and delete all remaining tracked temps.
        let paths: Vec<PathBuf> = self
            .active
            .get_mut()
            .map(|s| s.drain().collect())
            .unwrap_or_default();
        for path in paths {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!(
                        "TempRegistry: drop cleanup failed for {}: {e}",
                        path.display()
                    );
                } else {
                    log::debug!("TempRegistry: drop-cleaned {}", path.display());
                }
            }
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

    #[test]
    fn test_register_and_release() {
        let reg = TempRegistry::new();
        let path = PathBuf::from("/tmp/test-rdlp-tmp-abc.mp4");
        reg.register(&path);
        assert!(reg.contains(&path));
        reg.release(&path);
        assert!(!reg.contains(&path));
    }

    #[test]
    fn test_drop_deletes_remaining_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp-tmp-123.mp4");
        fs::write(&path, b"test").unwrap();
        assert!(path.exists());
        {
            let reg = TempRegistry::new();
            reg.register(&path);
            // reg drops here — should delete the file
        }
        assert!(!path.exists());
    }

    #[test]
    fn test_drop_skips_released_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp-tmp-456.mp4");
        fs::write(&path, b"test").unwrap();
        {
            let reg = TempRegistry::new();
            reg.register(&path);
            reg.release(&path);
            // reg drops — path was released so NOT deleted
        }
        assert!(path.exists());
    }

    #[test]
    fn test_cleanup_stale_removes_old_temps() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("video.rdlp-tmp-abc123.mp4");
        fs::write(&path, b"test").unwrap();
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

    #[test]
    fn test_cleanup_all_deletes_registered_files() {
        let dir = TempDir::new().unwrap();
        let path1 = dir.path().join("a.rdlp-tmp-111.mp4");
        let path2 = dir.path().join("b.rdlp-tmp-222.mp4");
        fs::write(&path1, b"test").unwrap();
        fs::write(&path2, b"test").unwrap();

        let reg = TempRegistry::new();
        reg.register(&path1);
        reg.register(&path2);

        reg.cleanup_all();

        assert!(!path1.exists());
        assert!(!path2.exists());
        // Registry is now empty — Drop should be a no-op
        assert!(!reg.contains(&path1));
        assert!(!reg.contains(&path2));
    }

    #[test]
    fn test_cleanup_all_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.rdlp-tmp-333.mp4");
        fs::write(&path, b"test").unwrap();

        let reg = TempRegistry::new();
        reg.register(&path);
        reg.cleanup_all();
        // Second call must not panic — file is gone, set is empty
        reg.cleanup_all();
    }
}
