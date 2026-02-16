//! Download handle and ID types.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque download identifier.
///
/// Uses an internal `u64` counter. The representation is intentionally opaque
/// to allow future changes (e.g., UUID) without breaking the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadId(u64);

/// Global counter for generating unique download IDs.
// Used by `DownloadId::next()` which is `pub(crate)` — will be called
// by `DownloadHandle` and `Engine` once those modules are added.
#[allow(dead_code)]
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl DownloadId {
    /// Create the next unique download ID.
    #[allow(dead_code)]
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
