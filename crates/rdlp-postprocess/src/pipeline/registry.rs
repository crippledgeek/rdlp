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
//! ## Claim entries (#572)
//!
//! [`TempRegistry::claim`] registers an entry of a SECOND kind: an ownership
//! lock on a path rdlp does not create or own the lifecycle of (the
//! `.rdlp-part` download-in-progress file). Sweeps (`cleanup_all`, `Drop`)
//! release a Claim entry's lock exactly like a Temp entry's, but NEVER
//! delete the tracked file itself — the download layer (`finalize_part` /
//! `discard_part`) owns that decision.
//!
//! `cleanup_stale()` filters on the `.rdlp-tmp-` marker, so it never even
//! looks at a `.rdlp-part` file or its `.lock` sidecar — a sidecar orphaned
//! by a `SIGKILL` (no graceful `release()`) is simply never swept. It is
//! reclaimed the ordinary way: the next `claim()` on that path calls
//! `File::create` on the sidecar, which truncates the stale (unlocked)
//! sidecar in place and re-locks it.
//!
//! # Lint allowances
//!
//! - `clippy::case_sensitive_file_extension_comparisons`: `.lock` and `.rdlp-tmp-`
//!   are always lowercase on all supported platforms; case-insensitive comparison
//!   would be misleading noise here.
//! - `clippy::unnecessary_literal_bound`: `fn name()` trait method returns literals.

#![allow(
    clippy::case_sensitive_file_extension_comparisons,
    clippy::unnecessary_literal_bound,
    // `Duration::from_hours` (lint's suggested replacement) needs Rust 1.95; MSRV is 1.85.
    clippy::duration_suboptimal_units
)]

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fs4::fs_std::FileExt;
use thiserror::Error;

/// Errors raised by [`TempRegistry::register`] and [`TempRegistry::claim`].
#[derive(Debug, Error)]
pub enum RegistryError {
    /// Another process — or another `TempRegistry` instance, which is what a
    /// second rdlp process looks like — already holds the advisory lock on
    /// `path`. Two processes writing the same output path is a conflict
    /// worth reporting, not one worth silently making safe (rdlp#572): the
    /// caller must refuse the claim rather than let a second writer share
    /// the same chunk files.
    #[error("path already claimed by another rdlp process: {}", path.display())]
    HeldElsewhere {
        /// The path that could not be claimed.
        path: PathBuf,
    },
}

/// What a registered entry means for the sweeps (`cleanup_all`, `Drop`).
///
/// The two are NOT interchangeable: a `Temp` entry names a file rdlp itself
/// created and owns outright, so a sweep deleting it is correct. A `Claim`
/// entry (rdlp#572) is a lock on a path rdlp does NOT own the lifecycle of —
/// the `.rdlp-part` download-in-progress file, which `finalize_part` /
/// `discard_part` manage explicitly. A sweep that can't tell the two apart
/// deletes a live, resumable download out from under its owner the moment
/// `cleanup_all` runs on process shutdown while a download is still active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EntryKind {
    /// Rdlp created and owns `path`; sweeps may delete it.
    Temp,
    /// Rdlp only holds an ownership claim on `path`; sweeps must NEVER
    /// delete the file itself, only release the lock and its sidecar.
    Claim,
}

/// Entry stored for each registered temp file.
///
/// Keeping the `File` open holds the exclusive advisory lock on the sidecar for
/// the lifetime of this entry.
struct TempEntry {
    /// Open handle to the `.lock` sidecar — holds the exclusive advisory lock.
    _lock_file: File,
    /// Whether a sweep may delete the tracked file itself.
    kind: EntryKind,
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

    /// Register a temp file rdlp itself owns. Called by
    /// `FileTracker::temp_path()`. Sweeps (`cleanup_all`, `Drop`) may delete
    /// the file itself once registered this way.
    ///
    /// Idempotent within one registry instance: re-registering a path this
    /// SAME instance already holds succeeds trivially. This idempotence is
    /// specific to `Temp` entries (a UUID temp name is never re-registered
    /// in practice, but a coincidental re-registration must not collide with
    /// our own lock) — [`Self::claim`] deliberately does NOT share it; see
    /// its doc comment.
    ///
    /// # Errors
    /// Returns [`RegistryError::HeldElsewhere`] when another live holder
    /// already has `path` locked. I/O failures opening the sidecar are
    /// non-fatal (see the private `acquire` helper below) — they predate #572 and are
    /// unrelated to the ownership-conflict guarantee this fn makes.
    pub fn register(&self, path: &Path) -> Result<(), RegistryError> {
        if self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(path)
        {
            return Ok(());
        }
        self.acquire(path, EntryKind::Temp)
    }

    /// Claim `path` as an ownership lock rdlp does NOT own the file
    /// lifecycle of — the `.rdlp-part` download-in-progress path (rdlp#572).
    /// Sweeps (`cleanup_all`, `Drop`) release the lock but NEVER delete the
    /// file itself; the caller (`finalize_part` / `discard_part`) owns that.
    ///
    /// Deliberately NOT idempotent, unlike [`Self::register`]: two
    /// concurrent downloads of the SAME target sharing one registry (e.g.
    /// two queued desktop downloads, `RdlpClient` hands every orchestrator
    /// the same registry) must be `HeldElsewhere` to the second caller, not
    /// a silent no-op success — the whole point is "this path already has an
    /// owner", regardless of whether that owner is this registry instance or
    /// another process's. `flock` is per-open-file-description, so opening a
    /// FRESH fd and locking naturally returns `Ok(false)` in exactly that
    /// case (verified in `test_register_refuses_when_lock_held_elsewhere`) —
    /// no separate same-instance check is needed here.
    ///
    /// # Errors
    /// Returns [`RegistryError::HeldElsewhere`] when `path` is already
    /// claimed or registered — by this registry instance or another.
    pub fn claim(&self, path: &Path) -> Result<(), RegistryError> {
        self.acquire(path, EntryKind::Claim)
    }

    /// Shared lock-acquisition mechanics for [`Self::register`] and
    /// [`Self::claim`] — the only difference between the two call sites is
    /// the idempotence check ([`Self::register`]'s, run by the caller before
    /// this fn) and the [`EntryKind`] recorded.
    ///
    /// Creates a `.lock` sidecar and acquires an exclusive advisory lock on
    /// it. Holding this lock IS the ownership claim on `path`: a second
    /// acquisition for the same path is a genuine conflict, reported as
    /// [`RegistryError::HeldElsewhere`], not a duplicate to paper over. The
    /// lock is held until the entry is released or the registry drops.
    // Safe: sync helper — never called inside an async executor worker.
    #[allow(clippy::disallowed_methods)]
    fn acquire(&self, path: &Path, kind: EntryKind) -> Result<(), RegistryError> {
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
                        return Ok(());
                    }
                };
                return self.insert_or_conflict(
                    path,
                    TempEntry {
                        _lock_file: placeholder,
                        kind,
                    },
                );
            }
        };
        match lock_file.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) => {
                // Held by a live process — the whole point of #572: refuse
                // rather than silently register a second, unlocked claim.
                return Err(RegistryError::HeldElsewhere {
                    path: path.to_path_buf(),
                });
            }
            Err(e) => {
                log::warn!(
                    "TempRegistry: could not lock {}: {e}; advisory lock not held",
                    lock_path.display()
                );
            }
        }
        self.insert_or_conflict(
            path,
            TempEntry {
                _lock_file: lock_file,
                kind,
            },
        )
    }

    /// Insert `new_entry` for `path`, UNLESS `path` is already held by a
    /// live `Claim` — an I/O-error fallback branch of [`Self::acquire`] has
    /// no working `flock` to rely on, so this reproduces the same refusal by
    /// hand. Fixes a #572 reintroduction: an unconditional `insert` here
    /// would silently replace an existing Claim entry, dropping the first
    /// holder's `_lock_file` (releasing its advisory lock) and reporting
    /// `Ok(())` to the second caller — the exact collision #572 exists to
    /// prevent, reached via the I/O-error path instead of `try_lock_exclusive`.
    ///
    /// An existing `Temp` entry is kept as-is and this reports `Ok(())`:
    /// `register`'s caller already short-circuits on `contains_key` before
    /// reaching `acquire` in the common case, so this only matters for a
    /// `claim` racing a `Temp` entry, which is not the conflict #572 guards.
    fn insert_or_conflict(&self, path: &Path, new_entry: TempEntry) -> Result<(), RegistryError> {
        use std::collections::hash_map::Entry;
        let mut map = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match map.entry(path.to_path_buf()) {
            Entry::Occupied(occupied) => match occupied.get().kind {
                EntryKind::Claim => Err(RegistryError::HeldElsewhere {
                    path: path.to_path_buf(),
                }),
                EntryKind::Temp => Ok(()),
            },
            Entry::Vacant(vacant) => {
                vacant.insert(new_entry);
                Ok(())
            }
        }
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
            // Capture kind before dropping: a Claim entry's file is NOT ours
            // to delete (rdlp#572) — only Temp entries are swept.
            let kind = entry.kind;
            drop(entry); // release the advisory lock
            let lock_path = lock_path_for(&path);
            if kind == EntryKind::Temp && path.exists() {
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
            // Same Claim-vs-Temp guard as cleanup_all (rdlp#572).
            let kind = entry.kind;
            drop(entry); // release advisory lock
            let lock_path = lock_path_for(&path);
            if kind == EntryKind::Temp && path.exists() {
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
        reg.register(&path).unwrap();
        assert!(reg.contains(&path));
        // Lock sidecar should exist while registered.
        assert!(lock_path_for(&path).exists());
        reg.release(&path);
        assert!(!reg.contains(&path));
        // Lock sidecar should be gone after release.
        assert!(!lock_path_for(&path).exists());
    }

    /// #572 regression: `register` must refuse when another handle already
    /// holds the sidecar lock, instead of silently registering a second,
    /// unlocked claim.
    ///
    /// `flock` locks are per-open-file-description: a second `open()` +
    /// `try_lock_exclusive()` on the SAME file from the SAME process reads
    /// back `Ok(false)` exactly like a real external holder would, which is
    /// what makes this a valid same-process simulation of "another rdlp
    /// process already owns this path" without spawning a child process.
    /// Verified directly below before asserting on `register`.
    #[test]
    fn test_register_refuses_when_lock_held_elsewhere() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp-part.mp4");
        fs::write(&path, b"test").unwrap();
        let lock_path = lock_path_for(&path);

        // Simulate another process: hold the lock on a fd this test controls
        // directly, bypassing TempRegistry entirely.
        let other_holder = File::create(&lock_path).unwrap();
        assert!(
            other_holder.try_lock_exclusive().unwrap(),
            "flock-semantics precondition: the first exclusive lock must succeed"
        );
        let second_fd = File::open(&lock_path).unwrap();
        assert!(
            !second_fd.try_lock_exclusive().unwrap(),
            "flock-semantics precondition: a second fd on the SAME process must be refused, \
             confirming try_lock_exclusive is a reliable same-process stand-in for \
             \"held by another process\""
        );
        drop(second_fd);

        // RED against the unpatched code: register() ignored Ok(false) and
        // registered anyway.
        let reg = TempRegistry::new();
        let result = reg.register(&path);
        assert!(
            matches!(result, Err(RegistryError::HeldElsewhere { .. })),
            "register() must refuse a path whose lock is held elsewhere, got: {result:?}"
        );
        assert!(
            !reg.contains(&path),
            "a refused claim must not be recorded in the registry"
        );

        drop(other_holder);
    }

    #[test]
    fn test_drop_deletes_remaining_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp-tmp-123.mp4");
        fs::write(&path, b"test").unwrap();
        assert!(path.exists());
        {
            let reg = TempRegistry::new();
            reg.register(&path).unwrap();
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
            reg.register(&path).unwrap();
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
        reg.register(&path1).unwrap();
        reg.register(&path2).unwrap();

        reg.cleanup_all();

        assert!(!path1.exists());
        assert!(!path2.exists());
        // Registry is now empty — Drop should be a no-op
        assert!(!reg.contains(&path1));
        assert!(!reg.contains(&path2));
    }

    /// #572 review finding 3: `cleanup_all` must NEVER delete a `Claim`
    /// entry's file — it does not own that file's lifecycle. RED against the
    /// pre-fix code: registering `.rdlp-part` (via `register`, the only
    /// entry point that existed) made it a `cleanup_all` target exactly like
    /// any pipeline temp, so a shutdown mid-download deleted the resumable
    /// partial.
    #[test]
    fn test_cleanup_all_preserves_claimed_file_releases_lock_only() {
        let dir = TempDir::new().unwrap();
        let claimed = dir.path().join("video.rdlp-part.mp4");
        let owned = dir.path().join("owned.rdlp-tmp-abc.mp4");
        fs::write(&claimed, b"partial download bytes").unwrap();
        fs::write(&owned, b"pipeline temp").unwrap();

        let reg = TempRegistry::new();
        reg.claim(&claimed).unwrap();
        reg.register(&owned).unwrap();

        reg.cleanup_all();

        assert!(
            claimed.exists(),
            "cleanup_all must NEVER delete a Claim entry's file"
        );
        assert_eq!(
            fs::read(&claimed).unwrap(),
            b"partial download bytes",
            "claimed file content must survive untouched"
        );
        assert!(
            !owned.exists(),
            "cleanup_all must still delete a Temp entry's file as before"
        );
        // The lock sidecar IS released — a fresh claim on the same path
        // must succeed afterward.
        assert!(!reg.contains(&claimed));
        let reg2 = TempRegistry::new();
        reg2.claim(&claimed)
            .expect("lock must be released so a later claim can succeed");
    }

    /// Code-quality review finding 1: the I/O-error fallback in `acquire`
    /// (sidecar `File::create` fails) must NOT unconditionally overwrite an
    /// existing `Claim` entry. Forces the fallback by replacing the sidecar
    /// with a DIRECTORY (Linux `File::create` on a directory path fails with
    /// EISDIR) after a genuine first claim already holds a real flock, then
    /// asserts the first claim survives and the second is refused.
    #[test]
    fn test_io_error_fallback_does_not_replace_an_existing_claim() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("video.rdlp-part.mp4");
        fs::write(&path, b"partial").unwrap();

        let reg = TempRegistry::new();
        reg.claim(&path).expect("first claim must succeed normally");

        // Force the SECOND acquire's File::create(sidecar) to fail: replace
        // the sidecar with a directory. The first holder's already-open fd
        // is unaffected — Unix allows unlinking a path out from under an
        // open file.
        let lock_path = lock_path_for(&path);
        fs::remove_file(&lock_path).expect("remove real sidecar");
        fs::create_dir(&lock_path).expect("replace sidecar with a directory");

        let result = reg.claim(&path);
        assert!(
            matches!(result, Err(RegistryError::HeldElsewhere { .. })),
            "second claim hitting the I/O-error fallback must still be refused, got: {result:?}"
        );
        assert!(
            reg.contains(&path),
            "the first claim's entry must survive the second's fallback attempt"
        );

        fs::remove_dir(&lock_path).ok(); // cleanup for the directory sidecar
    }

    #[test]
    fn test_cleanup_all_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("c.rdlp-tmp-333.mp4");
        fs::write(&path, b"test").unwrap();

        let reg = TempRegistry::new();
        reg.register(&path).unwrap();
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
        reg.register(&path).unwrap();

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
