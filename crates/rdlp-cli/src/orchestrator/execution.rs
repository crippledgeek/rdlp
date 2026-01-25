//! Download execution and progress tracking

use super::{Orchestrator, errors::*};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use log::info;
use rdlp_core::{DownloadProgress, DownloadStats, Downloader, ProgressCallback};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::instrument;

/// Progress callback that updates a progress bar
struct ProgressBarCallback {
    progress_bar: ProgressBar,
    /// Expected total size (used when downloader doesn't report total)
    expected_size: Option<u64>,
}

impl ProgressBarCallback {
    fn new(progress_bar: ProgressBar, expected_size: Option<u64>) -> Self {
        Self {
            progress_bar,
            expected_size,
        }
    }
}

impl ProgressCallback for ProgressBarCallback {
    fn on_progress(&self, progress: &DownloadProgress) {
        // Check if this is segment-based progress (HLS downloads)
        if progress.is_segmented() {
            // For HLS: progress bar tracks segments, message shows bytes
            if let (Some(completed), Some(total)) =
                (progress.segments_downloaded, progress.total_segments)
            {
                self.progress_bar.set_length(total);
                self.progress_bar.set_position(completed);

                // Update message with byte info
                let bytes_str = progress.bytes_string();
                let speed_str = progress.speed_string();
                let eta_str = progress
                    .eta
                    .map(|d| format!("~{}s", d.as_secs()))
                    .unwrap_or_else(|| "calculating...".to_string());

                self.progress_bar.set_message(format!(
                    "{completed}/{total} segments ({bytes_str}, {speed_str}, {eta_str})"
                ));
            }
        } else {
            // For HTTP: progress bar tracks bytes
            // Use reported total if available, otherwise fall back to expected size
            let total = progress.total_bytes.or(self.expected_size);

            // If actual bytes exceed expected, update the total to prevent >100% display
            let effective_total = match total {
                Some(t) if progress.bytes_downloaded > t => progress.bytes_downloaded,
                Some(t) => t,
                None => progress.bytes_downloaded, // Unknown total - show as indeterminate
            };

            self.progress_bar.set_length(effective_total);
            self.progress_bar.set_position(progress.bytes_downloaded);
        }
    }

    fn on_complete(&self, _stats: &DownloadStats) {
        // Progress bar will be finished by caller
    }

    fn on_error(&self, error: &str) {
        self.progress_bar
            .abandon_with_message(format!("Error: {error}"));
    }
}

impl Orchestrator {
    /// Create a progress bar for download tracking
    ///
    /// Uses steady tick for smooth animation regardless of download speed.
    ///
    /// # Errors
    /// Returns an error if progress bar template is invalid
    pub(super) fn create_progress_bar(
        &self,
        filesize: Option<u64>,
        resume_from: u64,
    ) -> Result<Option<ProgressBar>> {
        if !self.config.progress {
            return Ok(None);
        }

        // Use higher refresh rate (30 fps) for smoother animation
        let pb = ProgressBar::with_draw_target(filesize, ProgressDrawTarget::stderr_with_hz(30));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
                )
                .map_err(|e| OrchestratorError::ProgressBarFailed(e.to_string()))?
                .progress_chars("#>-"),
        );

        // Enable steady tick for smooth spinner animation (10 fps)
        pb.enable_steady_tick(Duration::from_millis(100));

        if resume_from > 0 {
            pb.set_position(resume_from);
        }

        Ok(Some(pb))
    }

    /// Create a segment-based progress bar for HLS downloads
    ///
    /// Unlike byte-based progress bars, this tracks segment completion.
    /// The message shows bytes and speed, while the bar shows segment progress.
    /// Uses steady tick for smooth animation between segment completions.
    ///
    /// # Errors
    /// Returns an error if progress bar template is invalid
    pub(super) fn create_hls_progress_bar(&self) -> Result<Option<ProgressBar>> {
        if !self.config.progress {
            return Ok(None);
        }

        // Use higher refresh rate (30 fps) for smoother animation
        let pb = ProgressBar::with_draw_target(
            Some(0), // Will be set when we know total segments
            ProgressDrawTarget::stderr_with_hz(30),
        );
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {msg}")
                .map_err(|e| OrchestratorError::ProgressBarFailed(e.to_string()))?
                .progress_chars("#>-"),
        );

        // Enable steady tick for smooth spinner animation (10 fps)
        // This keeps the spinner moving even when waiting for segments
        pb.enable_steady_tick(Duration::from_millis(100));

        pb.set_message("Downloading segments...");

        Ok(Some(pb))
    }

    /// Execute download with Ctrl+C signal handling
    ///
    /// Returns Ok(Some(stats)) on success, Ok(None) if user cancelled
    ///
    /// # Arguments
    /// * `downloader` - The downloader to use
    /// * `url` - URL to download
    /// * `output_path` - Path to save the file
    /// * `resume_from` - Byte offset to resume from (0 for fresh download)
    /// * `progress_bar` - Optional progress bar for UI
    /// * `expected_size` - Expected file size for accurate progress (used when downloader doesn't report total)
    ///
    /// # Errors
    /// Returns an error if download fails
    #[instrument(skip(self, downloader, progress_bar), fields(url = %url, output = %output_path.display()))]
    pub(super) async fn execute_download(
        &self,
        downloader: &Arc<dyn Downloader>,
        url: &str,
        output_path: &Path,
        resume_from: u64,
        progress_bar: Option<&ProgressBar>,
        expected_size: Option<u64>,
    ) -> Result<Option<DownloadStats>> {
        let progress_callback: Option<Box<dyn ProgressCallback>> = progress_bar.map(|pb| {
            Box::new(ProgressBarCallback::new(pb.clone(), expected_size))
                as Box<dyn ProgressCallback>
        });

        info!("Press Ctrl+C to pause and save progress");

        let download_future = if resume_from > 0 {
            downloader.download_with_resume(url, output_path, resume_from, progress_callback)
        } else {
            downloader.download_to_file(url, output_path, progress_callback)
        };

        // Race between download and Ctrl+C signal
        let stats = tokio::select! {
            result = download_future => {
                result.map_err(OrchestratorError::DownloadFailed)?
            }
            _ = tokio::signal::ctrl_c() => {
                if let Some(pb) = progress_bar {
                    // Clear the progress bar completely to avoid stale rendering
                    pb.finish_and_clear();
                }
                info!("Download interrupted by user");
                info!("Progress saved. Run the same command again to resume.");
                return Ok(None);
            }
        };

        if let Some(pb) = progress_bar {
            pb.finish_with_message("Download complete");
        }

        Ok(Some(stats))
    }
}
