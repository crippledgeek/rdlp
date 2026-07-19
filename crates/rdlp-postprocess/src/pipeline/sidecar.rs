//! Who owns the sidecar files a stage discovers next to the media file.
//!
//! Several stages locate companion files by stem-matching beside the media
//! file — `ThumbnailStage` finds `{stem}.jpg`, `SubtitleStage` finds
//! `{stem}.{lang}.srt` — and then delete them once done, unless the user asked
//! to keep them (`--write-thumbnail` / `--write-subtitles`).
//!
//! Stem-matching cannot by itself tell "a file rdlp downloaded" from "a file
//! the user already had". For a **borrowed** input (`rdlp /home/me/clip.mp4`,
//! local-file post-processing) the neighbouring `clip.jpg` / `clip.en.srt` is
//! the USER'S OWN FILE, and deleting it is silent data loss — the #414
//! incident class (local-file source preservation) applied to sidecars rather
//! than to the media file.
//!
//! This lives here, not on a stage, because the question is stage-agnostic:
//! the same predicate governs every stage that discovers-then-deletes a
//! companion file. It was originally a private helper on `ThumbnailStage`;
//! `SubtitleStage` had the identical defect and no gate at all, which is what
//! prompted hoisting it.

use std::path::{Path, PathBuf};

use crate::pipeline::PipelineMessage;

/// A companion file located by stem-matching next to the media file.
///
/// Wraps the path so its ownership CANNOT be skipped. `FileTracker::mark_temp`
/// takes a `PathBuf`, so a `DiscoveredSidecar` cannot reach it without first
/// going through [`Self::into_disposable`] — a stage that forgets to ask who
/// owns the file gets a type error instead of silently deleting a user's file.
///
/// This exists because the convention alone was not enough: the same defect
/// shipped or nearly shipped three times in one session (#548's keep-container
/// guard, the thumbnail embed path, and `SubtitleStage`), each time as "a stage
/// discovers a companion file, then deletes it without asking whose it is".
/// Two of the three were caught only because a reviewer went looking (#553).
///
/// Reading the path for any NON-destructive purpose (embedding, probing,
/// logging) is unrestricted via [`Self::path`] — the type constrains deletion,
/// not use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSidecar(PathBuf);

impl DiscoveredSidecar {
    /// Wrap a stem-matched companion file whose ownership is not yet known.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// Borrow the path for non-destructive use.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Yield a deletable path, but only when rdlp owns the file.
    ///
    /// `None` means the file is the user's and must be left alone — the caller
    /// simply has nothing to mark temp.
    #[must_use]
    pub fn into_disposable(self, ownership: SidecarOwnership) -> Option<PathBuf> {
        ownership.is_disposable().then_some(self.0)
    }
}

/// Who owns the sidecar files sitting next to the media file.
///
/// Deliberately an enum rather than a `bool`: `is_disposable(msg) -> bool`
/// reads as a yes/no at the call site and loses *why*, and a boolean this
/// close to a `remove_file` is exactly the kind of flag that gets inverted in
/// a refactor without anyone noticing. Naming both states makes the
/// data-loss-relevant one impossible to read past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarOwnership {
    /// rdlp downloaded these files itself, so cleaning them up is correct.
    RdlpOwned,
    /// The user brought these files. Never delete them, on any path.
    UserOwned,
}

impl SidecarOwnership {
    /// Resolve ownership for this pipeline run.
    ///
    /// The question is whether the RUN started from a user-owned file, not
    /// whether the file currently in hand is borrowed. By the time a late
    /// stage executes, a container-changing stage has usually replaced the
    /// borrowed input with an rdlp-created derivative (`RemuxStage` turns the
    /// user's `clip.mp4` into `clip.ts`), so a per-path check answers
    /// "not borrowed" and deletes the user's sidecar anyway. That distinction
    /// is invisible to a single-stage test and was caught only by running the
    /// release binary.
    #[must_use]
    pub const fn of(msg: &PipelineMessage) -> Self {
        if msg.tracker.has_borrowed_inputs() {
            Self::UserOwned
        } else {
            Self::RdlpOwned
        }
    }

    /// Whether a discovered sidecar may be marked temp for deletion.
    #[must_use]
    pub const fn is_disposable(self) -> bool {
        matches!(self, Self::RdlpOwned)
    }
}

#[cfg(test)]
mod tests {
    use super::{DiscoveredSidecar, SidecarOwnership};
    use std::path::{Path, PathBuf};

    #[test]
    fn rdlp_owned_sidecars_are_disposable() {
        assert!(SidecarOwnership::RdlpOwned.is_disposable());
    }

    /// The load-bearing direction: a user-owned sidecar must never be
    /// disposable. Inverting `is_disposable` fails here.
    #[test]
    fn user_owned_sidecars_are_never_disposable() {
        assert!(!SidecarOwnership::UserOwned.is_disposable());
    }

    #[test]
    fn rdlp_owned_sidecar_yields_a_deletable_path() {
        let sidecar = DiscoveredSidecar::new(PathBuf::from("/tmp/video.jpg"));
        assert_eq!(
            sidecar.into_disposable(SidecarOwnership::RdlpOwned),
            Some(PathBuf::from("/tmp/video.jpg"))
        );
    }

    /// The data-loss direction: a user-owned sidecar yields NO path, so there
    /// is nothing the caller could hand to `mark_temp`.
    #[test]
    fn user_owned_sidecar_yields_no_deletable_path() {
        let sidecar = DiscoveredSidecar::new(PathBuf::from("/home/me/myvideo.jpg"));
        assert_eq!(sidecar.into_disposable(SidecarOwnership::UserOwned), None);
    }

    /// Non-destructive reads stay unrestricted — the type constrains deletion,
    /// not use (the embed path still needs to read the image).
    #[test]
    fn path_is_readable_without_resolving_ownership() {
        let sidecar = DiscoveredSidecar::new(PathBuf::from("/tmp/video.jpg"));
        assert_eq!(sidecar.path(), Path::new("/tmp/video.jpg"));
    }
}
