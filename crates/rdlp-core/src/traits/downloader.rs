use async_trait::async_trait;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use crate::Result;

/// Protocol-specific downloader trait
///
/// Downloaders handle transferring video/audio content from various streaming protocols
/// (HTTP, HLS, DASH, RTMP, etc.) to local files.
///
/// # Lifetime Semantics
///
/// - `fn protocol(&self) -> &str` - Borrows protocol name from `&self` (lifetime elided)
/// - `fn supports(&self, url: &str)` - Input `&str` borrowed only during function execution
/// - Async methods return owned `DownloadStats` to enable Send across async boundaries
#[async_trait]
pub trait Downloader: Send + Sync {
    /// Protocol name (e.g., "http", "https", "hls", "dash", "rtmp")
    ///
    /// **Lifetime:** Returns `&str` with lifetime tied to `&self` (elided: `<'a>`).
    fn protocol(&self) -> &str;

    /// Download content to a file path
    ///
    /// # Arguments
    /// * `url` - The URL to download from
    /// * `path` - Local file path to save to
    /// * `progress` - Optional progress callback for tracking download progress
    ///
    /// # Returns
    /// Download statistics (bytes downloaded, duration, average speed)
    ///
    /// # Box<dyn Trait> Pattern
    ///
    /// The progress callback uses `Box<dyn ProgressCallback>` for:
    /// - **Runtime polymorphism:** Different callback implementations (CLI progress bar, GUI, silent)
    /// - **Ownership transfer:** Box moves ownership into the async function
    /// - **Sized requirement:** Trait objects must be behind a pointer (Box, Arc, &)
    ///
    /// **Why Box instead of Arc?**
    /// - Progress callbacks are typically used by a single download task
    /// - No need for shared ownership (Arc would add unnecessary atomic overhead)
    /// - Box is simpler and slightly cheaper than Arc for single-owner scenarios
    ///
    /// **Why Option<Box<...>>?**
    /// - Allows downloads without progress tracking (headless/batch mode)
    /// - Caller can pass `None` to skip callback overhead
    async fn download_to_file(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats>;

    /// Check if this downloader supports the given URL
    ///
    /// # Arguments
    /// * `url` - URL to check
    ///
    /// # Returns
    /// `true` if this downloader can handle the URL
    fn supports(&self, url: &str) -> bool;

    /// Get estimated download size (if available)
    ///
    /// # Arguments
    /// * `url` - URL to check size for
    ///
    /// # Returns
    /// Size in bytes, or `None` if size cannot be determined
    async fn get_size(&self, url: &str) -> Result<Option<u64>>;

    /// Download with resume support
    ///
    /// # Arguments
    /// * `url` - The URL to download from
    /// * `path` - Local file path to save to
    /// * `resume_from` - Byte offset to resume from
    /// * `progress` - Optional progress callback
    ///
    /// # Returns
    /// Download statistics
    async fn download_with_resume(
        &self,
        url: &str,
        path: &Path,
        _resume_from: u64,
        progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // Default implementation doesn't support resume, just downloads normally
        self.download_to_file(url, path, progress).await
    }
}

/// Progress tracking callback trait
///
/// Implement this trait to receive progress updates during downloads.
pub trait ProgressCallback: Send + Sync {
    /// Called when download progress updates
    ///
    /// # Arguments
    /// * `progress` - Current download progress information
    fn on_progress(&self, progress: &DownloadProgress);

    /// Called when download completes
    fn on_complete(&self, stats: &DownloadStats);

    /// Called when download fails
    fn on_error(&self, error: &str);
}

/// Current download progress information
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadProgress {
    /// Bytes downloaded so far
    pub bytes_downloaded: u64,

    /// Total bytes to download (if known)
    pub total_bytes: Option<u64>,

    /// Current download speed in bytes per second
    pub speed: f64,

    /// Estimated time remaining (if total size is known)
    pub eta: Option<Duration>,

    /// Progress percentage (0.0 to 100.0) if total size is known
    pub percentage: Option<f64>,

    /// Segments completed (for HLS/segmented downloads)
    pub segments_downloaded: Option<u64>,

    /// Total segments (for HLS/segmented downloads)
    pub total_segments: Option<u64>,

    /// Duration downloaded in seconds (for HLS with duration tracking)
    pub duration_downloaded: Option<f64>,

    /// Total stream duration in seconds (for HLS with duration tracking)
    pub total_duration: Option<f64>,
}

impl DownloadProgress {
    /// Create a new progress update
    #[must_use]
    pub fn new(bytes_downloaded: u64, total_bytes: Option<u64>, speed: f64) -> Self {
        let percentage = total_bytes.map(|total| {
            if total > 0 {
                (bytes_downloaded as f64 / total as f64) * 100.0
            } else {
                0.0
            }
        });

        let eta = if speed > 0.0 {
            total_bytes.map(|total| {
                let remaining = total.saturating_sub(bytes_downloaded);
                Duration::from_secs_f64(remaining as f64 / speed)
            })
        } else {
            None
        };

        Self {
            bytes_downloaded,
            total_bytes,
            speed,
            eta,
            percentage,
            segments_downloaded: None,
            total_segments: None,
            duration_downloaded: None,
            total_duration: None,
        }
    }

    /// Create a new segment-based progress update (for HLS downloads)
    #[must_use]
    pub fn new_with_segments(
        bytes_downloaded: u64,
        speed: f64,
        segments_downloaded: u64,
        total_segments: u64,
    ) -> Self {
        let percentage = if total_segments > 0 {
            Some((segments_downloaded as f64 / total_segments as f64) * 100.0)
        } else {
            None
        };

        let eta = if speed > 0.0 && total_segments > 0 {
            // Estimate based on segments remaining and current speed
            let remaining_segments = total_segments.saturating_sub(segments_downloaded);
            let avg_segment_size = if segments_downloaded > 0 {
                bytes_downloaded / segments_downloaded
            } else {
                2 * 1024 * 1024 // Default 2MB estimate
            };
            let remaining_bytes = remaining_segments * avg_segment_size;
            Some(Duration::from_secs_f64(remaining_bytes as f64 / speed))
        } else {
            None
        };

        Self {
            bytes_downloaded,
            total_bytes: None, // Unknown for HLS
            speed,
            eta,
            percentage,
            segments_downloaded: Some(segments_downloaded),
            total_segments: Some(total_segments),
            duration_downloaded: None,
            total_duration: None,
        }
    }

    /// Create a new duration-based progress update (for HLS with duration tracking)
    ///
    /// Uses stream duration for more accurate progress/ETA than segment count,
    /// since segments can have variable lengths.
    #[must_use]
    pub fn new_with_duration(
        bytes_downloaded: u64,
        speed: f64,
        segments_downloaded: u64,
        total_segments: u64,
        duration_downloaded: f64,
        total_duration: f64,
    ) -> Self {
        let percentage = if total_duration > 0.0 {
            Some((duration_downloaded / total_duration) * 100.0)
        } else {
            None
        };

        let eta = if speed > 0.0 && total_duration > 0.0 && duration_downloaded < total_duration {
            // Estimate remaining bytes based on duration ratio and current progress
            let remaining_duration = total_duration - duration_downloaded;
            let duration_ratio = if duration_downloaded > 0.0 {
                remaining_duration / duration_downloaded
            } else {
                1.0
            };
            let estimated_remaining_bytes = (bytes_downloaded as f64 * duration_ratio) as u64;
            Some(Duration::from_secs_f64(
                estimated_remaining_bytes as f64 / speed,
            ))
        } else {
            None
        };

        Self {
            bytes_downloaded,
            total_bytes: None,
            speed,
            eta,
            percentage,
            segments_downloaded: Some(segments_downloaded),
            total_segments: Some(total_segments),
            duration_downloaded: Some(duration_downloaded),
            total_duration: Some(total_duration),
        }
    }

    /// Check if this is a segment-based progress (HLS)
    #[must_use]
    pub fn is_segmented(&self) -> bool {
        self.total_segments.is_some()
    }

    /// Format speed as human-readable string (e.g., "1.5 MB/s")
    #[must_use]
    pub fn speed_string(&self) -> String {
        format_bytes_per_second(self.speed)
    }

    /// Format bytes downloaded as human-readable string (e.g., "15.3 MB")
    #[must_use]
    pub fn bytes_string(&self) -> String {
        format_bytes(self.bytes_downloaded)
    }

    /// Format total size as human-readable string (e.g., "100.0 MB")
    #[must_use]
    pub fn total_string(&self) -> String {
        self.total_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "Unknown".to_string())
    }
}

impl fmt::Display for DownloadProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(pct) = self.percentage {
            write!(
                f,
                "{pct:.1}% ({} / {})",
                self.bytes_string(),
                self.total_string()
            )
        } else {
            write!(f, "{} downloaded", self.bytes_string())
        }
    }
}

/// Download statistics after completion
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadStats {
    /// Total bytes downloaded
    pub bytes_downloaded: u64,

    /// Time taken for download
    pub duration: Duration,

    /// Average download speed in bytes per second
    pub average_speed: f64,

    /// Number of retries (if any)
    pub retries: usize,

    /// Number of fragments downloaded (for segmented downloads)
    pub fragments: Option<usize>,
}

impl DownloadStats {
    /// Create new download statistics
    #[must_use]
    pub fn new(bytes_downloaded: u64, duration: Duration, retries: usize) -> Self {
        let average_speed = if duration.as_secs_f64() > 0.0 {
            bytes_downloaded as f64 / duration.as_secs_f64()
        } else {
            0.0
        };

        Self {
            bytes_downloaded,
            duration,
            average_speed,
            retries,
            fragments: None,
        }
    }

    /// Create new stats with fragment information
    #[must_use]
    pub fn with_fragments(mut self, fragments: usize) -> Self {
        self.fragments = Some(fragments);
        self
    }

    /// Format average speed as human-readable string
    #[must_use]
    pub fn speed_string(&self) -> String {
        format_bytes_per_second(self.average_speed)
    }

    /// Format bytes downloaded as human-readable string
    #[must_use]
    pub fn bytes_string(&self) -> String {
        format_bytes(self.bytes_downloaded)
    }
}

impl fmt::Display for DownloadStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} in {:.1}s ({})",
            self.bytes_string(),
            self.duration.as_secs_f64(),
            self.speed_string()
        )
    }
}

/// Format bytes as human-readable string using binary units (1024-based)
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let bytes_f = bytes as f64;
    let exponent = (bytes_f.log2() / 10.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);

    let value = bytes_f / 1024_f64.powi(exponent as i32);
    let unit = UNITS[exponent];

    format!("{value:.1} {unit}")
}

/// Format bytes per second as human-readable string
fn format_bytes_per_second(bytes_per_sec: f64) -> String {
    format!("{}/s", format_bytes(bytes_per_sec as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500.0 B");
        assert_eq!(format_bytes(1536), "1.5 KiB"); // 1.5 * 1024
        assert_eq!(format_bytes(1572864), "1.5 MiB"); // 1.5 * 1024^2
        assert_eq!(format_bytes(1610612736), "1.5 GiB"); // 1.5 * 1024^3
    }

    #[test]
    fn test_download_progress() {
        let progress = DownloadProgress::new(50, Some(100), 10.0);
        assert_eq!(progress.percentage, Some(50.0));
        assert!(progress.eta.is_some());
        assert_eq!(progress.eta.unwrap().as_secs(), 5);
    }
}
