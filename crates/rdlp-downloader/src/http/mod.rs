//! HTTP/HTTPS downloader implementation
//!
//! Provides HTTP downloading with parallel chunk support, resume capability,
//! and automatic retry logic using the backon crate.

mod config;
mod parallel;
mod trait_impl;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use parallel::{download_chunk_with_retry, verify_merged_size};

use backon::Retryable;
use log::warn;
use rdlp_core::{
    DownloadProgress, DownloadStats, ProgressCallback, RdlpError, Result, RetryConfig,
    check_http_response, is_retryable_error,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use wreq::header::{HeaderMap, HeaderName, HeaderValue};

use crate::chunking::ChunkSizeStrategy;
use crate::progress::SpeedMeter;
use config::{DownloaderConfig, PROGRESS_UPDATE_INTERVAL};
use rdlp_ratelimit::RateLimiter;

/// Convert optional `HashMap` headers to wreq `HeaderMap`
fn to_header_map(headers: Option<&HashMap<String, String>>) -> HeaderMap {
    let Some(headers) = headers else {
        return HeaderMap::new();
    };
    let mut map = HeaderMap::new();
    for (key, value) in headers {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.insert(name, val);
        }
    }
    map
}

/// Execute an async operation with retry logic
async fn with_retry<F, Fut, T>(
    retry_config: &RetryConfig,
    context: &'static str,
    operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let backoff = retry_config.to_backoff();
    operation
        .retry(backoff)
        .when(is_retryable_error)
        .notify(|err, dur| {
            warn!(delay:? = dur; "{context} failed, retrying: {err}");
        })
        .await
}

/// Re-export of the shared `ProbeResult` from `rdlp-http`. Single source of
/// truth for probe-result shape, used by both `HttpDownloader::probe` and
/// `BaseExtractor::detect_file_size`. Closes #306.
pub(crate) use rdlp_http::ProbeResult;

/// HTTP status a single-part ranged response must carry (RFC 9110 §15.3.7).
///
/// A `200` means the server ignored `Range` — permitted by §14.2 — and the
/// content is the WHOLE representation, not the requested span. Writing such a
/// body into a chunk's offset slot is the corruption in #526, so the parallel
/// chunk path accepts this status and no other.
const HTTP_PARTIAL_CONTENT: u16 = 206;

/// A parsed, validated single-part `Content-Range` response header.
///
/// Grammar (RFC 9110 §14.4):
///
/// ```text
/// Content-Range = range-unit SP ( range-resp / unsatisfied-range )
/// range-resp    = incl-range "/" ( complete-length / "*" )
/// incl-range    = first-pos "-" last-pos
/// ```
///
/// Both positions are INCLUSIVE, so the span covers
/// `last_pos - first_pos + 1` bytes.
///
/// Only the `bytes` range unit is represented: §14.4 requires that a recipient
/// which does not understand the unit "MUST NOT attempt to recombine it with a
/// stored representation", and recombining is exactly what the chunk merge
/// does. `unsatisfied-range` (`bytes */1234`, sent with 416) is likewise not
/// represented — it describes no enclosed content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentRange {
    /// First byte position of the enclosed span (inclusive).
    first_pos: u64,
    /// Last byte position of the enclosed span (inclusive).
    last_pos: u64,
    /// Total length of the selected representation; `None` when the server
    /// sent `*` to signal that the complete length was unknown (§14.4).
    complete_length: Option<u64>,
}

impl ContentRange {
    /// Parse a `Content-Range` field value, returning `None` when it is absent,
    /// malformed, carries a non-`bytes` unit, or is *invalid* per RFC 9110
    /// §14.4 — that is, `last-pos < first-pos`, or a `complete-length` less
    /// than or equal to `last-pos`. The spec's directive for an invalid value
    /// is that the recipient "MUST NOT attempt to recombine the received
    /// content with a stored representation", so an unparseable or invalid
    /// header and a missing one are treated alike: the response is not usable.
    fn parse(value: &str) -> Option<Self> {
        let (unit, rest) = value.trim().split_once(' ')?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return None;
        }

        let (incl_range, complete) = rest.trim().split_once('/')?;
        let (first, last) = incl_range.split_once('-')?;

        let first_pos: u64 = first.trim().parse().ok()?;
        let last_pos: u64 = last.trim().parse().ok()?;

        // §14.4: a last-pos below first-pos makes the field value invalid.
        if last_pos < first_pos {
            return None;
        }

        let complete_length = match complete.trim() {
            "*" => None,
            digits => {
                let total: u64 = digits.parse().ok()?;
                // §14.4: a complete-length <= last-pos makes the value invalid.
                if total <= last_pos {
                    return None;
                }
                Some(total)
            }
        };

        Some(Self {
            first_pos,
            last_pos,
            complete_length,
        })
    }

    /// Read the header map and parse the `Content-Range` field if present.
    fn from_headers(headers: &wreq::header::HeaderMap) -> Option<Self> {
        headers
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(Self::parse)
    }
}

/// Parse the `Content-Range` header's total-bytes field.
///
/// Used by `trait_impl::download_with_resume_with_cancel` to discover the
/// total resource size from a resume Range response. The probe path uses
/// `rdlp_http::probe_size` (which has its own parser); this helper is kept
/// for the resume path's standalone header inspection.
///
/// Returns the `complete-length` of a valid single-part `bytes` range, and
/// `None` for `bytes 0-N/*` (server signalled unknown total), a missing
/// header, or any value [`ContentRange::parse`] rejects.
pub(crate) fn parse_content_range_total(headers: &wreq::header::HeaderMap) -> Option<u64> {
    ContentRange::from_headers(headers).and_then(|range| range.complete_length)
}

/// The inclusive byte span a ranged chunk fetch asked the server for.
///
/// Both bounds are inclusive, matching the `Range: bytes=start-end` request
/// form and RFC 9110 §14.4's `incl-range`, so the span covers
/// `end - start + 1` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestedSpan {
    /// First byte position requested (inclusive).
    start: u64,
    /// Last byte position requested (inclusive).
    end: u64,
}

impl RequestedSpan {
    /// Number of bytes the span covers.
    const fn len(self) -> u64 {
        self.end - self.start + 1
    }
}

/// Confirm a ranged response actually carries the requested span before any of
/// its bytes are written into the chunk's offset slot in the merged output.
///
/// A parallel chunk is concatenated at a fixed position, so a response that
/// encloses a different span silently relocates every byte after it — the
/// ~517 MB interior displacement in #526. RFC 9110 §15.3.7 places this duty on
/// the client: "A client MUST inspect a 206 response's Content-Type and
/// Content-Range field(s) to determine what parts are enclosed and whether
/// additional requests are needed."
///
/// Returns `Err` (never a silent acceptance) when the status is not 206, the
/// `Content-Range` is absent/malformed/invalid, or the enclosed span is not
/// exactly the one requested.
fn validate_range_response(
    response: &wreq::Response,
    span: RequestedSpan,
    url: &str,
) -> Result<()> {
    let redacted = || Some(rdlp_redact::RedactedUrlBuf::from(url));

    // §14.2 permits a server to ignore Range; the reply is then a 200 carrying
    // the WHOLE representation. Accepting it here is what wrote whole-file
    // content into a chunk slot.
    let status = response.status().as_u16();
    if status != HTTP_PARTIAL_CONTENT {
        return Err(RdlpError::Download {
            url: redacted(),
            message: format!(
                "ranged chunk request for bytes {}-{} got HTTP {status}, expected \
                 {HTTP_PARTIAL_CONTENT} (Partial Content). The server ignored the Range \
                 header, so the body is the whole resource rather than the requested span \
                 and cannot be placed at this chunk's offset.",
                span.start, span.end
            ),
        });
    }

    // §15.3.7.1: a single-part 206 MUST carry Content-Range. Without it there
    // is no way to confirm which span arrived.
    let Some(range) = ContentRange::from_headers(response.headers()) else {
        return Err(RdlpError::Download {
            url: redacted(),
            message: format!(
                "ranged chunk request for bytes {}-{} got a {HTTP_PARTIAL_CONTENT} response \
                 with a missing, malformed, or invalid Content-Range header; the enclosed \
                 span cannot be verified.",
                span.start, span.end
            ),
        });
    };

    if range.first_pos != span.start || range.last_pos != span.end {
        return Err(RdlpError::Download {
            url: redacted(),
            message: format!(
                "ranged chunk request for bytes {}-{} got Content-Range bytes {}-{}; the \
                 response encloses a different span than requested and would corrupt the \
                 output at this chunk's offset.",
                span.start, span.end, range.first_pos, range.last_pos
            ),
        });
    }

    Ok(())
}

/// HTTP/HTTPS downloader
///
/// **Clone performance:** O(1) - both client and config use Arc internally
#[derive(Clone)]
pub struct HttpDownloader {
    client: wreq::Client,
    pub(crate) config: Arc<DownloaderConfig>,
    pub(crate) rate_limiter: Option<Arc<RateLimiter>>,
    extra_headers: HeaderMap,
}

impl HttpDownloader {
    /// Create a new HTTP downloader
    #[must_use]
    pub fn new() -> Self {
        // Route through HttpClientFactory so the default browser emulation
        // profile (ChromeLatest) is applied — otherwise this constructor
        // would hand back a wreq client with no JA4 / JA4H emulation,
        // bypassing the Phase 2 fingerprint guarantee (spec §6.8).
        let client =
            rdlp_http::HttpClientFactory::from_config(&rdlp_http::HttpClientConfig::default())
                .build();
        Self::with_client(client)
    }

    /// Create with custom client
    #[must_use]
    pub fn with_client(client: wreq::Client) -> Self {
        Self {
            client,
            config: Arc::new(DownloaderConfig::default()),
            rate_limiter: None,
            extra_headers: HeaderMap::new(),
        }
    }

    /// Get reference to the HTTP client
    #[must_use]
    pub const fn client(&self) -> &wreq::Client {
        &self.client
    }

    /// Set buffer size for downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        Arc::make_mut(&mut self.config).buffer_size = size;
        self
    }

    /// Set retry configuration
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        Arc::make_mut(&mut self.config).retry_config = config;
        self
    }

    /// Set number of concurrent fragment downloads
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_concurrent_fragments(mut self, count: usize) -> Self {
        Arc::make_mut(&mut self.config).concurrent_fragments = count.max(1);
        self
    }

    /// Return the configured concurrent-fragments limit.
    ///
    /// Used by `download_pre_resolved_fragments` to size the `buffered(N)` parallel
    /// fetch stream and to set `AdaptiveConfig::max_connections`.
    #[must_use]
    pub fn concurrent_fragments(&self) -> usize {
        self.config.concurrent_fragments
    }

    /// Set the minimum file size in bytes at which the downloader switches
    /// to parallel chunked mode. Below this, sequential I/O is used.
    /// Default: `DEFAULT_PARALLEL_THRESHOLD_BYTES` (10 MiB).
    ///
    /// `bytes` is clamped to a floor of 1 to mirror the `Config::validate()`
    /// lower bound and prevent threshold = 0 from amplifying HEAD-probe
    /// traffic on every download.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_parallel_threshold(mut self, bytes: u64) -> Self {
        Arc::make_mut(&mut self.config).parallel_threshold = bytes.max(1);
        self
    }

    /// Set chunk size strategy.
    ///
    /// When `Fixed` or `Legacy` is used, adaptive mode is forced off because
    /// the caller has explicitly chosen a predictable chunk sizing scheme.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_chunk_strategy(mut self, strategy: ChunkSizeStrategy) -> Self {
        let cfg = Arc::make_mut(&mut self.config);
        if !matches!(strategy, ChunkSizeStrategy::Auto) {
            cfg.adaptive = false;
        }
        cfg.chunk_strategy = strategy;
        self
    }

    /// Enable or disable adaptive chunk sizing and connection tuning.
    ///
    /// When `false`, the downloader uses the static `chunk_strategy` with a
    /// fixed connection count. Automatically forced to `false` when
    /// `chunk_strategy` is not `Auto`.
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_adaptive(mut self, adaptive: bool) -> Self {
        Arc::make_mut(&mut self.config).adaptive = adaptive;
        self
    }

    /// Set per-read idle timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        Arc::make_mut(&mut self.config).read_timeout = timeout;
        self
    }

    /// Set total download timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_download_timeout(mut self, timeout: Duration) -> Self {
        Arc::make_mut(&mut self.config).download_timeout = timeout;
        self
    }

    /// Set merge operation timeout
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_merge_timeout(mut self, timeout: Duration) -> Self {
        Arc::make_mut(&mut self.config).merge_timeout = timeout;
        self
    }

    /// Set the rate limiter for bandwidth throttling
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_rate_limiter(mut self, limiter: Option<Arc<RateLimiter>>) -> Self {
        self.rate_limiter = limiter;
        self
    }

    /// Set extra HTTP headers sent with every download request (e.g. Referer for CDN auth)
    #[must_use = "builder methods consume self and return a new instance"]
    pub fn with_extra_headers(mut self, headers: Option<&HashMap<String, String>>) -> Self {
        self.extra_headers = to_header_map(headers);
        self
    }

    /// Get a clone of extra headers for use in closures
    #[must_use]
    pub fn headers(&self) -> HeaderMap {
        self.extra_headers.clone()
    }

    /// F3 single-GET probe: replaces the HEAD×2 + Range:bytes=0-0 sequence.
    /// Sends `GET Range: bytes=0-{PROBE_WINDOW_BYTES-1}`, parses headers only,
    /// discards body. Returns `ProbeResult` for the downstream parallel-vs-sequential
    /// decision in `download_to_file`.
    ///
    /// Status handling:
    /// - 206 → parse total from `Content-Range`, `supports_ranges = true`.
    /// - 200 → server ignored Range; parse total from `Content-Length`,
    ///   `supports_ranges = false`. The probe body is discarded (caller
    ///   re-issues a plain GET via `download_sequential`).
    /// - other (4xx/5xx after retry) → `ProbeResult { size: None,
    ///   supports_ranges: false }`. Non-2xx is "no info", not an error;
    ///   caller falls to sequential.
    pub(crate) async fn probe(&self, url: &str) -> Result<ProbeResult> {
        use config::PROBE_WINDOW_BYTES;

        // F3 probe delegates to the shared `rdlp_http::probe_size` helper
        // (closes #306). Retry semantics preserved via the with_retry
        // wrapper; the shared helper itself is leaf-level (no retry).
        // Non-2xx and network errors after retry both produce the
        // `ProbeResult { size: None, supports_ranges: false }` form so the
        // caller falls back to sequential download.
        let client = self.client.clone();
        let url_string = url.to_string();
        let hdrs = self.headers();
        let window = PROBE_WINDOW_BYTES;
        let timeout = self.config.read_timeout;

        let probed = with_retry(&self.config.retry_config, "HTTP probe (F3)", || {
            let client = client.clone();
            let url = url_string.clone();
            let hdrs = hdrs.clone();
            async move {
                rdlp_http::probe_size(&client, &url, Some(&hdrs), window, timeout)
                    .await
                    .map_err(|e| RdlpError::Network {
                        message: format!("probe failed: {e}"),
                        url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                    })
            }
        })
        .await;

        Ok(probed.unwrap_or(ProbeResult {
            size: None,
            supports_ranges: false,
        }))
    }

    /// Download a specific byte range with shared progress tracking.
    ///
    /// `cancel` — when `Some`, each chunk poll races the token via
    /// `next_with_cancel_and_timeout`. On cancellation the `BufWriter` is
    /// flushed before returning `RdlpError::Cancelled` so partial bytes already
    /// buffered reach disk.
    pub(crate) async fn download_range_with_progress(
        &self,
        url: &str,
        start: u64,
        end: u64,
        chunk_path: &Path,
        progress_counter: Option<Arc<std::sync::atomic::AtomicU64>>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<u64> {
        // Pre-cancel guard: bail before any network I/O if already cancelled.
        if let Some(token) = cancel
            && token.is_cancelled()
        {
            return Err(RdlpError::Cancelled);
        }

        let client = self.client.clone();
        let url = url.to_string();
        let hdrs = self.headers();

        let response = with_retry(&self.config.retry_config, "HTTP GET (range)", || {
            let client = client.clone();
            let url = url.clone();
            let hdrs = hdrs.clone();
            async move {
                let response = client
                    .get(&url)
                    .headers(hdrs)
                    .header("Range", format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(|e| RdlpError::Network {
                        message: format!("Range request failed: {e}"),
                        url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                    })?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .await?;

        // Confirm the response encloses exactly the requested span BEFORE any
        // of its bytes reach the chunk file (#526).
        let span = RequestedSpan { start, end };
        validate_range_response(&response, span, &url)?;
        let expected_len = span.len();

        let file = File::create(chunk_path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to create chunk file '{}': {e}",
                    chunk_path.display()
                ),
            ))
        })?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let stream = response.bytes_stream();
        tokio::pin!(stream);
        let mut downloaded = 0u64;
        let read_timeout = self.config.read_timeout;

        loop {
            match next_with_cancel_and_timeout(stream.as_mut(), cancel, read_timeout, &url).await {
                Ok(Some(Ok(chunk))) => {
                    writer.write_all(&chunk).await.map_err(|e| {
                        RdlpError::Io(std::io::Error::new(
                            e.kind(),
                            format!(
                                "failed to write to chunk file '{}': {e}",
                                chunk_path.display()
                            ),
                        ))
                    })?;
                    downloaded += chunk.len() as u64;

                    // Stop the moment the body overruns the span the headers
                    // promised, rather than streaming an unbounded body to
                    // disk before the post-loop check notices.
                    if downloaded > expected_len {
                        return Err(RdlpError::Download {
                            url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                            message: format!(
                                "ranged chunk for bytes {start}-{end} delivered more than the \
                                 {expected_len} bytes its Content-Range promised; aborting to \
                                 avoid overrunning this chunk's offset in the merged output."
                            ),
                        });
                    }

                    if let Some(ref counter) = progress_counter {
                        counter.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    }

                    if let Some(ref limiter) = self.rate_limiter {
                        limiter.acquire(chunk.len()).await;
                    }
                }
                Ok(Some(Err(e))) => {
                    return Err(RdlpError::Network {
                        message: format!(
                            "Failed to read chunk body from {}: {e}",
                            rdlp_redact::RedactedUrl::new(url.as_str())
                        ),
                        url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                    });
                }
                Ok(None) => break,
                Err(RdlpError::Cancelled) => {
                    let _ = writer.flush().await;
                    return Err(RdlpError::Cancelled);
                }
                Err(e) => return Err(e),
            }
        }

        writer.flush().await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to flush chunk file '{}': {e}", chunk_path.display()),
            ))
        })?;

        // The stream ending is not proof the whole span arrived. hyper normally
        // surfaces an interrupted body as an error, but that is an empirical
        // property, not a guarantee — hyperium/hyper#3253 is a fixed case where
        // a truncated chunked body ended the stream with no error at all. A
        // short chunk shifts every later chunk in the merged output, so the
        // byte count is verified here rather than trusted to the transport.
        //
        // A 206 enclosing less than was requested is also legal on its own
        // terms (§14.2: a server "may only be possible (or efficient) to send a
        // portion of the requested ranges first, while expecting the client to
        // re-request the remaining portions later"). Re-requesting only the
        // remainder is the spec's answer; this returns a RETRYABLE error so
        // `download_chunk_with_retry` re-fetches the whole chunk instead, which
        // is correct but wasteful. Tracked as a follow-up.
        if downloaded != expected_len {
            return Err(RdlpError::Network {
                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                message: format!(
                    "ranged chunk for bytes {start}-{end} ended after {downloaded} of \
                     {expected_len} bytes; the chunk is incomplete and would displace every \
                     later chunk in the merged output."
                ),
            });
        }

        Ok(downloaded)
    }

    /// Sequential download with optional cooperative cancellation.
    ///
    /// `cancel` — when `Some`, each chunk poll races the token; the first arm
    /// that fires wins.  On cancellation the `BufWriter` is flushed before
    /// returning `RdlpError::Cancelled` so partial bytes already buffered reach
    /// disk.
    pub(crate) async fn download_sequential(
        &self,
        url: &str,
        path: &Path,
        progress: Option<Box<dyn ProgressCallback>>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<DownloadStats> {
        let progress: Option<Arc<dyn ProgressCallback>> = progress.map(Arc::from);
        let start_time = Instant::now();
        let client = self.client.clone();
        let url_string: Arc<str> = Arc::from(url);
        let hdrs = self.headers();

        // Check for pre-cancelled token before issuing the network request.
        if let Some(token) = cancel
            && token.is_cancelled()
        {
            return Err(RdlpError::Cancelled);
        }

        let response = with_retry(&self.config.retry_config, "HTTP GET", || {
            let client = client.clone();
            let url = url_string.clone();
            let hdrs = hdrs.clone();
            async move {
                let response = client.get(&*url).headers(hdrs).send().await.map_err(|e| {
                    RdlpError::Network {
                        message: format!("GET request failed: {e}"),
                        url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_ref())),
                    }
                })?;

                check_http_response(&response)?;
                Ok(response)
            }
        })
        .await?;

        let total_size = response.content_length();
        let file = File::create(path).await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to create output file '{}': {e}", path.display()),
            ))
        })?;
        let mut writer = BufWriter::with_capacity(self.config.buffer_size, file);

        let stream = response.bytes_stream();
        tokio::pin!(stream);
        let mut downloaded: u64 = 0;
        let mut last_update = Instant::now();
        let update_interval = PROGRESS_UPDATE_INTERVAL;
        let read_timeout = self.config.read_timeout;
        let mut speed_meter = SpeedMeter::new();
        speed_meter.update(downloaded, start_time);

        loop {
            let next = match next_with_cancel_and_timeout(
                stream.as_mut(),
                cancel,
                read_timeout,
                &url_string,
            )
            .await
            {
                Ok(item) => item,
                Err(RdlpError::Cancelled) => {
                    // Flush partial bytes already in BufWriter to disk.
                    writer.flush().await.ok();
                    return Err(RdlpError::Cancelled);
                }
                Err(e) => return Err(e),
            };

            let Some(chunk_result) = next else { break };
            let chunk = chunk_result.map_err(|e| RdlpError::Network {
                message: format!(
                    "Failed to read response body from {}: {e}",
                    rdlp_redact::RedactedUrl::new(url_string.as_ref())
                ),
                url: Some(rdlp_redact::RedactedUrlBuf::from(url_string.as_ref())),
            })?;

            writer.write_all(&chunk).await.map_err(|e| {
                RdlpError::Io(std::io::Error::new(
                    e.kind(),
                    format!("failed to write to output file '{}': {e}", path.display()),
                ))
            })?;
            downloaded += chunk.len() as u64;

            if let Some(ref callback) = progress {
                let now = Instant::now();
                if now.duration_since(last_update) >= update_interval {
                    speed_meter.update(downloaded, now);
                    let speed = speed_meter.bytes_per_sec().unwrap_or(0.0);

                    let progress_info = DownloadProgress::new(downloaded, total_size, speed);
                    callback.on_progress(&progress_info);
                    last_update = now;
                }
            }

            if let Some(ref limiter) = self.rate_limiter {
                limiter.acquire(chunk.len()).await;
            }
        }

        writer.flush().await.map_err(|e| {
            RdlpError::Io(std::io::Error::new(
                e.kind(),
                format!("failed to flush output file '{}': {e}", path.display()),
            ))
        })?;

        let duration = start_time.elapsed();
        let stats = DownloadStats::new(downloaded, duration, 0);

        if let Some(callback) = progress {
            callback.on_complete(&stats);
        }

        Ok(stats)
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

/// Race a `bytes_stream()` poll against (a) the per-read timeout and (b) an
/// optional cancellation token.
///
/// Returns:
/// - `Ok(Some(Ok(bytes)))` — chunk delivered.
/// - `Ok(Some(Err(stream_err)))` — stream-level error from the body; caller
///   decides how to surface it.
/// - `Ok(None)` — stream ended cleanly.
/// - `Err(RdlpError::Cancelled)` — cancel arm fired; caller MUST flush its
///   writer before returning.
/// - `Err(RdlpError::Network { .. })` — read timed out.
///
/// `biased;` is required: the cancel arm must take priority when both are
/// ready, otherwise tokio's PRNG branch selection can starve the cancel arm
/// under load. A static test in `tests.rs` (Task 12) will assert the
/// `biased;` keyword is present in this select.
pub(crate) async fn next_with_cancel_and_timeout<S, E>(
    mut stream: std::pin::Pin<&mut S>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    read_timeout: std::time::Duration,
    url: &str,
) -> Result<Option<std::result::Result<bytes::Bytes, E>>>
where
    S: futures::Stream<Item = std::result::Result<bytes::Bytes, E>>,
{
    use futures::StreamExt;
    let timeout_err = || RdlpError::Network {
        message: format!("Read timed out (no data for {}s)", read_timeout.as_secs()),
        url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
    };
    match cancel {
        Some(token) => {
            tokio::select! {
                biased;
                () = token.cancelled() => Err(RdlpError::Cancelled),
                r = tokio::time::timeout(read_timeout, stream.next()) => {
                    r.map_or_else(|_| Err(timeout_err()), Ok)
                },
            }
        }
        None => tokio::time::timeout(read_timeout, stream.next())
            .await
            .map_or_else(|_| Err(timeout_err()), Ok),
    }
}

#[cfg(test)]
mod content_range_tests {
    //! Tests for the local `parse_content_range_total` helper. The probe
    //! path uses `rdlp_http::probe_size`'s internal parser; this helper
    //! is the standalone version used by the resume path in
    //! `trait_impl::download_with_resume_with_cancel`.
    use super::*;
    use wreq::header::HeaderMap;

    fn make_headers(content_range: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(cr) = content_range {
            h.insert("content-range", cr.parse().unwrap());
        }
        h
    }

    #[test]
    fn parses_total_from_content_range_206() {
        let h = make_headers(Some("bytes 0-262143/1048576"));
        assert_eq!(parse_content_range_total(&h), Some(1_048_576));
    }

    #[test]
    fn returns_none_when_header_missing() {
        let h = make_headers(None);
        assert_eq!(parse_content_range_total(&h), None);
    }

    #[test]
    fn returns_none_when_header_malformed_no_slash() {
        let h = make_headers(Some("bytes 0-262143"));
        assert_eq!(parse_content_range_total(&h), None);
    }

    #[test]
    fn returns_none_when_total_is_star() {
        let h = make_headers(Some("bytes 0-262143/*"));
        assert_eq!(parse_content_range_total(&h), None);
    }

    #[test]
    fn returns_none_when_total_unparseable() {
        let h = make_headers(Some("bytes 0-262143/notanumber"));
        assert_eq!(parse_content_range_total(&h), None);
    }
}

#[cfg(test)]
mod cancel_helper_tests {
    use super::*;
    use bytes::Bytes;
    use futures::stream;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn returns_item_when_stream_ready_no_cancel() {
        let s = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from("hello"))]);
        tokio::pin!(s);
        let timeout = Duration::from_secs(1);

        let res = next_with_cancel_and_timeout(s.as_mut(), None, timeout, "test").await;

        let item = res.unwrap();
        assert!(matches!(item, Some(Ok(b)) if b == "hello"));
    }

    #[tokio::test]
    async fn returns_none_at_stream_end() {
        let s = stream::iter::<Vec<std::result::Result<Bytes, std::io::Error>>>(vec![]);
        tokio::pin!(s);
        let timeout = Duration::from_secs(1);

        let res = next_with_cancel_and_timeout(s.as_mut(), None, timeout, "test").await;

        assert!(res.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancel_fires_returns_cancelled_error() {
        let s = stream::pending::<std::result::Result<Bytes, std::io::Error>>();
        tokio::pin!(s);
        let timeout = Duration::from_secs(10);
        let token = CancellationToken::new();
        token.cancel();

        let res = next_with_cancel_and_timeout(s.as_mut(), Some(&token), timeout, "test").await;

        assert!(matches!(res, Err(RdlpError::Cancelled)));
    }

    #[tokio::test]
    async fn read_timeout_fires_returns_network_error() {
        let s = stream::pending::<std::result::Result<Bytes, std::io::Error>>();
        tokio::pin!(s);
        let timeout = Duration::from_millis(50);

        let res = next_with_cancel_and_timeout(s.as_mut(), None, timeout, "http://test").await;

        match res {
            Err(RdlpError::Network { message, url }) => {
                assert!(
                    message.to_lowercase().contains("timed out"),
                    "got: {message}"
                );
                assert_eq!(
                    url.as_ref().map(rdlp_redact::RedactedUrlBuf::expose),
                    Some("http://test")
                );
            }
            other => panic!("expected Network timeout, got {other:?}"),
        }
    }
}
