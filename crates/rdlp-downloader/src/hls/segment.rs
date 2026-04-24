//! HLS segment download logic with retry support.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use wreq::header::HeaderMap;
use std::time::Duration;

use backon::Retryable;
use futures::StreamExt;
use log::{debug, warn};
use rdlp_core::{RdlpError, Result, is_retryable_error};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::instrument;

use super::types::InitSegmentInfo;
use crate::http::HttpDownloader;

/// Download a single HLS segment with retry logic using backon
///
/// Handles network errors, timeouts, and expired URLs by retrying with exponential backoff.
///
/// # Arguments
/// * `http_downloader` - The HTTP downloader to use
/// * `retry_config` - Retry configuration
/// * `buffer_size` - Buffer size for writing
/// * `idx` - Segment index (for logging)
/// * `url` - Segment URL
/// * `segment_path` - Path to save segment
/// * `progress` - Shared progress counter
///
/// # Returns
/// * `Ok((index, path, bytes))` - Successfully downloaded segment
/// * `Err(_)` - Failed after all retries
#[instrument(skip(http_downloader, retry_config, progress), fields(segment = idx))]
pub(crate) async fn download_segment_with_retry(
    http_downloader: &HttpDownloader,
    retry_config: &rdlp_core::RetryConfig,
    buffer_size: usize,
    idx: usize,
    url: String,
    segment_path: PathBuf,
    progress: Arc<AtomicU64>,
) -> Result<(usize, PathBuf, u64)> {
    let http_client = http_downloader.client().clone();
    let rate_limiter = http_downloader.rate_limiter.clone();
    let backoff = retry_config.to_backoff();
    let url: Arc<str> = Arc::from(url.as_str());
    let segment_path: Arc<Path> = Arc::from(segment_path.as_path());
    let hdrs: Arc<HeaderMap> = Arc::new(http_downloader.headers());

    // Use backon for retry with exponential backoff and jitter
    (|| {
        let client = http_client.clone();
        let url = Arc::clone(&url);
        let segment_path = Arc::clone(&segment_path);
        let progress = Arc::clone(&progress);
        let rate_limiter = rate_limiter.clone();
        let hdrs = Arc::clone(&hdrs);

        async move {
            // Download segment to file
            let response = client
                .get(url.as_ref())
                .headers((*hdrs).clone())
                .timeout(Duration::from_secs(30)) // 30 second timeout per segment
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        RdlpError::Network {
                            message: format!("Segment {idx} timeout"),
                            url: Some(url.to_string()),
                        }
                    } else if e.is_connect() {
                        RdlpError::Network {
                            message: format!("Segment {idx} connection failed"),
                            url: Some(url.to_string()),
                        }
                    } else {
                        RdlpError::Network {
                            message: format!("Segment {idx} request failed: {e}"),
                            url: Some(url.to_string()),
                        }
                    }
                })?;

            if !response.status().is_success() {
                return Err(RdlpError::Http {
                    status: response.status().as_u16(),
                    reason: format!("Segment {idx} returned HTTP {}", response.status()),
                });
            }

            // Stream segment to file with progress tracking
            let file = File::create(&*segment_path).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to create segment file '{}': {e}",
                        segment_path.display()
                    ),
                ))
            })?;
            let mut writer = BufWriter::with_capacity(buffer_size, file);
            let mut stream = response.bytes_stream();
            let mut downloaded = 0u64;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.map_err(|e| RdlpError::Network {
                    message: format!("Segment {idx} read error from {url}: {e}"),
                    url: Some(url.to_string()),
                })?;

                writer.write_all(&chunk).await.map_err(|e| {
                    RdlpError::Io(std::io::Error::new(
                        e.kind(),
                        format!(
                            "failed to write segment {idx} to '{}': {e}",
                            segment_path.display()
                        ),
                    ))
                })?;
                downloaded += chunk.len() as u64;

                // Update shared progress counter (lock-free atomic)
                progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);

                if let Some(ref limiter) = rate_limiter {
                    limiter.acquire(chunk.len()).await;
                }
            }

            writer.flush().await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to flush segment {idx} to '{}': {e}",
                        segment_path.display()
                    ),
                ))
            })?;

            Ok((idx, PathBuf::from(&*segment_path), downloaded))
        }
    })
    .retry(backoff)
    .when(is_retryable_error)
    .notify(|err, dur| {
        warn!(segment = idx, delay:? = dur; "Segment download failed, retrying: {err}");
    })
    .await
}

/// Download an fMP4 initialization segment, respecting optional byte range.
///
/// EXT-X-MAP may specify a `BYTERANGE` when the init data lives inside a
/// larger resource.  When present we send an HTTP `Range` header so we only
/// fetch the relevant bytes.
pub(crate) async fn download_init_segment(
    http_downloader: &HttpDownloader,
    retry_config: &rdlp_core::RetryConfig,
    init: &InitSegmentInfo,
    dest: &Path,
) -> Result<()> {
    let client = http_downloader.client().clone();
    let backoff = retry_config.to_backoff();
    let url: Arc<str> = Arc::from(init.url.as_str());
    let byte_range = init.byte_range;
    let dest: Arc<Path> = Arc::from(dest);
    let hdrs: Arc<HeaderMap> = Arc::new(http_downloader.headers());

    (|| {
        let client = client.clone();
        let url = Arc::clone(&url);
        let dest = Arc::clone(&dest);
        let hdrs = Arc::clone(&hdrs);
        async move {
            let mut req = client
                .get(url.as_ref())
                .headers((*hdrs).clone())
                .timeout(Duration::from_secs(30));

            // Apply Range header when EXT-X-MAP specifies BYTERANGE
            if let Some((length, offset)) = byte_range {
                let start = offset.unwrap_or(0);
                let end = start + length - 1;
                req = req.header("Range", format!("bytes={start}-{end}"));
                debug!(start, end; "Init segment byte-range request");
            }

            let response = req.send().await.map_err(|e| RdlpError::Network {
                message: format!("Init segment request failed: {e}"),
                url: Some(url.to_string()),
            })?;

            if !response.status().is_success() {
                return Err(RdlpError::Http {
                    status: response.status().as_u16(),
                    reason: format!("Init segment returned HTTP {}", response.status()),
                });
            }

            let bytes = response.bytes().await.map_err(|e| RdlpError::Network {
                message: format!("Init segment read error: {e}"),
                url: Some(url.to_string()),
            })?;

            tokio::fs::write(&*dest, &bytes).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!("failed to write init segment to '{}': {e}", dest.display()),
                ))
            })?;
            debug!(bytes = bytes.len(); "Init segment downloaded");
            Ok(())
        }
    })
    .retry(backoff)
    .when(is_retryable_error)
    .notify(|err, dur| {
        warn!(delay:? = dur; "Init segment download failed, retrying: {err}");
    })
    .await
}
