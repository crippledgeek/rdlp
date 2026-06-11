//! Download handle and ID types.
//!
//! [`DownloadId`] is an opaque identifier for correlating events with
//! downloads. [`DownloadHandle`] is the primary interface for interacting
//! with a running download — receiving events, cancelling, and waiting
//! for the final result.

use crate::cancel::CancelDisposition;
use crate::errors::RdlpApiError;
use crate::events::Event;
use crate::result::DownloadResult;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Opaque download identifier.
///
/// Uses an internal `u64` counter. The representation is intentionally opaque
/// to allow future changes (e.g., UUID) without breaking the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadId(u64);

/// Global counter for generating unique download IDs.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl DownloadId {
    /// Create the next unique download ID.
    pub(crate) fn next() -> Self {
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// Get the raw numeric value.
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for DownloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Handle to a running download.
///
/// Provides access to the event stream, cancellation, and final result.
/// The download runs in a background Tokio task.
///
/// # Ownership
///
/// - [`events()`](Self::events) borrows the receiver for polling.
/// - [`cancel()`](Self::cancel) is non-blocking and idempotent.
/// - [`interrupt()`](Self::interrupt) is like cancel but keeps the resumable partial.
/// - [`wait()`](Self::wait) consumes the handle and returns the final result.
pub struct DownloadHandle {
    id: DownloadId,
    events_rx: mpsc::Receiver<Event>,
    cancel_token: CancellationToken,
    cancel_disposition: CancelDisposition,
    join_handle: JoinHandle<Result<DownloadResult, RdlpApiError>>,
}

impl DownloadHandle {
    /// Create a new handle (crate-internal).
    #[allow(clippy::missing_const_for_fn)] // Arc inside CancelDisposition prevents const
    pub(crate) fn new(
        id: DownloadId,
        events_rx: mpsc::Receiver<Event>,
        cancel_token: CancellationToken,
        cancel_disposition: CancelDisposition,
        join_handle: JoinHandle<Result<DownloadResult, RdlpApiError>>,
    ) -> Self {
        Self {
            id,
            events_rx,
            cancel_token,
            cancel_disposition,
            join_handle,
        }
    }

    /// The download ID for this handle.
    #[must_use]
    pub const fn id(&self) -> DownloadId {
        self.id
    }

    /// Mutable reference to the event receiver for polling.
    ///
    /// Use `recv().await` to get the next event, or use in a `tokio::select!`.
    pub const fn events(&mut self) -> &mut mpsc::Receiver<Event> {
        &mut self.events_rx
    }

    /// Clone of the cancellation token.
    ///
    /// Useful for storing in external state (e.g., Tauri `DownloadManager`).
    #[must_use]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// Cancel the download.
    ///
    /// Non-blocking and idempotent. The orchestrator will emit
    /// [`Event::Cancelled`] and clean up resources.
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    /// Interrupt the download (Ctrl+C / SIGINT semantics): KEEP the resumable
    /// `.rdlp-part` for resume, then cancel. Non-blocking, idempotent. #413.
    pub fn interrupt(&self) {
        self.cancel_disposition.set_keep();
        self.cancel_token.cancel();
    }

    /// A detachable interrupt trigger that can be moved into a signal-handler
    /// task while the handle retains `events()`/`wait()`. #413.
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle {
            token: self.cancel_token.clone(),
            disposition: self.cancel_disposition.clone(),
        }
    }

    /// Wait for the download to complete, consuming the handle.
    ///
    /// Returns the final result or error. Drain all events from
    /// [`events()`](Self::events) before calling this to avoid missing events.
    ///
    /// # Errors
    ///
    /// Returns [`RdlpApiError`] if the download failed, was cancelled,
    /// or the background task panicked.
    pub async fn wait(self) -> Result<DownloadResult, RdlpApiError> {
        match self.join_handle.await {
            Ok(result) => result,
            Err(join_err) => Err(RdlpApiError::IoError {
                message: format!("Download task panicked: {join_err}"),
            }),
        }
    }
}

/// Detached interrupt trigger (clonable bits of a [`DownloadHandle`]), so a
/// signal-handler task can request a keep-the-partial cancel without holding
/// the whole handle. #413.
#[must_use = "InterruptHandle must be moved into the signal-handler task; dropping it makes interrupt() a no-op"]
pub struct InterruptHandle {
    token: CancellationToken,
    disposition: CancelDisposition,
}

impl InterruptHandle {
    /// Interrupt: KEEP the resumable partial, then cancel. Idempotent.
    pub fn interrupt(&self) {
        self.disposition.set_keep();
        self.token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_id_unique() {
        let id1 = DownloadId::next();
        let id2 = DownloadId::next();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_download_id_display() {
        let id = DownloadId(42);
        assert_eq!(format!("{id}"), "42");
    }

    #[test]
    fn test_download_id_as_u64() {
        let id = DownloadId(99);
        assert_eq!(id.as_u64(), 99);
    }

    #[test]
    fn test_download_id_copy_clone() {
        let id = DownloadId::next();
        let id2 = id;
        assert_eq!(id, id2);
    }

    #[tokio::test]
    async fn test_interrupt_sets_keep_and_cancels() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let token = CancellationToken::new();
        let disposition = crate::cancel::CancelDisposition::new();
        let jh = tokio::spawn(async {
            Ok(crate::result::DownloadResult {
                id: DownloadId::next(),
                output_files: vec![],
                info: rdlp_types::InfoDict::new("id", "title", "ext", "url"),
                stats: rdlp_core::DownloadStats::new(0, std::time::Duration::ZERO, 0),
            })
        });
        let handle = DownloadHandle::new(
            DownloadId::next(),
            rx,
            token.clone(),
            disposition.clone(),
            jh,
        );
        let ih = handle.interrupt_handle();
        ih.interrupt();
        assert!(token.is_cancelled(), "interrupt cancels the token");
        assert!(disposition.should_keep(), "interrupt sets Keep");
    }

    #[tokio::test]
    async fn test_cancel_leaves_discard_default() {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let token = CancellationToken::new();
        let disposition = crate::cancel::CancelDisposition::new();
        let jh = tokio::spawn(async {
            Ok(crate::result::DownloadResult {
                id: DownloadId::next(),
                output_files: vec![],
                info: rdlp_types::InfoDict::new("id", "title", "ext", "url"),
                stats: rdlp_core::DownloadStats::new(0, std::time::Duration::ZERO, 0),
            })
        });
        let handle = DownloadHandle::new(
            DownloadId::next(),
            rx,
            token.clone(),
            disposition.clone(),
            jh,
        );
        handle.cancel();
        assert!(token.is_cancelled());
        assert!(
            !disposition.should_keep(),
            "cancel() keeps the Discard default"
        );
    }
}
