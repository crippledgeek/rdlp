//! Crash-safe temp file registry with per-temp advisory lock files.
//!
//! `TempRegistry` tracks all pipeline temp files globally. On `Drop`, it
//! deletes any remaining registered paths (orphaned temps from a crash or
//! early exit). On startup, `cleanup_stale()` removes files left by a prior
//! crash.
//!
//! ## Lock-file protocol (audit finding H10)
//!
//! A sidecar `.lock` file lives alongside each registered temp:
//!
//! ```text
//! video.rdlp-tmp-<uuid>.mp4        ← the temp itself
//! video.rdlp-tmp-<uuid>.mp4.lock   ← exclusive advisory lock held by this process
//! ```
//!
//! `cleanup_stale()` uses `try_lock_exclusive` on the sidecar. If the lock is
//! already held (`try_lock_exclusive` returns `Ok(false)`), the temp is still
//! live in another rdlp process and is left alone. If `try_lock_exclusive`
//! returns `Ok(true)`, the owner has crashed and the temp is safe to delete.
//!
//! This prevents one rdlp process from deleting another process's in-progress
//! temp files during startup cleanup.
//!
//! # Lint allowances
//!
//! - `clippy::case_sensitive_file_extension_comparisons`: `.lock` and `.rdlp-tmp-`
//!   are always lowercase on all supported platforms; case-insensitive comparison
//!   would be misleading noise here.
//! - `clippy::unnecessary_literal_bound`: `fn name()` trait method returns literals.

#![allow(
    clippy::case_sensitive_file_extension_comparisons,
    clippy::unnecessary_literal_bound
)]

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fs4::fs_std::FileExt;

/// Entry stored for each registered temp file.
///
/// Keeping the `File` open holds the exclusive advisory lock on the sidecar for
/// the lifetime of this entry.
struct TempEntry {
    /// Open handle to the `.lock` sidecar — holds the exclusive advisory lock.
    _lock_file: File,
}

/// Global registry of pipeline temp files for crash-safe cleanup.
///
/// Share as `Arc<TempRegistry>` across stages.
pub struct TempRegistry {
    /// Maps temp path → its lock entry (holds the advisory lock open).
    active: Mutex<HashMap<PathBuf, TempEntry>>,
}

impl TempRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
        }
    }

    /// Register a temp file path. Called by `FileTracker::temp_path()`.
    ///
    /// Creates a `.lock` sidecar and acquires an exclusive advisory lock on it.
    /// The lock is held until the entry is released or the registry drops.
    // Safe: sync helper — never called inside an async executor worker.
    #[allow(clippy::disallowed_methods)]
    pub fn register(&self, path: &Path) {
        let lock_path = lock_path_for(path);
        let lock_file = match File::create(&lock_path) {
            Ok(f) => f,
            Err(e) => {
                // Non-fatal: log and fall back to a non-locking entry. The temp
                // will still be cleaned up on Drop; it just won't be skipped by
                // cleanup_stale in concurrent processes.
                log::warn!(
                    "TempRegistry: could not create lock file {}: {e}; \
                     cleanup_stale protection disabled for this temp",
                    lock_path.display()
                );
                // Insert without a lock file: we can't open /dev/null portably,
                // so re-open the temp itself as a placeholder. On drop the lock_file
                // field just closes an fd — no advisory lock to release.
                let placeholder = match File::open(path) {
                    Ok(f) => f,
                    // If even the temp can't be opened, skip registration entirely
                    // to avoid a spurious entry with a dangling placeholder fd.
                    Err(e2) => {
                        log::warn!(
                            "TempRegistry: could not open temp {} for placeholder: {e2}; \
                             skipping registration",
                            path.display()
                        );
                        return;
                    }
                };
                self.active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(
                        path.to_path_buf(),
                        TempEntry {
                            _lock_file: placeholder,
                        },
                    );
                return;
            }
        };
        if let Err(e) = lock_file.try_lock_exclusive() {
            log::warn!(
                "TempRegistry: could not lock {}: {e}; advisory lock not held",
                lock_path.display()
            );
        }
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                path.to_path_buf(),
                TempEntry {
                    _lock_file: lock_file,
                },
            );
    }

    /// Release a path — file has been moved to its final location and no
    /// longer needs crash cleanup.
    ///
    /// Drops the advisory lock and removes the `.lock` sidecar.
    // Safe: sync helper.
    #[allow(clippy::disallowed_methods)]
    pub fn release(&self, path: &Path) {
        let entry = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(path);
        // Drop the TempEntry here → advisory lock released, fd closed.
        drop(entry);
        // Remove the sidecar (best-effort; ignore errors).
        let lock_path = lock_path_for(path);
        let _ = std::fs::remove_file(&lock_path);
    }

    /// Check whether a path is currently registered.
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(path)
    }

    /// Drain the registry atomically and delete all registered temp files.
    ///
    /// Called on explicit shutdown (e.g. `RunEvent::ExitRequested`, SIGTERM).
    /// The map is drained under the lock; file deletion happens outside the
    /// lock to avoid holding the lock across I/O. Double-delete is impossible
    /// because the map is drained atomically.
    // Safe: invoked from sync CLI/Tauri shutdown paths, not from an async runtime worker.
    #[allow(clippy::disallowed_methods)]
    pub fn cleanup_all(&self) {
        let entries: Vec<(PathBuf, TempEntry)> = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .collect();
        for (path, entry) in entries {
            // Drop the advisory lock first, then delete both files.
            drop(entry);
            let lock_path = lock_path_for(&path);
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
            let _ = std::fs::remove_file(&lock_path);
        }
    }

    /// Scan `dir` for stale `*.rdlp-tmp-*` files and delete those whose
    /// advisory lock is not held by any live process.
    ///
    /// A file is considered stale (and safe to delete) when:
    /// 1. It matches the `*.rdlp-tmp-*` naming pattern, AND
    /// 2. Its `.lock` sidecar is either absent OR `try_lock_exclusive` returns
    ///    `Ok(true)` (meaning no process holds it).
    ///
    /// If `try_lock_exclusive` returns `Ok(false)`, the temp is in active
    /// use by another process and is left alone — this prevents a fresh rdlp
    /// instance from deleting another process's live temp files (audit finding H10).
    ///
    /// The age-based 1-hour threshold is retained as an additional guard: if
    /// somehow a lock sidecar was lost, we only prune files older than 1 hour.
    // Safe: CLI invokes via spawn_blocking; Tauri invokes from sync startup/shutdown.
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
            // Skip .lock sidecars themselves.
            if name.ends_with(".lock") {
                continue;
            }
            if !name.contains(".rdlp-tmp-") {
                continue;
            }

            // ── Advisory lock check ──────────────────────────────────────────
            // `try_lock_exclusive` returns:
            //   Ok(true)  → lock acquired → no live process owns it → safe to delete
            //   Ok(false) → lock held by another process → LEAVE IT ALONE
            //   Err(_)    → I/O error on the sidecar → treat as orphaned (delete)
            let lock_path = lock_path_for(&path);
            let live_process_holds_lock = if lock_path.exists() {
                match File::open(&lock_path) {
                    Ok(f) => match f.try_lock_exclusive() {
                        Ok(true) => {
                            // We got the lock → orphaned. Unlock before deleting
                            // so the OS can clean up the lock state.
                            let _ = f.unlock();
                            false // not held by another process
                        }
                        Ok(false) => true, // held by another live process
                        Err(_) => false,   // can't take lock → treat as orphaned
                    },
                    Err(_) => false, // can't open → treat as orphaned
                }
            } else {
                false // no sidecar → legacy / crashed before sidecar was created
            };

            if live_process_holds_lock {
                log::debug!(
                    "TempRegistry: skipping {} (lock held by another process)",
                    path.display()
                );
                continue;
            }

            // Age check: avoid deleting very-recently-created temps that have
            // no sidecar yet (window between file creation and register()).
            let age = match entry.metadata().and_then(|m| m.modified()) {
                Ok(mtime) => now.duration_since(mtime).unwrap_or_default(),
                Err(e) => {
                    log::debug!(
                        "TempRegistry: stat failed for {} ({e}); skipping stale-prune",
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
                    let _ = std::fs::remove_file(&lock_path);
                }
            }
        }
    }
}

/// Compute the path of the advisory lock sidecar for a given temp file.
///
/// Convention: `{temp_path}.lock`
fn lock_path_for(temp: &Path) -> PathBuf {
    let mut p = temp.as_os_str().to_owned();
    p.push(".lock");
    PathBuf::from(p)
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
        let entries: Vec<(PathBuf, TempEntry)> = self
            .active
            .get_mut()
            .map(|m| m.drain().collect())
            .unwrap_or_default();
        for (path, entry) in entries {
            drop(entry); // release advisory lock
            let lock_path = lock_path_for(&path);
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
            let _ = std::fs::remove_file(&lock_path);
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
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp-tmp-abc.mp4");
        fs::write(&path, b"test").unwrap();
        let reg = TempRegistry::new();
        reg.register(&path);
        assert!(reg.contains(&path));
        // Lock sidecar should exist while registered.
        assert!(lock_path_for(&path).exists());
        reg.release(&path);
        assert!(!reg.contains(&path));
        // Lock sidecar should be gone after release.
        assert!(!lock_path_for(&path).exists());
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
            // reg drops here — should delete the file and sidecar
        }
        assert!(!path.exists());
        assert!(!lock_path_for(&path).exists());
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
    fn test_cleanup_stale_removes_old_orphaned_temps() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("video.rdlp-tmp-abc123.mp4");
        fs::write(&path, b"test").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(two_hours_ago))
            .unwrap();
        // No sidecar → treated as orphaned.
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
        // Second call must not panic — file is gone, map is empty
        reg.cleanup_all();
    }

    // ── H10 regression: cleanup_stale skips live-locked temps ───────────────
    //
    // Registers a temp file (creating the sidecar + holding its exclusive lock),
    // then runs `cleanup_stale` with a 2-hour-old mtime. The temp must survive
    // because the sidecar is locked by THIS process (simulating another live
    // rdlp process).
    //
    // After the registry drops (releasing the lock and removing the sidecar),
    // a second cleanup run should remove the file.
    #[test]
    fn test_cleanup_stale_preserves_live_locked_temp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("live.rdlp-tmp-live123.mp4");
        fs::write(&path, b"data").unwrap();

        let reg = TempRegistry::new();
        reg.register(&path);

        // Make the file appear very old so the age-based check would normally
        // trigger deletion. The lock check must prevent deletion.
        let very_old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(very_old)).unwrap();

        // With the lock held, cleanup_stale must NOT delete the temp.
        TempRegistry::cleanup_stale(dir.path());
        assert!(
            path.exists(),
            "cleanup_stale must not delete a temp whose lock is held by a live process"
        );

        // Release the lock (drop removes the sidecar and the temp).
        drop(reg);

        // The temp is now gone because Drop cleaned it up. Verify the sidecar
        // was also removed.
        assert!(
            !lock_path_for(&path).exists(),
            "sidecar must be removed after drop"
        );
    }
}
