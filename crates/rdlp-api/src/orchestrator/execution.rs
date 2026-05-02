//! Download execution and progress tracking via events

use super::{
    Orchestrator,
    errors::{OrchestratorError, Result},
};
use crate::events::Event;
use crate::handle::DownloadId;
use log::{debug, info};
use rdlp_core::{DownloadProgress, DownloadStats, Downloader, ProgressCallback};
use rdlp_types::Format;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;
use tracing::instrument;

/// Progress callback that emits [`Event::Progress`] and [`Event::Warning`]
/// over an `mpsc` channel instead of updating a terminal progress bar.
struct EventProgressCallback {
    event_tx: mpsc::Sender<Event>,
    download_id: DownloadId,
}

impl EventProgressCallback {
    const fn new(event_tx: mpsc::Sender<Event>, download_id: DownloadId) -> Self {
        Self {
            event_tx,
            download_id,
        }
    }
}

impl ProgressCallback for EventProgressCallback {
    fn on_progress(&self, progress: &DownloadProgress) {
        let _ = self.event_tx.try_send(Event::Progress {
            id: self.download_id,
            progress: progress.clone(),
        });
    }

    fn on_complete(&self, _stats: &DownloadStats) {
        // Completion is reported by the caller via Event::Completed
    }

    fn on_error(&self, error: &str) {
        let _ = self.event_tx.try_send(Event::Warning {
            id: self.download_id,
            message: format!("Download error: {error}"),
        });
    }

    fn on_log(&self, message: &str) {
        let _ = self.event_tx.try_send(Event::Debug {
            id: self.download_id,
            message: message.to_owned(),
        });
    }
}

impl Orchestrator {
    /// Execute download with cancellation token support
    ///
    /// Returns Ok(Some(stats)) on success, Ok(None) if user cancelled
    ///
    /// # Arguments
    /// * `downloader` - The downloader to use
    /// * `format` - Format descriptor (provides URL and optional pre-resolved fragments)
    /// * `output_path` - Path to save the file
    /// * `resume_from` - Byte offset to resume from (0 for fresh download)
    /// * `expected_size` - Expected file size (for progress percentage)
    ///
    /// # Errors
    /// Returns an error if download fails
    #[instrument(skip(self, downloader, format), fields(url = %format.url, output = %output_path.display()))]
    #[allow(clippy::used_underscore_binding)] // _expected_size is a reserved parameter slot
    pub(super) async fn execute_download(
        &self,
        downloader: &Arc<dyn Downloader>,
        format: &Format,
        output_path: &Path,
        resume_from: u64,
        _expected_size: Option<u64>,
    ) -> Result<Option<DownloadStats>> {
        let progress_callback: Option<Box<dyn ProgressCallback>> = Some(Box::new(
            EventProgressCallback::new(self.event_tx.clone(), self.download_id),
        ));

        debug!("Starting download (cancel via CancellationToken)");

        let download_future = if resume_from > 0 {
            // Resume path uses the raw URL + byte offset; Format fields beyond
            // `url` (e.g. pre-resolved fragments) are not meaningful for resume.
            downloader.download_with_resume(
                &format.url,
                output_path,
                resume_from,
                progress_callback,
            )
        } else {
            downloader.download_format(format, output_path, progress_callback)
        };

        // Race between download and cancellation token
        let stats = tokio::select! {
            result = download_future => {
                result.map_err(OrchestratorError::DownloadFailed)?
            }
            () = self.cancel_token.cancelled() => {
                debug!("Download interrupted by cancellation");
                info!("Progress saved. Run the same command again to resume.");
                return Ok(None);
            }
        };

        Ok(Some(stats))
    }

    /// Execute download to an async writer (stdout) with cancellation support.
    ///
    /// Streams bytes directly to `writer` instead of writing to disk.
    /// Used for `-o -` (stdout) mode.
    ///
    /// # Arguments
    /// * `downloader` - The downloader to use
    /// * `url` - URL to download
    /// * `writer` - Async writer to stream bytes to (e.g. `tokio::io::stdout()`)
    ///
    /// # Returns
    /// - `Ok(Some(stats))` on success
    /// - `Ok(None)` if user cancelled via `CancellationToken`
    ///
    /// # Errors
    /// Returns an error if the download fails or the downloader does not
    /// support writer-based output.
    #[instrument(skip(self, downloader, writer), fields(url = %url))]
    pub(super) async fn execute_download_to_writer(
        &self,
        downloader: &Arc<dyn Downloader>,
        url: &str,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<Option<DownloadStats>> {
        let progress_callback: Option<Box<dyn ProgressCallback>> = Some(Box::new(
            EventProgressCallback::new(self.event_tx.clone(), self.download_id),
        ));

        debug!("Starting stdout download (cancel via CancellationToken)");

        let download_future = downloader.download_to_writer(url, writer, progress_callback);

        let stats = tokio::select! {
            result = download_future => {
                result.map_err(OrchestratorError::DownloadFailed)?
            }
            () = self.cancel_token.cancelled() => {
                debug!("Download to stdout interrupted by cancellation");
                return Ok(None);
            }
        };

        Ok(Some(stats))
    }
}
