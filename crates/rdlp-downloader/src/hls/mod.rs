//! HLS (HTTP Live Streaming) downloader module.
//!
//! Downloads HLS streams by fetching pre-resolved fragments produced by
//! `expand_hls_in_place` in the extractor layer. Every `Format` with
//! `protocol: M3u8 | M3u8Native` that reaches this downloader MUST have
//! `Format.fragments` populated; absent fragments indicate a programmer error
//! (the extractor did not call the expander) and are surfaced as a typed
//! `RdlpError::Download`.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use rdlp_core::{DownloadStats, Downloader, ProgressCallback, RdlpError, Result};

use crate::http::HttpDownloader;

/// HLS (HTTP Live Streaming) downloader
///
/// Downloads HLS streams from pre-resolved fragment lists produced by
/// `expand_hls_in_place`. Direct playlist parsing (the legacy path) has
/// been removed; all HLS formats MUST carry `Format.fragments` before
/// reaching this downloader.
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_downloader::HlsDownloader;
/// use rdlp_core::Downloader;
/// use std::path::Path;
///
/// # async fn example() -> rdlp_core::Result<()> {
/// let downloader = HlsDownloader::new();
/// downloader.download_to_file(
///     "https://example.com/playlist.m3u8",
///     Path::new("video.mp4"),
///     None
/// ).await?;
/// # Ok(())
/// # }
/// ```
///
/// Every knob — concurrency, buffer size, timeouts, retries — lives on the
/// inner [`HttpDownloader`], which the pre-resolved fragments path reads
/// directly; this type carries no settings of its own.
#[derive(Clone)]
pub struct HlsDownloader {
    http_downloader: HttpDownloader,
}

impl HlsDownloader {
    /// Create a new HLS downloader over a default [`HttpDownloader`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            http_downloader: HttpDownloader::new(),
        }
    }

    /// Set the HTTP downloader to use
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_http_downloader(mut self, http: HttpDownloader) -> Self {
        self.http_downloader = http;
        self
    }

    /// Set extra HTTP headers sent with every request (delegates to inner `HttpDownloader`)
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_extra_headers(
        mut self,
        headers: Option<&std::collections::HashMap<String, String>>,
    ) -> Self {
        self.http_downloader = self.http_downloader.with_extra_headers(headers);
        self
    }
}

impl Default for HlsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for HlsDownloader {
    fn protocol(&self) -> &'static str {
        "hls"
    }

    fn supports(&self, url: &str) -> bool {
        std::path::Path::new(url)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("m3u8"))
            || url.contains("/playlist.m3u8")
            || url.contains(".m3u8?")
    }

    async fn download_format(
        &self,
        format: &rdlp_types::Format,
        output: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<DownloadStats> {
        // After #267, every M3u8 / M3u8Native row reaching the downloader has
        // Format.fragments populated by expand_hls_in_place. A row without
        // fragments indicates an extractor that did NOT call the expander —
        // programmer error, not a runtime case to handle gracefully.
        let Some(fragments) = format.fragments.as_deref() else {
            return Err(RdlpError::Download {
                message: format!(
                    "internal error: HLS Format reached HlsDownloader without \
                     pre-resolved fragments — extractor must call \
                     expand_hls_in_place. Format: {}",
                    format.format_id
                ),
                url: Some(rdlp_redact::RedactedUrlBuf::from(format.url.as_str())),
            });
        };

        crate::fragments::download_pre_resolved_fragments(
            &self.http_downloader,
            fragments,
            format.fragment_base_url.as_deref(),
            format.filesize,
            progress.map(Arc::from),
            output,
            Some(&format.url),
            cancel,
        )
        .await
    }

    async fn download_to_file(
        &self,
        url: &str,
        _path: &Path,
        _progress: Option<Box<dyn ProgressCallback>>,
    ) -> Result<DownloadStats> {
        // The legacy playlist-parsing path has been removed in #267. All HLS
        // downloads must go through download_format with pre-resolved fragments.
        // This entry point is retained only to satisfy the Downloader trait; it
        // should never be reached in production.
        Err(RdlpError::Download {
            message: "internal error: HlsDownloader::download_to_file called directly — \
                      use download_format with pre-resolved fragments (expand_hls_in_place)"
                .to_string(),
            url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hls_downloader_creation() {
        let downloader = HlsDownloader::new();
        assert_eq!(downloader.protocol(), "hls");
    }

    #[test]
    fn test_supports_m3u8_urls() {
        let downloader = HlsDownloader::new();

        assert!(downloader.supports("https://example.com/video.m3u8"));
        assert!(downloader.supports("https://example.com/playlist.m3u8"));
        assert!(downloader.supports("https://example.com/index.m3u8?token=abc"));
        assert!(!downloader.supports("https://example.com/video.mp4"));
    }
}
