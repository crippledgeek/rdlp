//! RAII claim on a `.rdlp-part` output path (rdlp#572).
//!
//! Two rdlp processes downloading the same target to the same output path
//! write the same deterministic `.rdlp-part` chunk paths and corrupt each
//! other — a conflict worth reporting, not one worth silently making safe.
//! [`PartLock`] converges onto the advisory-lock mechanism `TempRegistry`
//! already uses for pipeline temp files: holding it IS the ownership claim
//! on the path, and it releases on every exit path (success, cancel, or any
//! early `?` return) via `Drop`, never a manual call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rdlp_postprocess::{RegistryError, TempRegistry};

use super::Orchestrator;
use super::errors::{OrchestratorError, Result};

impl Orchestrator {
    /// Claim the `.rdlp-part` output path and probe its resume point, in
    /// that order — the single choke point both the Single-video path
    /// (`state/mod.rs`) and the playlist-episode path (`playlist/episode.rs`)
    /// route through (rdlp#572 review finding 1: only one of the two used to
    /// claim at all, leaving the other free to collide).
    ///
    /// Claiming BEFORE probing matters: probing may merge/delete legacy
    /// chunk files, which a second racing process must not be allowed to
    /// also touch.
    pub(super) async fn claim_part_and_detect_resume(
        &self,
        part: &Path,
        filesize: Option<u64>,
    ) -> Result<(PartLock, u64)> {
        let lock = PartLock::claim(self.temp_registry(), part.to_path_buf())?;
        let offset = self.detect_resume_point(part, filesize).await?;
        Ok((lock, offset))
    }
}

/// Holds the exclusive advisory claim on one output path for as long as this
/// value is alive. Dropping it releases the claim.
pub struct PartLock {
    registry: Arc<TempRegistry>,
    /// The canonicalized key this claim is registered under — see
    /// [`canonical_lock_key`]. Kept alongside `path` because the registry's
    /// `active` map and lock sidecar are keyed by this, not by whatever
    /// spelling the caller passed in.
    key: PathBuf,
}

impl PartLock {
    /// Claim `path` via `registry`, refusing (as [`OrchestratorError::OutputBusy`])
    /// when the path is already claimed — by another process, OR by another
    /// caller sharing this SAME `registry` (`RdlpClient` hands every spawned
    /// orchestrator the same registry, so "two processes" undersells the
    /// threat model here; see [`TempRegistry::claim`]'s doc comment).
    ///
    /// Registers via [`TempRegistry::claim`], NOT `register` — this is an
    /// ownership lock on a file rdlp does not create or own the lifecycle
    /// of, so a sweep (`cleanup_all`/`Drop`) must release the lock without
    /// ever deleting `path` itself.
    ///
    /// # Errors
    /// Returns [`OrchestratorError::OutputBusy`] when `path` is already
    /// claimed elsewhere.
    pub fn claim(registry: Arc<TempRegistry>, path: PathBuf) -> Result<Self> {
        let key = canonical_lock_key(&path);
        match registry.claim(&key) {
            Ok(()) => Ok(Self { registry, key }),
            Err(RegistryError::HeldElsewhere { .. }) => Err(OrchestratorError::OutputBusy { path }),
        }
    }
}

/// Resolve `path` to the key its claim is registered under: the PARENT
/// directory canonicalized (resolves symlinks, `..`, and relative segments)
/// joined with the file name.
///
/// Canonicalizing `path` itself would fail — the `.rdlp-part` file usually
/// does not exist yet at claim time (that's the point: the claim precedes
/// resume detection and the download). The parent directory DOES exist by
/// then (`Orchestrator::ensure_parent_dir` runs before the caller computes
/// `path`), so canonicalizing it is reliable.
///
/// Without this, two differently-spelled references to the SAME file from
/// the SAME process — a relative path and its absolute equivalent, or a path
/// through a symlinked directory — would produce two different sidecar
/// paths and never collide, i.e. a false negative on the very case #572
/// exists to catch (as well as a false-positive risk on separate targets
/// that happen to share only a relative spelling). A parent directory that
/// cannot be canonicalized (rare: symlink race, permissions) falls back to
/// `path` unchanged rather than failing the claim outright — a narrower
/// guarantee, not a crash, and worth documenting rather than promising more
/// than this fn can deliver.
fn canonical_lock_key(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name() else {
        return path.to_path_buf();
    };
    match path.parent().map(std::fs::canonicalize) {
        Some(Ok(parent)) => parent.join(file_name),
        _ => path.to_path_buf(),
    }
}

impl Drop for PartLock {
    fn drop(&mut self) {
        self.registry.release(&self.key);
    }
}

impl std::fmt::Debug for PartLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `registry` is an opaque `Arc<TempRegistry>` with nothing debug-worthy
        // beyond what `key` already says about this claim.
        f.debug_struct("PartLock")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claiming a free path succeeds and holds the registry's `.lock` sidecar.
    #[test]
    fn claim_succeeds_and_registers() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("Title.rdlp-part.mp4");
        let key = canonical_lock_key(&path);
        let registry = Arc::new(TempRegistry::new());

        let lock = PartLock::claim(Arc::clone(&registry), path).expect("must claim");
        assert!(registry.contains(&key));
        drop(lock);
        assert!(
            !registry.contains(&key),
            "dropping the guard must release the claim"
        );
    }

    /// A path already claimed on a SEPARATE registry instance — what a
    /// second rdlp process looks like — is refused as `OutputBusy`.
    #[test]
    fn claim_refuses_when_held_by_another_registry() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("Title.rdlp-part.mp4");

        let first_registry = Arc::new(TempRegistry::new());
        let _held = PartLock::claim(Arc::clone(&first_registry), path.clone())
            .expect("first claim must succeed");

        let second_registry = Arc::new(TempRegistry::new());
        let result = PartLock::claim(second_registry, path.clone());
        assert!(
            matches!(result, Err(OrchestratorError::OutputBusy { path: ref p }) if *p == path),
            "second claim must be refused as OutputBusy, got: {result:?}"
        );
    }

    /// Review finding 2: two orchestrators SHARING one registry (what
    /// `RdlpClient` actually hands two queued desktop downloads) must NOT
    /// silently both succeed — the second claim on the SAME registry is
    /// refused exactly like a separate-registry (separate-process) claim.
    /// RED against `TempRegistry::register`'s idempotent semantics: that
    /// path made a same-registry re-claim a silent `Ok`.
    #[test]
    fn claim_refuses_when_held_by_the_same_registry() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("Title.rdlp-part.mp4");
        let shared_registry = Arc::new(TempRegistry::new());

        let _held = PartLock::claim(Arc::clone(&shared_registry), path.clone())
            .expect("first claim must succeed");

        let result = PartLock::claim(Arc::clone(&shared_registry), path.clone());
        assert!(
            matches!(result, Err(OrchestratorError::OutputBusy { path: ref p }) if *p == path),
            "second claim on the SAME registry must be refused as OutputBusy, got: {result:?}"
        );
    }

    /// `canonical_lock_key` resolves a symlinked-parent spelling to the SAME
    /// key as the real path — the canonicalization LOW fix, tested directly
    /// against the helper rather than end-to-end through `TempRegistry`.
    ///
    /// End-to-end collision via `PartLock::claim` was tried first and found
    /// to be a non-discriminating oracle: `flock` is inode-based on this
    /// platform, so the OS already unifies the two spellings' `.lock`
    /// sidecars regardless of what this fn does — that end-to-end test
    /// passed identically with `canonical_lock_key` stubbed out to the
    /// identity function. This test targets the actual, narrower value of
    /// the fix: keeping `TempRegistry`'s in-memory bookkeeping (`contains`,
    /// the map key) consistent for a symlinked spelling, which the OS-level
    /// collision does NOT provide on its own.
    #[test]
    #[cfg(unix)]
    fn canonical_lock_key_resolves_symlinked_parent_to_the_real_path() {
        let real_dir = tempfile::TempDir::new().unwrap();
        let link_root = tempfile::TempDir::new().unwrap();
        let link = link_root.path().join("alias");
        std::os::unix::fs::symlink(real_dir.path(), &link).unwrap();

        let via_real = real_dir.path().join("Title.rdlp-part.mp4");
        let via_symlink = link.join("Title.rdlp-part.mp4");
        assert_ne!(
            via_real, via_symlink,
            "precondition: the two spellings must be textually different"
        );

        let expected = std::fs::canonicalize(real_dir.path())
            .unwrap()
            .join("Title.rdlp-part.mp4");
        assert_eq!(
            canonical_lock_key(&via_symlink),
            expected,
            "symlinked spelling must resolve to the real path's canonical key"
        );
        assert_eq!(
            canonical_lock_key(&via_real),
            expected,
            "real path must resolve to its own canonical key unchanged"
        );
    }
}
