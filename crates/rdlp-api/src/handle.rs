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

/// The shared keep-then-cancel interrupt sequence (#413), used by both
/// [`DownloadHandle::interrupt`] and [`InterruptHandle::interrupt`].
///
/// Ordering is load-bearing: the "keep the resumable partial" disposition MUST
/// be set *before* the token is cancelled, so the download task cannot observe
/// cancellation (and discard the partial) before the keep intent is visible.
fn request_interrupt(disposition: &CancelDisposition, token: &CancellationToken) {
    disposition.set_keep();
    token.cancel();
}

/// Record why a download task failed to join, then map it for the caller.
///
/// A `JoinError` is NOT always a panic. The other shape — cancellation —
/// arises when the task is aborted OR when the runtime shuts down with a
/// download still in flight, which is what a normal app quit looks like.
/// Reporting that as `outcome=panicked` at ERROR would make a clean shutdown
/// read as an internal bug, so the two are told apart here.
///
/// The panic payload is redacted before it is logged OR returned. `JoinError`'s `Display`
/// embeds the raw payload string when it downcasts to `String`/`&str`
/// (tokio's `panic_payload_as_str`), and these records now persist to disk, so
/// a future `panic!` written with a URL in its message would put that URL in
/// the log file. `scripts/check-url-redaction.sh` cannot see this — its
/// patterns match url-NAMED values, not a value embedded in a foreign type's
/// `Display` (the documented non-goal in that gate's header; #684). The
/// defence has to be the call, not the grep.
fn task_join_error(join_err: &tokio::task::JoinError) -> RdlpApiError {
    if join_err.is_panic() {
        // ERROR: a panicking task is an internal bug.
        let reason = rdlp_redact::redact_str(&join_err.to_string());
        log::error!("download: action=download outcome=panicked reason={reason}");
        RdlpApiError::IoError {
            message: format!("Download task panicked: {reason}"),
        }
    } else {
        // DEBUG: an aborted task or a runtime shutting down mid-download is
        // ordinary, and was previously reported to the caller as a panic.
        log::debug!("download: action=download outcome=cancelled reason={join_err}");
        RdlpApiError::IoError {
            message: format!("Download task cancelled: {join_err}"),
        }
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
        request_interrupt(&self.cancel_disposition, &self.cancel_token);
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
            Err(join_err) => Err(task_join_error(&join_err)),
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
        request_interrupt(&self.disposition, &self.token);
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
    async fn test_download_handle_interrupt_sets_keep_and_cancels() {
        // Directly exercises DownloadHandle::interrupt (the InterruptHandle path
        // is covered above); both route through the shared request_interrupt.
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
        handle.interrupt();
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod join_error_tests {
    use super::task_join_error;
    use crate::errors::RdlpApiError;

    /// A panic is reported as a panic — and its payload is redacted.
    ///
    /// `JoinError`'s Display embeds the payload string, so a `panic!` written
    /// with a credential-bearing URL would otherwise put it in a log file that
    /// now persists. The probe panics with exactly that shape.
    #[tokio::test]
    async fn a_panicking_task_is_reported_as_a_panic() {
        let join_err = tokio::spawn(async {
            panic!("boom https://user:pw@example.com/v?token=secret");
        })
        .await
        .expect_err("the task panicked");
        assert!(join_err.is_panic());

        let mapped = task_join_error(&join_err);
        let msg = format!("{mapped}");
        assert!(msg.contains("panicked"), "got: {msg}");

        // Assert on the STORED FIELD, not on `Display`. `RdlpApiError`'s
        // Display is `#[error("I/O error: {}", redact(message))]`, so the
        // rendered form is clean whether or not the field is — asserting
        // against `format!("{mapped}")` passes even when the raw payload is
        // stored, i.e. it is a tautology. Verified by mutation: putting
        // `{join_err}` back in the constructor leaves a Display assertion
        // green and turns this one red. The field matters because a future
        // path could read it without going through Display.
        let RdlpApiError::IoError { message: stored } = &mapped else {
            panic!("expected IoError, got: {mapped:?}");
        };
        assert!(
            !stored.contains("secret"),
            "stored token must not survive: {stored}"
        );
        assert!(
            !stored.contains("user:pw"),
            "stored userinfo must not survive: {stored}"
        );

        // And the same value reaches the log.
        let logged = rdlp_redact::redact_str(&join_err.to_string());
        assert!(
            !logged.contains("secret"),
            "payload token must not survive: {logged}"
        );
        assert!(
            !logged.contains("user:pw"),
            "payload userinfo must not survive: {logged}"
        );
    }

    /// An aborted task is NOT a panic, and must not be reported as one.
    ///
    /// The same shape occurs when the runtime shuts down with a download in
    /// flight — an ordinary app quit. Before this split it produced
    /// "Download task panicked" and, once logged, an ERROR claiming an
    /// internal bug.
    #[tokio::test]
    async fn a_cancelled_task_is_not_reported_as_a_panic() {
        let handle = tokio::spawn(async {
            // Never completes, so the abort below always wins. A timer would
            // work too, but every round duration trips
            // `clippy::duration_suboptimal_units` toward a constructor newer
            // than our MSRV (`from_hours`/`from_mins` are 1.91; MSRV is 1.88).
            std::future::pending::<()>().await;
        });
        handle.abort();
        let join_err = handle.await.expect_err("the task was aborted");
        assert!(
            join_err.is_cancelled(),
            "precondition: this is the cancel shape"
        );

        let msg = format!("{}", task_join_error(&join_err));
        assert!(msg.contains("cancelled"), "got: {msg}");
        assert!(!msg.contains("panicked"), "must not claim a panic: {msg}");
    }
}
