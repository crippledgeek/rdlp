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
/// uncommitted path it issued via [`temp_path`] plus any `current_files` and
/// superseded `temp_files`, and releases each from [`TempRegistry`]. This
/// prevents partial recode output and
/// intermediate files from leaking when a job is cancelled mid-pipeline (#335, #404).
/// [`TempRegistry`] remains the crash/stale backstop for the case where the
/// process dies before `Drop` runs.
///
/// When post-processing a pre-existing local file (`process_local_file`, #414),
/// use [`new_borrowing`] so the user's original file is never deleted — neither
/// on success nor on cancel. The download path uses [`new`] (empty borrowed set)
/// and retains its existing behaviour unchanged.
pub struct FileTracker {
    /// The live files currently being processed.
    pub(crate) current_files: Vec<PathBuf>,
    /// Files that have been superseded by a later stage's output.
    pub(crate) temp_files: Vec<PathBuf>,
    /// Every path handed out by [`temp_path`] — the uncommitted-output set
    /// deleted on cancel-drop.
    issued: Vec<PathBuf>,
    /// Caller-supplied, user-owned inputs that MUST NOT be deleted — neither on
    /// success ([`cleanup`]) nor on cancel ([`Drop`]) — and are never moved into
    /// the delete-set by [`replace`]. Empty for the download path (rdlp owns its
    /// temp-named files); populated by `process_local_file` for a pre-existing
    /// local file (#414). Encodes the ownership rule: delete what we created,
    /// preserve what the user brought us.
    ///
    /// Each entry is `(raw, canonical)`: the raw path as supplied by the caller,
    /// plus the result of [`std::fs::canonicalize`] at [`new_borrowing`] time
    /// (when the file is guaranteed to exist, so canonicalize succeeds).
    /// Storing the canonical form enables [`is_borrowed`] to match equivalent
    /// but textually-distinct representations (symlinks, resolved mount points)
    /// without relying on the file still existing at check time (#416 M2).
    borrowed: Vec<(PathBuf, Option<PathBuf>)>,
    /// Disarms the `Drop` cleanup once the job succeeds (`commit`/`cleanup`).
    committed: bool,
    temp_registry: Arc<TempRegistry>,
}

impl FileTracker {
    /// Shared constructor — the field list lives in exactly ONE place. Both
    /// [`new`] and [`new_borrowing`] delegate here so changes to the struct
    /// fields are made only once.
    const fn with_borrowed(
        files: Vec<PathBuf>,
        borrowed: Vec<(PathBuf, Option<PathBuf>)>,
        temp_registry: Arc<TempRegistry>,
    ) -> Self {
        Self {
            current_files: files,
            temp_files: Vec::new(),
            issued: Vec::new(),
            borrowed,
            committed: false,
            temp_registry,
        }
    }

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
    /// constructing the tracker. To post-process a pre-existing user-owned file
    /// (which must NOT be deleted), use [`new_borrowing`] instead of satisfying
    /// this precondition.
    pub const fn new(files: Vec<PathBuf>, temp_registry: Arc<TempRegistry>) -> Self {
        Self::with_borrowed(files, Vec::new(), temp_registry)
    }

    /// Like [`new`], but marks every initial file as a **borrowed** user-owned
    /// input: never deleted (success or cancel), never queued for deletion by
    /// [`replace`]. For post-processing a pre-existing local file (#414) rather
    /// than a file rdlp downloaded.
    ///
    /// Each borrowed path is canonicalized at construction time (the files exist
    /// here, so [`std::fs::canonicalize`] succeeds). The canonical form is stored
    /// alongside the raw path so [`is_borrowed`] can match equivalent-but-
    /// textually-distinct representations such as symlinks at cancel/cleanup time,
    /// when the file may have been moved and canonicalize on the candidate would
    /// fail (#416 M2).
    #[must_use]
    pub fn new_borrowing(files: Vec<PathBuf>, temp_registry: Arc<TempRegistry>) -> Self {
        let borrowed = files
            .iter()
            .map(|raw| {
                let canonical = std::fs::canonicalize(raw).ok();
                (raw.clone(), canonical)
            })
            .collect();
        Self::with_borrowed(files, borrowed, temp_registry)
    }

    /// Whether `path` is a borrowed (user-owned) input that must never be
    /// deleted or released by this tracker (#414, #416 M2).
    ///
    /// Two checks, in order:
    /// 1. **Raw `PathBuf` equality** — cheap, no syscall; covers the common case
    ///    where the pipeline round-trips the path unchanged.
    /// 2. **Canonical equality** — canonicalize `path` and compare against the
    ///    stored canonical form (computed at [`new_borrowing`] time when the file
    ///    existed). Catches equivalent representations such as symlinks and
    ///    resolved mount points.
    ///
    /// When `std::fs::canonicalize` can't run (Drop/cleanup, file already gone),
    /// step 2 simply doesn't match and the result is determined by step 1 —
    /// never worse than the original exact-match implementation.
    fn is_borrowed(&self, path: &Path) -> bool {
        // Step 1: raw equality — O(n) but no syscall.
        if self.borrowed.iter().any(|(raw, _)| raw.as_path() == path) {
            return true;
        }
        // Step 2: equivalent representation (e.g. a symlink): if the candidate
        // resolves to the same real path as a borrowed input's stored canonical,
        // it's borrowed. `canonicalize` needs the file to exist, so it can fail
        // in Drop/cleanup (file already gone) — in that case we simply fall
        // through to the raw result above, never worse than exact-match.
        if let Ok(candidate_canon) = std::fs::canonicalize(path)
            && self
                .borrowed
                .iter()
                .any(|(_, stored)| stored.as_deref() == Some(candidate_canon.as_path()))
        {
            return true;
        }
        false
    }

    /// Promote `new_files` to `current_files`, moving the old current files
    /// into `temp_files`.
    ///
    /// Borrowed (user-owned) inputs are dropped from tracking rather than
    /// queued for deletion (#414).
    pub fn replace(&mut self, new_files: Vec<PathBuf>) {
        let old = std::mem::replace(&mut self.current_files, new_files);
        for f in old {
            // #414: a borrowed (user-owned) input is never deleted — drop it from
            // tracking instead of queueing it in `temp_files`.
            if self.is_borrowed(&f) {
                continue;
            }
            self.temp_files.push(f);
        }
    }

    /// Mark a path as a temp file without changing `current_files`.
    pub fn mark_temp(&mut self, path: PathBuf) {
        debug_assert!(
            !self.is_borrowed(&path),
            "a borrowed (user-owned) input must never be marked temp (#414)"
        );
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

    /// Delete all temp files and unregister them from [`TempRegistry`], then
    /// disarm the cancel-cleanup [`Drop`] by committing. Does NOT rename the
    /// surviving `current_files` to clean names — the pipeline returns them
    /// temp-named (`*.rdlp-tmp-{uuid}.*`) and the orchestrator performs the
    /// single final rename to the user-visible name (#406 Option X).
    ///
    /// The surviving `current_files` are released from the [`TempRegistry`] here
    /// (so no stale entry/lock lingers) but are NOT renamed — the orchestrator
    /// performs the single final rename to the clean name and owns the survivor's
    /// lifecycle thereafter.
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

        // The surviving `current_files` are returned temp-named for the
        // orchestrator to finalize (#406 Option X). Release them from the
        // registry HERE — registry bookkeeping stays in the pipeline, which owns
        // the `TempRegistry`. This drops the advisory lock + removes the `.lock`
        // sidecar so no stale entry survives; the orchestrator then renames the
        // now-unregistered temp file to its clean user-visible name.
        // Skip borrowed paths: a borrowed source was never registered with the
        // registry, so releasing it would be a contract violation.
        for file in &self.current_files {
            if !self.is_borrowed(file) {
                self.temp_registry.release(file.as_path());
            }
        }

        // Successful completion: disarm the cancel-cleanup Drop.
        self.committed = true;
    }
}

impl Drop for FileTracker {
    /// Graceful-cancel cleanup: if the job did not [`commit`](FileTracker::commit)
    /// (or [`cleanup`](FileTracker::cleanup), which commits last), delete every
    /// uncommitted issued path, any `current_files`, AND any `temp_files`
    /// (superseded working files / the original download), releasing each from
    /// the [`TempRegistry`] (#335, #404). This mirrors the success-path
    /// `cleanup()`, which already drains `temp_files` — cancel is now symmetric.
    // Safe: synchronous remove_file in Drop is unavoidable — Drop cannot be
    // async, and this mirrors the existing justification on `cleanup()` and the
    // registry's own shutdown hook.
    #[allow(clippy::disallowed_methods)]
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Visible guard (#339): `issued` + `current_files` SHOULD be temp-
        // namespaced. If a caller violated the `new()` precondition by passing a
        // non-temp file, warn (but still delete — the cancel intent is to clear
        // the working set). `temp_files` is deliberately EXCLUDED from this check:
        // it legitimately holds real-named superseded files (e.g. the original
        // download moved here by `replace()`), so a non-temp name there is
        // expected, not a contract violation. Borrowed paths are excluded entirely
        // — they are user-owned and must never be deleted (#414). Drop never panics.
        for path in self.issued.iter().chain(self.current_files.iter()) {
            if !is_temp_named(path) && !self.is_borrowed(path) {
                log::warn!(
                    "FileTracker::Drop deleting non-temp-named file {} — unexpected; \
                     current_files should be temp-namespaced",
                    path.display(),
                );
            }
        }
        // Delete all three sets (issued + current_files + superseded temp_files),
        // excluding borrowed (user-owned) inputs which must never be deleted (#414).
        // Collect into an owned Vec to avoid borrowing self while mutating it.
        let to_delete: Vec<PathBuf> = self
            .issued
            .iter()
            .chain(self.current_files.iter())
            .chain(self.temp_files.iter())
            .filter(|p| !self.is_borrowed(p)) // #414: never delete borrowed
            .cloned()
            .collect();
        for path in to_delete {
            // The sets may overlap (mark_temp re-files issued paths); the
            // exists() guard + idempotent release make this safe to run per path.
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

    /// Slice 2 (#406 Option X): `cleanup()` MUST NOT rename the temp-named
    /// survivor — it returns it as-is and the orchestrator does the final rename.
    #[test]
    fn cleanup_no_longer_renames_survivor() {
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        // The survivor is the current_files entry — already temp-named (as the
        // orchestrator UUID-renames inputs before handing them to FileTracker).
        let survivor = dir.path().join("Title.rdlp-tmp-aaaa.mkv");
        fs::write(&survivor, b"final").unwrap();
        let mut tracker = FileTracker::new(vec![survivor.clone()], reg);
        tracker.cleanup();
        // Registry release assertion: the survivor was passed via FileTracker::new()
        // (not via temp_path()), so it was never registered — reg.contains() would
        // be vacuously false before and after. The release-on-cleanup path for
        // temp_path()-registered survivors is covered by TempRegistry's own
        // test_register_and_release test and by the registry Drop semantics tests.
        // (a) The file STILL EXISTS at its .rdlp-tmp- name (NOT renamed to Title.mkv).
        assert!(survivor.exists(), "cleanup() must NOT delete the survivor");
        assert!(
            !dir.path().join("Title.mkv").exists(),
            "cleanup() must NOT create a clean-named file"
        );
        // (b) current_files[0] still contains the temp marker.
        assert!(
            tracker.current_files[0]
                .to_str()
                .unwrap()
                .contains(".rdlp-tmp-"),
            "current_files[0] must still be temp-named after cleanup()"
        );
        // (c) committed is true (observable: a second drop must not delete the survivor).
        drop(tracker);
        assert!(
            survivor.exists(),
            "committed tracker must not delete survivor on drop"
        );
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
    fn drop_uncommitted_deletes_temp_files() {
        // A stage supersedes the original input via replace(), moving it into
        // temp_files. On an uncommitted (cancel) drop, that superseded file MUST
        // be deleted — this is the #404 source-.mp4 leak.
        let dir = TempDir::new().unwrap();
        let reg = Arc::new(TempRegistry::new());
        let original = dir.path().join("video.mp4");
        fs::write(&original, b"orig").unwrap();
        {
            let mut tracker = FileTracker::new(vec![original.clone()], reg);
            let promoted = dir.path().join("video.rdlp-tmp-zzz.mp4");
            fs::write(&promoted, b"new").unwrap();
            tracker.replace(vec![promoted]); // original -> temp_files
            // dropped here WITHOUT commit() -> cancel semantics
        }
        assert!(
            !original.exists(),
            "superseded original in temp_files must be deleted on uncommitted drop (#404)"
        );
    }

    /// Slice 2 (#406): the pipeline input is seam-named (`*.rdlp-tmp-*`), so
    /// `FileTracker::Drop`'s `#339` non-temp-named warning CANNOT fire for it.
    /// Pins that `is_temp_named` returns `true` for a seam-style name and
    /// `false` for a clean name, i.e. the contract the caller (orchestrator)
    /// must satisfy before handing the file to `FileTracker::new`.
    #[test]
    fn seam_name_is_temp_named_so_339_warning_cannot_fire() {
        // Seam name produced by `seam_path()` — must be temp-named.
        assert!(
            is_temp_named(Path::new(
                "Title.rdlp-tmp-abc123def456abc123def456abc123de.mp4"
            )),
            "seam-style name must be recognised as temp-named"
        );
        // Dotted-stem variant (the common real-world case).
        assert!(
            is_temp_named(Path::new(
                "My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678.mkv"
            )),
            "dotted-stem seam name must be recognised as temp-named"
        );
        // Clean names must NOT be recognised as temp-named.
        assert!(
            !is_temp_named(Path::new("Title.mp4")),
            "clean name must NOT be temp-named"
        );
        assert!(
            !is_temp_named(Path::new("My.Video.mkv")),
            "dotted clean name must NOT be temp-named"
        );
    }

    // ── #416 M2 canonical borrowed-path matching tests ─────────────────────

    /// Primary regression guard for #416 M2: if the user hands us a SYMLINK
    /// path as the borrowed input but the pipeline internally resolves to the
    /// canonical path (puts it in `current_files`), a cancel-Drop must NOT
    /// delete the underlying real file.
    ///
    /// `PathBuf` equality is byte-for-byte, so `link.mp4` ≠ `source.mp4` under
    /// exact equality even though both point to the same inode. The fix must
    /// canonicalize stored borrowed paths at `new_borrowing` time so `Drop` can
    /// match via the canonical form.
    ///
    /// Against the current exact-equality `is_borrowed` this test MUST fail
    /// (the real source is deleted). After the fix it MUST pass.
    #[test]
    #[cfg(unix)] // symlinks are a POSIX concept; Windows requires privilege escalation
    fn cancel_drop_survives_canonical_when_borrowed_via_symlink() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let reg = test_registry();

        // The real file — the underlying target.
        let real_src = dir.path().join("source.mp4");
        fs::write(&real_src, b"user data").unwrap();

        // A symlink that resolves to the same inode.
        let link_src = dir.path().join("source_link.mp4");
        symlink(&real_src, &link_src).expect("symlink creation failed");

        // The user hands us the SYMLINK path as the borrowed input.
        let mut t = FileTracker::new_borrowing(vec![link_src], reg);

        // A pipeline stage resolves the symlink and puts the CANONICAL path
        // in current_files. Drop's is_borrowed(real_src) returns false under
        // exact equality → deletes real_src — the data-loss bug (#416 M2).
        t.current_files = vec![real_src.clone()];
        // uncommitted drop → cancel semantics
        drop(t);

        assert!(
            real_src.exists(),
            "borrowed source must survive cancel-Drop when borrowed via symlink \
             but Drop sees the canonical path (#416 M2)"
        );
    }

    /// Symmetric variant: tracker borrowed with the canonical path, pipeline
    /// returns a SYMLINK to it in `current_files`. The symlink path itself may be
    /// deleted (it is not the user's real file), but the real file must survive.
    ///
    /// NOTE: this variant passes even under exact equality (deleting the symlink
    /// path removes the dangling symlink, not the real file), so it is included
    /// as a regression guard to ensure the fix does not break this case.
    #[test]
    #[cfg(unix)]
    fn cancel_drop_preserves_real_file_when_drop_sees_symlink_path() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let reg = test_registry();

        let real_src = dir.path().join("source.mp4");
        fs::write(&real_src, b"user data").unwrap();

        let link_src = dir.path().join("source_link.mp4");
        symlink(&real_src, &link_src).expect("symlink creation failed");

        // Tracker borrowed with the canonical path.
        let mut t = FileTracker::new_borrowing(vec![real_src.clone()], reg);

        // Drop sees the symlink path in current_files.
        t.current_files = vec![link_src];
        drop(t);

        assert!(
            real_src.exists(),
            "real file must survive even when Drop only sees a symlink path (#416 M2)"
        );
    }

    // ── #414 borrowed-input tests ────────────────────────────────────────────

    /// A borrowed (user-owned) file must survive an uncommitted (cancel) Drop.
    #[test]
    fn test_drop_does_not_delete_borrowed_input() {
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        let src = dir.path().join("localfile.mp4");
        fs::write(&src, b"user data").unwrap();
        {
            let _t = FileTracker::new_borrowing(vec![src.clone()], reg);
            // uncommitted drop here → cancel semantics
        }
        assert!(src.exists(), "borrowed user input must survive cancel-Drop");
    }

    /// After a `replace()`, the borrowed source must NOT be queued for deletion.
    /// On an uncommitted Drop, the temp output is deleted but the source survives.
    #[test]
    fn test_replace_keeps_borrowed_then_drop() {
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        let src = dir.path().join("localfile.mp4");
        fs::write(&src, b"user data").unwrap();
        let out;
        {
            let mut t = FileTracker::new_borrowing(vec![src.clone()], reg);
            out = t.temp_path(&src, "mkv");
            fs::write(&out, b"recoded").unwrap();
            t.replace(vec![out.clone()]);
            // uncommitted drop → cancel: out (current_files) deleted, src preserved
        }
        assert!(
            src.exists(),
            "borrowed source must survive replace + cancel-Drop"
        );
        assert!(
            !out.exists(),
            "non-borrowed issued temp must be deleted on cancel-Drop"
        );
    }

    /// After a successful `cleanup()`, the borrowed source AND the recode output survive.
    #[test]
    fn test_cleanup_keeps_borrowed_after_replace() {
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        let src = dir.path().join("localfile.mp4");
        fs::write(&src, b"user data").unwrap();
        let out;
        {
            let mut t = FileTracker::new_borrowing(vec![src.clone()], reg);
            out = t.temp_path(&src, "mkv");
            fs::write(&out, b"recoded").unwrap();
            t.replace(vec![out.clone()]);
            t.cleanup();
        }
        assert!(
            src.exists(),
            "borrowed source must survive successful cleanup"
        );
        assert!(out.exists(), "the survivor (recode output) must be kept");
    }

    /// #404 regression guard: a file owned via the NORMAL `new()` constructor
    /// (rdlp-owned temp) must still be deleted on an uncommitted Drop.
    #[test]
    fn test_non_borrowed_temp_still_deleted_on_drop() {
        let dir = TempDir::new().unwrap();
        let reg = test_registry();
        let tmp = dir.path().join("video.rdlp-tmp-abc.mp4");
        fs::write(&tmp, b"partial").unwrap();
        {
            let _t = FileTracker::new(vec![tmp.clone()], reg);
            // uncommitted drop → cancel semantics
        }
        assert!(
            !tmp.exists(),
            "non-borrowed owned temp must still be deleted on cancel"
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
