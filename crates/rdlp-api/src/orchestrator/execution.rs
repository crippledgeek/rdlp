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
                Some(&self.cancel_token),
            )
        } else {
            downloader.download_format(
                format,
                output_path,
                progress_callback,
                Some(&self.cancel_token),
            )
        };

        // Race between download and cancellation token. Belt-and-braces:
        // - Fragment downloaders (HLS, DASH per-Repr) check the token
        //   cooperatively between fragments and return RdlpError::Cancelled.
        // - Streaming-HTTP (HttpDownloader::download_format → download_sequential)
        //   honours the token per `bytes_stream().next()` via
        //   next_with_cancel_and_timeout, flushing BufWriter before returning
        //   RdlpError::Cancelled. (F6, PR #287 follow-up.)
        // - DashDownloader::download_format non-fragment branch delegates to
        //   HttpDownloader::download_format and inherits the same.
        // The outer `cancelled()` arm remains as a fallback for pre-start
        // cancellation, the cancel-less trait methods (download_to_writer,
        // download_with_resume), and third-party Downloader impls that
        // ignore the cancel parameter.
        //
        // Both arms produce Ok(None) on user-cancellation: the `cancelled()`
        // arm directly, and the `download_future` arm by mapping a cooperative
        // `RdlpError::Cancelled` (which the downloader returns when its own
        // cancel gate wins the race) to Ok(None). Any OTHER error from the
        // download future is mapped to `OrchestratorError::DownloadFailed`.
        let stats = tokio::select! {
            result = download_future => {
                match result {
                    Ok(stats) => stats,
                    Err(rdlp_core::RdlpError::Cancelled) => {
                        debug!("Download cancelled cooperatively");
                        info!("Progress saved. Run the same command again to resume.");
                        return Ok(None);
                    }
                    Err(e) => return Err(OrchestratorError::DownloadFailed(e)),
                }
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

        // As in `execute_download`: a cooperative `RdlpError::Cancelled` from
        // the download future is user-cancellation (Ok(None)); other errors map
        // to `OrchestratorError::DownloadFailed`.
        let stats = tokio::select! {
            result = download_future => {
                match result {
                    Ok(stats) => stats,
                    Err(rdlp_core::RdlpError::Cancelled) => {
                        debug!("Download to stdout cancelled cooperatively");
                        return Ok(None);
                    }
                    Err(e) => return Err(OrchestratorError::DownloadFailed(e)),
                }
            }
            () = self.cancel_token.cancelled() => {
                debug!("Download to stdout interrupted by cancellation");
                return Ok(None);
            }
        };

        Ok(Some(stats))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    clippy::unnecessary_literal_bound
)]
mod tests {
    use super::*;
    use crate::events::Event;
    use crate::handle::DownloadId;
    use async_trait::async_trait;
    use rdlp_core::{DownloadStats, RdlpError, Result as CoreResult};
    use rdlp_types::{Config, DownloadProtocol, Format};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// Downloader stub whose download methods return `RdlpError::Cancelled`
    /// cooperatively (mirrors the fragment/HTTP/DASH downloaders' behaviour
    /// when their cancel gate wins the race against the `cancelled()` arm).
    struct CancellingDownloader;

    #[async_trait]
    impl Downloader for CancellingDownloader {
        fn protocol(&self) -> &str {
            "stub-cancel"
        }

        async fn download_to_file(
            &self,
            _url: &str,
            _path: &Path,
            _progress: Option<Box<dyn ProgressCallback>>,
        ) -> CoreResult<DownloadStats> {
            Err(RdlpError::Cancelled)
        }

        fn supports(&self, _url: &str) -> bool {
            true
        }

        async fn download_with_resume(
            &self,
            _url: &str,
            _path: &Path,
            _resume_from: u64,
            _progress: Option<Box<dyn ProgressCallback>>,
            _cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> CoreResult<DownloadStats> {
            Err(RdlpError::Cancelled)
        }

        async fn download_to_writer(
            &self,
            _url: &str,
            _writer: Box<dyn AsyncWrite + Unpin + Send>,
            _progress: Option<Box<dyn ProgressCallback>>,
        ) -> CoreResult<DownloadStats> {
            Err(RdlpError::Cancelled)
        }
    }

    /// Downloader stub whose download methods return a non-cancellation error
    /// (must still be classified as `DownloadFailed`).
    struct FailingDownloader;

    #[async_trait]
    impl Downloader for FailingDownloader {
        fn protocol(&self) -> &str {
            "stub-fail"
        }

        async fn download_to_file(
            &self,
            _url: &str,
            _path: &Path,
            _progress: Option<Box<dyn ProgressCallback>>,
        ) -> CoreResult<DownloadStats> {
            Err(RdlpError::Other("boom".to_string()))
        }

        fn supports(&self, _url: &str) -> bool {
            true
        }

        async fn download_to_writer(
            &self,
            _url: &str,
            _writer: Box<dyn AsyncWrite + Unpin + Send>,
            _progress: Option<Box<dyn ProgressCallback>>,
        ) -> CoreResult<DownloadStats> {
            Err(RdlpError::Other("boom".to_string()))
        }
    }

    fn test_orchestrator() -> Orchestrator {
        let config = Arc::new(Config::default());
        let (tx, _rx) = mpsc::channel::<Event>(64);
        let id = DownloadId::next();
        // A live (uncancelled) token: the download_future arm must win the race
        // by returning RdlpError::Cancelled, not the cancelled() arm.
        let token = CancellationToken::new();
        Orchestrator::new(config, tx, id, token, None)
    }

    fn test_format() -> Format {
        Format::new(
            "f1",
            "https://example.com/video.mp4",
            "mp4",
            DownloadProtocol::Https,
        )
    }

    #[tokio::test]
    async fn execute_download_classifies_cooperative_cancelled_as_cancelled() {
        let orch = test_orchestrator();
        let downloader: Arc<dyn Downloader> = Arc::new(CancellingDownloader);
        let path = Path::new("/tmp/rdlp-test-cancel-output.mp4");

        // resume_from = 0 → download_format path.
        let result = orch
            .execute_download(&downloader, &test_format(), path, 0, None)
            .await;

        assert!(
            matches!(result, Ok(None)),
            "cooperative RdlpError::Cancelled must be classified as cancelled (Ok(None)), got: {result:?}"
        );
    }

    #[tokio::test]
    async fn execute_download_resume_classifies_cooperative_cancelled_as_cancelled() {
        let orch = test_orchestrator();
        let downloader: Arc<dyn Downloader> = Arc::new(CancellingDownloader);
        let path = Path::new("/tmp/rdlp-test-cancel-resume.mp4");

        // resume_from > 0 → download_with_resume path.
        let result = orch
            .execute_download(&downloader, &test_format(), path, 1024, None)
            .await;

        assert!(
            matches!(result, Ok(None)),
            "cooperative RdlpError::Cancelled on resume path must be Ok(None), got: {result:?}"
        );
    }

    #[tokio::test]
    async fn execute_download_other_error_still_fails() {
        let orch = test_orchestrator();
        let downloader: Arc<dyn Downloader> = Arc::new(FailingDownloader);
        let path = Path::new("/tmp/rdlp-test-fail-output.mp4");

        let result = orch
            .execute_download(&downloader, &test_format(), path, 0, None)
            .await;

        assert!(
            matches!(result, Err(OrchestratorError::DownloadFailed(_))),
            "non-cancellation errors must still be DownloadFailed, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn execute_download_to_writer_classifies_cooperative_cancelled_as_cancelled() {
        let orch = test_orchestrator();
        let downloader: Arc<dyn Downloader> = Arc::new(CancellingDownloader);
        let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(Vec::<u8>::new());

        let result = orch
            .execute_download_to_writer(&downloader, "https://example.com/video.mp4", writer)
            .await;

        assert!(
            matches!(result, Ok(None)),
            "cooperative RdlpError::Cancelled (writer path) must be Ok(None), got: {result:?}"
        );
    }

    #[tokio::test]
    async fn execute_download_to_writer_other_error_still_fails() {
        let orch = test_orchestrator();
        let downloader: Arc<dyn Downloader> = Arc::new(FailingDownloader);
        let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(Vec::<u8>::new());

        let result = orch
            .execute_download_to_writer(&downloader, "https://example.com/video.mp4", writer)
            .await;

        assert!(
            matches!(result, Err(OrchestratorError::DownloadFailed(_))),
            "non-cancellation errors (writer path) must still be DownloadFailed, got: {result:?}"
        );
    }
}
