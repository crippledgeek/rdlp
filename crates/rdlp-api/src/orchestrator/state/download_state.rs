//! Download resume state type

use std::fmt;

/// Download state for resume logic
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DownloadState {
    /// Fresh download with no resume
    Fresh,
    /// Resume from byte offset
    Resume(u64),
}

impl DownloadState {
    /// Get the resume offset (0 for fresh downloads)
    #[must_use]
    pub fn offset(&self) -> u64 {
        match self {
            Self::Fresh => 0,
            Self::Resume(offset) => *offset,
        }
    }
}

impl fmt::Display for DownloadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fresh => write!(f, "fresh download"),
            Self::Resume(offset) => {
                let mb = *offset as f64 / (1024.0 * 1024.0);
                write!(f, "resuming from {mb:.1} MB")
            }
        }
    }
}
