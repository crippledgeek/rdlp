//! Download handle and ID types.
//!
//! [`DownloadId`] is an opaque identifier for correlating events with
//! downloads. [`DownloadHandle`] is the primary interface for interacting
//! with a running download — receiving events, cancelling, and waiting
//! for the final result.

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
    pub fn as_u64(&self) -> u64 {
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
/// - [`wait()`](Self::wait) consumes the handle and returns the final result.
pub struct DownloadHandle {
    id: DownloadId,
    events_rx: mpsc::Receiver<Event>,
    cancel_token: CancellationToken,
    join_handle: JoinHandle<Result<DownloadResult, RdlpApiError>>,
}

impl DownloadHandle {
    /// Create a new handle (crate-internal).
    pub(crate) fn new(
        id: DownloadId,
        events_rx: mpsc::Receiver<Event>,
        cancel_token: CancellationToken,
        join_handle: JoinHandle<Result<DownloadResult, RdlpApiError>>,
    ) -> Self {
        Self {
            id,
            events_rx,
            cancel_token,
            join_handle,
        }
    }

    /// The download ID for this handle.
    #[must_use]
    pub fn id(&self) -> DownloadId {
        self.id
    }

    /// Mutable reference to the event receiver for polling.
    ///
    /// Use `recv().await` to get the next event, or use in a `tokio::select!`.
    pub fn events(&mut self) -> &mut mpsc::Receiver<Event> {
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
}
