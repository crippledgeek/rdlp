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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_result() -> DownloadResult {
        let id = DownloadId::next();
        let info = InfoDict::new(
            "vid123",
            "Test Video",
            "TestExtractor",
            "https://example.com",
        );
        let stats = DownloadStats::new(1024 * 1024, Duration::from_secs(10), 0);

        DownloadResult {
            id,
            output_files: vec![PathBuf::from("/tmp/test.mp4")],
            info,
            stats,
        }
    }

    #[test]
    fn test_result_fields_accessible() {
        let result = sample_result();
        assert!(result.id.as_u64() > 0);
        assert_eq!(result.output_files.len(), 1);
        assert_eq!(result.info.title, "Test Video");
        assert_eq!(result.stats.bytes_downloaded, 1024 * 1024);
    }

    #[test]
    fn test_result_clone() {
        let result = sample_result();
        let cloned = result.clone();
        assert_eq!(cloned.id, result.id);
        assert_eq!(cloned.output_files, result.output_files);
        assert_eq!(cloned.info.title, result.info.title);
    }

    #[test]
    fn test_result_empty_output_files() {
        let result = DownloadResult {
            id: DownloadId::next(),
            output_files: vec![],
            info: InfoDict::new("id", "title", "ext", "url"),
            stats: DownloadStats::new(0, Duration::ZERO, 0),
        };
        assert!(result.output_files.is_empty());
    }

    #[test]
    fn test_result_multiple_output_files() {
        let result = DownloadResult {
            id: DownloadId::next(),
            output_files: vec![
                PathBuf::from("/tmp/video.mp4"),
                PathBuf::from("/tmp/video.srt"),
                PathBuf::from("/tmp/thumb.jpg"),
            ],
            info: InfoDict::new("id", "title", "ext", "url"),
            stats: DownloadStats::new(0, Duration::ZERO, 0),
        };
        assert_eq!(result.output_files.len(), 3);
    }

    #[test]
    fn test_result_debug_format() {
        let result = sample_result();
        let debug = format!("{result:?}");
        assert!(debug.contains("DownloadResult"));
        assert!(debug.contains("test.mp4"));
    }
}
