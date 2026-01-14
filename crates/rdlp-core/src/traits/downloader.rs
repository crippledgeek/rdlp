use async_trait::async_trait;
use std::path::Path;
use std::time::Duration;

use crate::Result;

/// Protocol-specific downloader trait
///
/// Downloaders handle transferring video/audio content from various streaming protocols
/// (HTTP, HLS, DASH, RTMP, etc.) to local files.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// Protocol name (e.g., "http", "https", "hls", "dash", "rtmp")
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
#[derive(Debug, Clone)]
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
}

impl DownloadProgress {
    /// Create a new progress update
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
        }
    }

    /// Format speed as human-readable string (e.g., "1.5 MB/s")
    pub fn speed_string(&self) -> String {
        format_bytes_per_second(self.speed)
    }

    /// Format bytes downloaded as human-readable string (e.g., "15.3 MB")
    pub fn bytes_string(&self) -> String {
        format_bytes(self.bytes_downloaded)
    }

    /// Format total size as human-readable string (e.g., "100.0 MB")
    pub fn total_string(&self) -> String {
        self.total_bytes.map(format_bytes).unwrap_or_else(|| "Unknown".to_string())
    }
}

/// Download statistics after completion
#[derive(Debug, Clone)]
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
    pub fn with_fragments(mut self, fragments: usize) -> Self {
        self.fragments = Some(fragments);
        self
    }

    /// Format average speed as human-readable string
    pub fn speed_string(&self) -> String {
        format_bytes_per_second(self.average_speed)
    }

    /// Format bytes downloaded as human-readable string
    pub fn bytes_string(&self) -> String {
        format_bytes(self.bytes_downloaded)
    }
}

/// Format bytes as human-readable string
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let bytes_f = bytes as f64;
    let exponent = (bytes_f.log10() / 3.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);

    let value = bytes_f / 1000_f64.powi(exponent as i32);
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
        assert_eq!(format_bytes(1500), "1.5 KB");
        assert_eq!(format_bytes(1500000), "1.5 MB");
        assert_eq!(format_bytes(1500000000), "1.5 GB");
    }

    #[test]
    fn test_download_progress() {
        let progress = DownloadProgress::new(50, Some(100), 10.0);
        assert_eq!(progress.percentage, Some(50.0));
        assert!(progress.eta.is_some());
        assert_eq!(progress.eta.unwrap().as_secs(), 5);
    }
}
