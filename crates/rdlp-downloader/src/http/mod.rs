//! HTTP/HTTPS downloader implementation
//!
//! Provides HTTP downloading with parallel chunk support, resume capability,
//! and automatic retry logic using the backon crate.

mod chunk_ledger;
pub(crate) mod chunk_name;
mod config;
mod parallel;
mod trait_impl;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use parallel::{ChunkRequestSpec, download_chunk_with_retry, verify_merged_size};

use rdlp_core::{
    DownloadProgress, DownloadStats, ProgressCallback, RdlpError, Result, RetryConfig,
    check_http_response,
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
use crate::retry::{RetryPolicy, with_retry};
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

/// Re-export of the shared `ProbeResult` from `rdlp-http`. Single source of
/// truth for probe-result shape, used by both `HttpDownloader::probe` and
/// `BaseExtractor::detect_file_size`. Closes #306.
pub(crate) use rdlp_http::ProbeResult;

/// Multiple of the idle timeout used as a whole-request backstop.
///
/// Sized so only a request that is pathological reaches it, never one that is
/// merely slow: at the 60s default that is a ten-minute ceiling per fragment.
const TRANSFER_DEADLINE_MULTIPLE: u32 = 10;

/// Apply the two timeouts a media transfer needs, on their two distinct axes.
///
/// Read from wreq 6.0.0-rc.28's source rather than its doc lines, because the
/// two methods' names do not describe when they actually fire:
///
/// - **Before the response arrives**, `ResponseFuture::poll`
///   (`client/layer/timeout/future.rs:53-61`) polls *both* sleeps as plain
///   deadlines running from request start. So `read_timeout` alone already
///   bounds a connection that is accepted and then never answered — the
///   header phase is not the "read" phase its name suggests.
/// - **Once the body is arriving**, they diverge
///   (`client/layer/timeout/body.rs`): `ReadTimeoutBody` resets its timer on
///   every frame, so it never punishes a transfer that is slow but
///   progressing, while `TotalTimeoutBody` holds one sleep that is never
///   reset.
///
/// That is why both are set. `read_timeout` does the real work — silence
/// before the headers, and inactivity during the body. The total deadline
/// exists for the one case the idle timer cannot see: a body that dribbles a
/// frame at a time forever, resetting the idle timer on each one, which would
/// otherwise hold a fragment open indefinitely.
///
/// Wiring the idle value to `timeout` instead — as this code did briefly, and
/// as the DASH path did since its Item 8 — makes a 60s *total* deadline for
/// the whole transfer, which kills a large fragment on a slow link while bytes
/// are still arriving. `Config::read_timeout` documents itself as "per-read
/// inactivity, not total"; this is what honouring that requires.
///
/// Shared by the fragment and DASH segment paths so the two cannot drift back
/// to disagreeing about which axis they bound.
pub(crate) fn with_transfer_timeouts(
    req: wreq::RequestBuilder,
    idle: Duration,
) -> wreq::RequestBuilder {
    req.read_timeout(idle)
        .timeout(idle.saturating_mul(TRANSFER_DEADLINE_MULTIPLE))
}

/// The operator's headers, but only for a target on the seed's origin.
///
/// `Format.http_headers` carry Referer, Cookie, Authorization and Origin. A
/// manifest names its own fragment and segment URLs, so a compromised or
/// hostile playlist can point them at a host of its choosing; forwarding the
/// operator's headers there hands that host the user's credentials. The gate
/// is origin equality per RFC 6454 — scheme, host and port.
///
/// Fails closed on every uncertainty: no seed, a target that will not parse,
/// and an opaque origin on either side (two opaque origins are never equal,
/// so a `data:` or otherwise non-tuple origin can never match).
///
/// One function because this decision existed twice — once for the fragment
/// path (#273) and once for DASH's legacy MPD path (#319) — and two copies of
/// a credential gate is one copy too many. Both call sites now share it.
pub(crate) fn same_origin_headers(
    seed: Option<&url::Origin>,
    target_url: &str,
    headers: &HeaderMap,
) -> HeaderMap {
    let same_origin = match (seed, url::Url::parse(target_url).ok()) {
        (Some(seed), Some(target)) => *seed == target.origin(),
        _ => false,
    };
    if same_origin {
        headers.clone()
    } else {
        HeaderMap::new()
    }
}

/// HTTP status a single-part ranged response must carry (RFC 9110 §15.3.7).
///
/// A `200` means the server ignored `Range` — permitted by §14.2 — and the
/// content is the WHOLE representation, not the requested span. Writing such a
/// body at a position computed for one span is the corruption in #526 (parallel
/// chunk path) and #564 (HLS/DASH fragment path), so every ranged fetch accepts
/// this status and no other.
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

/// The inclusive byte span a ranged fetch asked the server for.
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
    /// Build a span, rejecting an inverted range.
    ///
    /// The invariant `start <= end` is what makes [`Self::len`]'s subtraction
    /// total; enforcing it here rather than at the call site means no caller
    /// can construct a span whose length underflows.
    pub(crate) const fn new(start: u64, end: u64) -> Option<Self> {
        if end < start {
            return None;
        }
        Some(Self { start, end })
    }

    /// Number of bytes the span covers.
    ///
    /// Cannot underflow: [`Self::new`] rejects `end < start`.
    pub(crate) const fn len(self) -> u64 {
        self.end - self.start + 1
    }
}

/// Confirm a ranged response actually carries the requested span before any of
/// its bytes are written into the output.
///
/// Two callers, both writing a ranged body at a position they computed in
/// advance:
/// - the parallel chunk downloader ([`download_chunk_with_retry`]), which
///   concatenates each chunk at a fixed offset in the merged output;
/// - the HLS/DASH fragment fetcher (`fragments::fetch_with_optional_range`),
///   which appends `#EXT-X-BYTERANGE` / `mediaRange` bodies sequentially (#564).
///
/// In both cases a response enclosing a different span silently relocates every
/// byte after it — the ~517 MB interior displacement in #526. RFC 9110 §15.3.7
/// places this duty on the client: "A client MUST inspect a 206 response's
/// Content-Type and Content-Range field(s) to determine what parts are enclosed
/// and whether additional requests are needed."
///
/// Only `Content-Range` is inspected here. The Content-Type half of that
/// sentence exists to distinguish a single-part response from a
/// `multipart/byteranges` one (§14.6), which arises only for a multi-range
/// request; this client always asks for exactly one range. A multipart body
/// would carry no top-level `Content-Range` anyway, so it is rejected by the
/// missing-header branch below rather than silently accepted.
///
/// Returns `Err` (never a silent acceptance) when the status is not 206, the
/// `Content-Range` is absent/malformed/invalid, or the enclosed span is not
/// exactly the one requested.
pub(crate) fn validate_range_response(
    response: &wreq::Response,
    span: RequestedSpan,
    url: &str,
) -> Result<()> {
    let redacted = || Some(rdlp_redact::RedactedUrlBuf::from(url));

    // §14.2 permits a server to ignore Range; the reply is then a 200 carrying
    // the WHOLE representation. Accepting it here is what wrote whole-file
    // content into a slot sized for one span.
    let status = response.status().as_u16();
    if status != HTTP_PARTIAL_CONTENT {
        return Err(RdlpError::Download {
            url: redacted(),
            message: format!(
                "ranged request for bytes {}-{} got HTTP {status}, expected \
                 {HTTP_PARTIAL_CONTENT} (Partial Content). The server ignored the Range \
                 header, so the body is the whole resource rather than the requested span \
                 and cannot be placed at this position in the output.",
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
                "ranged request for bytes {}-{} got a {HTTP_PARTIAL_CONTENT} response \
                 with a missing, malformed, or invalid Content-Range header; the enclosed \
                 span cannot be verified.",
                span.start, span.end
            ),
        });
    };

    // A wrong span is a per-response anomaly rather than a statement about
    // what the server supports — a retry against another CDN node plausibly
    // gets the right bytes. Reported as `Network` so `is_retryable_error`
    // accepts it and `download_chunk_with_retry` re-fetches, instead of
    // failing a multi-gigabyte download over one bad response.
    if range.first_pos != span.start || range.last_pos != span.end {
        return Err(RdlpError::Network {
            url: redacted(),
            message: format!(
                "ranged request for bytes {}-{} got Content-Range bytes {}-{}; the \
                 response encloses a different span than requested and would corrupt the \
                 output at this position.",
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

    /// Set the per-fragment / per-segment retry policy (`--fragment-retries`).
    #[must_use]
    pub fn with_fragment_retry_config(mut self, config: RetryConfig) -> Self {
        Arc::make_mut(&mut self.config).fragment_retry_config = config;
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

        let probed = with_retry(
            RetryPolicy::new(&self.config.retry_config, &"HTTP probe (F3)"),
            || {
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
            },
        )
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

        // NOTE: plain `with_retry`, not the cancellable form — this loop's
        // backoff sleeps are not themselves raced against `cancel`. That is
        // safe only because the sole production caller
        // (`download_chunk_with_retry`) wraps this whole call in
        // `with_retry_cancellable`. A future caller
        // invoking this directly and expecting a cancel to interrupt a backoff
        // would not get one.
        let response = with_retry(
            RetryPolicy::new(&self.config.retry_config, &"HTTP GET (range)"),
            || {
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
            },
        )
        .await?;

        // Confirm the response encloses exactly the requested span BEFORE any
        // of its bytes reach the chunk file (#526).
        let Some(span) = RequestedSpan::new(start, end) else {
            return Err(RdlpError::Download {
                url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                message: format!(
                    "internal error: chunk requested an inverted byte range {start}-{end}"
                ),
            });
        };
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
                    // Reject the frame BEFORE it reaches disk if it would push
                    // this chunk past the span its Content-Range promised, so
                    // an over-long body never lands on disk at all rather than
                    // being written and caught on the following iteration.
                    let chunk_len = chunk.len() as u64;
                    if downloaded.saturating_add(chunk_len) > expected_len {
                        // Retryable for the same reason as a wrong span: this
                        // is one malformed response, not a server capability.
                        return Err(RdlpError::Network {
                            url: Some(rdlp_redact::RedactedUrlBuf::from(url.as_str())),
                            message: format!(
                                "ranged chunk for bytes {start}-{end} delivered more than the \
                                 {expected_len} bytes its Content-Range promised; aborting to \
                                 avoid overrunning this chunk's offset in the merged output."
                            ),
                        });
                    }

                    writer.write_all(&chunk).await.map_err(|e| {
                        RdlpError::Io(std::io::Error::new(
                            e.kind(),
                            format!(
                                "failed to write to chunk file '{}': {e}",
                                chunk_path.display()
                            ),
                        ))
                    })?;
                    downloaded += chunk_len;

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

        // The stream ending is not proof the whole span arrived. hyper does
        // normally surface an interrupted body as an error, but that is a
        // property of the current implementation rather than a guarantee the
        // type system enforces: hyperium/hyper#3253 was a case where an
        // interrupted chunked body's error was swallowed and the stream simply
        // ended (that report concerns reading a chunked *request* body, so it
        // is an analogous decoder path rather than this exact one — the point
        // is that a silent short read has occurred in this decoder family, not
        // that it is known to occur here). A short chunk shifts every later
        // chunk in the merged output, so the byte count is verified
        // independently rather than trusted to the transport.
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

        let response = with_retry(
            RetryPolicy::new(&self.config.retry_config, &"HTTP GET"),
            || {
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
            },
        )
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

    // -----------------------------------------------------------------------
    // `ContentRange::parse` grammar branches (RFC 9110 §14.4).
    //
    // The chunk path decides whether a response may be written into its offset
    // slot from this parser's verdict, so each accept/reject branch is pinned
    // individually rather than only through the mockito-driven tests.
    // -----------------------------------------------------------------------

    #[test]
    fn parses_a_conformant_single_part_range() {
        let range = ContentRange::parse("bytes 0-1023/2048").expect("valid range must parse");
        assert_eq!(range.first_pos, 0);
        assert_eq!(range.last_pos, 1023);
        assert_eq!(range.complete_length, Some(2048));
    }

    /// `*` for complete-length is explicitly legal — "An asterisk character
    /// ("*") in place of the complete-length indicates that the representation
    /// length was unknown when the header field was generated" (§14.4). The
    /// span is still fully usable, so this MUST parse; rejecting it would
    /// break every server that cannot cheaply determine total length.
    #[test]
    fn accepts_unknown_complete_length() {
        let range =
            ContentRange::parse("bytes 0-1023/*").expect("unknown complete-length is legal");
        assert_eq!(range.first_pos, 0);
        assert_eq!(range.last_pos, 1023);
        assert_eq!(range.complete_length, None);
    }

    /// §14.4: a recipient that does not understand the range unit "MUST NOT
    /// attempt to recombine it with a stored representation" — and recombining
    /// is exactly what the chunk merge does.
    #[test]
    fn rejects_unknown_range_unit() {
        assert!(ContentRange::parse("items 0-1023/2048").is_none());
        assert!(ContentRange::parse("seconds 0-10/60").is_none());
    }

    #[test]
    fn accepts_case_insensitive_unit() {
        assert!(ContentRange::parse("BYTES 0-1023/2048").is_some());
    }

    /// §14.4 invalidity: "a last-pos value less than its first-pos value".
    #[test]
    fn rejects_last_pos_below_first_pos() {
        assert!(ContentRange::parse("bytes 1023-0/2048").is_none());
    }

    /// §14.4 invalidity: "a complete-length value less than or equal to its
    /// last-pos value". Positions are inclusive, so a 0-1023 span needs at
    /// least 1024 total bytes; 1024 is the tightest legal value.
    #[test]
    fn rejects_complete_length_not_above_last_pos() {
        assert!(ContentRange::parse("bytes 0-1023/1023").is_none());
        assert!(ContentRange::parse("bytes 0-1023/1024").is_some());
    }

    /// A single-position span is legal and covers exactly one byte.
    #[test]
    fn accepts_single_byte_span() {
        let range = ContentRange::parse("bytes 5-5/10").expect("single-byte span is valid");
        assert_eq!(range.first_pos, range.last_pos);
    }

    #[test]
    fn rejects_unsatisfied_range_form() {
        // `bytes */1234` accompanies a 416 and encloses no content.
        assert!(ContentRange::parse("bytes */1234").is_none());
    }

    #[test]
    fn rejects_structurally_malformed_values() {
        assert!(ContentRange::parse("").is_none());
        assert!(ContentRange::parse("bytes").is_none());
        assert!(ContentRange::parse("bytes 0-1023").is_none());
        assert!(ContentRange::parse("bytes abc-def/2048").is_none());
        assert!(ContentRange::parse("0-1023/2048").is_none());
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
