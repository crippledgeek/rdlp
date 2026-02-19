//! Download queue state.
//!
//! Tracks all queued, active, and completed downloads for the
//! desktop application.

/// In-memory download queue holding all download entries.
pub struct DownloadQueue {
    // Fields will be added during implementation.
    _placeholder: (),
}

impl DownloadQueue {
    /// Create a new, empty download queue.
    #[must_use]
    pub fn new() -> Self {
        Self { _placeholder: () }
    }
}

impl Default for DownloadQueue {
    fn default() -> Self {
        Self::new()
    }
}
