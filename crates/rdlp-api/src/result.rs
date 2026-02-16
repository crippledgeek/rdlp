//! Download result types.

use crate::handle::DownloadId;
use rdlp_core::{DownloadStats, InfoDict};
use std::path::PathBuf;

/// Result of a completed download.
#[derive(Debug, Clone)]
pub struct DownloadResult {
    /// The download this result belongs to.
    pub id: DownloadId,
    /// Output files produced (may include remuxed/converted versions).
    pub output_files: Vec<PathBuf>,
    /// Extracted metadata for the downloaded content.
    pub info: InfoDict,
    /// Download performance statistics.
    pub stats: DownloadStats,
}
