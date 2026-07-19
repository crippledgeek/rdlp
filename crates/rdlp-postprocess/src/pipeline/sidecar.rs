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

use crate::pipeline::PipelineMessage;

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
    use super::SidecarOwnership;

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
}
