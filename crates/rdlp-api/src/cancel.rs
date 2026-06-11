//! Cancellation disposition: whether a cancel should KEEP the resumable
//! download partial (an interrupt — industry standard for download tools) or
//! DISCARD it (an explicit cancel — the desktop "clear job" intent). #413.
//!
//! Shared (cheap `Arc` clone) between [`crate::DownloadHandle`] and the
//! orchestrator so the signal-handler task can set the intent before cancelling
//! the [`tokio_util::sync::CancellationToken`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const DISCARD: u8 = 0;
#[allow(dead_code)] // wired into Orchestrator + DownloadHandle in Tasks 2-3
const KEEP: u8 = 1;

/// Keep-vs-discard intent for the next cancellation.
///
/// **Default is `Discard`** — this preserves the desktop's existing
/// delete-on-cancel behavior, because the desktop cancels via the raw
/// `CancellationToken` and never goes through [`crate::DownloadHandle::cancel`].
/// Only an explicit interrupt (`set_keep`) opts into keeping the partial.
#[allow(dead_code)] // wired into Orchestrator + DownloadHandle in Tasks 2-3
#[allow(clippy::redundant_pub_crate)] // module is pub(crate); items use pub(crate) for explicitness
#[derive(Clone)]
pub(crate) struct CancelDisposition(Arc<AtomicU8>);

impl Default for CancelDisposition {
    fn default() -> Self {
        Self(Arc::new(AtomicU8::new(DISCARD)))
    }
}

#[allow(dead_code)] // wired into Orchestrator + DownloadHandle in Tasks 2-3
#[allow(clippy::redundant_pub_crate)]
impl CancelDisposition {
    /// A fresh disposition defaulting to `Discard`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Mark the next cancel as an interrupt: KEEP the resumable `.rdlp-part`.
    pub(crate) fn set_keep(&self) {
        self.0.store(KEEP, Ordering::SeqCst);
    }

    /// True when the resumable partial should be kept (interrupt).
    pub(crate) fn should_keep(&self) -> bool {
        self.0.load(Ordering::SeqCst) == KEEP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_to_discard() {
        assert!(!CancelDisposition::new().should_keep());
        assert!(!CancelDisposition::default().should_keep());
    }

    #[test]
    fn test_set_keep_flips_to_keep() {
        let d = CancelDisposition::new();
        d.set_keep();
        assert!(d.should_keep());
    }

    #[test]
    fn test_clone_shares_state() {
        let a = CancelDisposition::new();
        let b = a.clone();
        a.set_keep();
        assert!(b.should_keep(), "clone must observe the shared store");
    }
}
