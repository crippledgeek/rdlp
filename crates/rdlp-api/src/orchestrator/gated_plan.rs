//! A [`DownloadPlan`] that has passed the unusable-`FFmpeg` gate.
//!
//! The gate used to be a method two transition sites remembered to call.
//! Deleting either call left every test green while reintroducing rdlp#727's
//! defect — a resumed merge downloading both streams and finalizing only the
//! video, silently audio-less. Review found exactly that, twice, which is the
//! evidence that "remember to call it" is not a mechanism.
//!
//! So the checked plan is a distinct type whose field is private to this
//! module. [`GatedPlan::new`] is the only way to obtain one and it runs the
//! check, so a plan reaching the download without being checked is a compile
//! error rather than a silent regression. Nothing else in the crate can
//! construct one — not a sibling module, not a test.

use super::errors::Result;
use super::{DownloadPlan, Orchestrator};

/// A plan that has been checked against the linked `FFmpeg`.
///
/// Held by [`DownloadPhase::Preparing`](super::DownloadPhase::Preparing),
/// which is the single door into the download.
#[derive(Debug)]
pub struct GatedPlan(Box<DownloadPlan>);

impl GatedPlan {
    /// Check a plan and, if it can run, wrap it.
    ///
    /// # Errors
    ///
    /// [`OrchestratorError::FFmpegAbiMismatch`](super::errors::OrchestratorError::FFmpegAbiMismatch)
    /// when the plan needs `FFmpeg` and the linked `FFmpeg` is unusable. This
    /// is deliberately before the download rather than after it: refusing here
    /// costs nothing, while refusing later would abandon a complete download
    /// under its `.rdlp-tmp-` seam name for `cleanup_stale` to delete.
    pub(super) fn new(orchestrator: &Orchestrator, plan: Box<DownloadPlan>) -> Result<Self> {
        orchestrator.refuse_plan_needing_unusable_ffmpeg(&plan)?;
        Ok(Self(plan))
    }

    /// The plan, for the phases that act on it.
    pub(super) fn into_inner(self) -> Box<DownloadPlan> {
        self.0
    }
}

impl std::ops::Deref for GatedPlan {
    type Target = DownloadPlan;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
