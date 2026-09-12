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
pub(crate) use parallel::{ChunkRequestSpec, download_chunk_with_retry, verify_output_size};

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

/// The span a ranged fetch asked the server for.
///
/// Two shapes share one validator: a **closed** span with a known end (the
/// parallel chunk fetch and the HLS/DASH fragment fetcher, both of which write
/// a body at a byte offset computed *before* the request), and an
/// **open-ended** span requesting "from `start` to the end of the resource"
/// (`Range: bytes={start}-`, the sequential-resume path, which does not
/// know the total length until the response headers arrive —
/// #674). Only the closed case can compare `last_pos` directly; the
/// open-ended case instead checks, when the server discloses a total, that
/// `last_pos` reaches its end — a shorter tail is exactly the shape that
/// silently truncates the file the caller is about to resume onto.
///
/// When the server answers `bytes {start}-{last_pos}/*` (total genuinely
/// unknown, §14.4), this span check has nothing to compare `last_pos`
/// against and accepts it. The transfer is not left unbounded, though: the
/// caller's byte-count and final-size checks (`ExpectedTransfer`,
/// `verify_output_size`) fall back to `Content-Length + resume_from` for
/// their own expected total in that case, so a short or over-long body is
/// still caught — just one layer up, once the actual byte count is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpectedSpan {
    /// `Range: bytes={start}-{end}` — end already known.
    Closed(RequestedSpan),
    /// `Range: bytes={start}-` — end determined by the server's response.
    OpenEnded {
        /// First byte position requested (inclusive).
        start: u64,
    },
}

impl ExpectedSpan {
    /// Render the span that was ASKED for, in error messages, using the
    /// `Content-Range` response header's `bytes 0-1023` notation (a space,
    /// no `/total` since none was requested) — not the *request*'s
    /// `Range: bytes=0-1023` header form, which uses `=` and never appears
    /// in these messages. Every message pairs this against the response's
    /// actual `Content-Range`, so the two use one notation.
    fn describe(self) -> String {
        match self {
            Self::Closed(span) => format!("bytes {}-{}", span.start, span.end),
            Self::OpenEnded { start } => format!("bytes {start}-"),
        }
    }
}

/// Confirm a ranged response actually carries the requested span before any of
/// its bytes are written into the output.
///
/// Three callers, each writing a ranged body at a position it already knows:
/// - the parallel chunk downloader ([`download_chunk_with_retry`]), which
///   concatenates each chunk at a fixed offset in the merged output;
/// - the HLS/DASH fragment fetcher (`fragments::fetch_with_optional_range`),
///   which appends `#EXT-X-BYTERANGE` / `mediaRange` bodies sequentially (#564);
/// - `trait_impl::download_with_resume_with_cancel`, which appends the resumed
///   tail at the end of a partial file (#674).
///
/// The first two pass [`ExpectedSpan::Closed`] and additionally know the
/// exact end, so their span is checked byte-for-byte; the resume caller
/// passes [`ExpectedSpan::OpenEnded`] and only learns the end from the
/// response.
///
/// In every case a response enclosing a different span silently relocates
/// every byte after it — the ~517 MB interior displacement in #526. RFC 9110
/// §15.3.7 places this duty on the client: "A client MUST inspect a 206
/// response's Content-Type and Content-Range field(s) to determine what parts
/// are enclosed and whether additional requests are needed."
///
/// Only `Content-Range` is inspected here. The Content-Type half of that
/// sentence exists to distinguish a single-part response from a
/// `multipart/byteranges` one (§14.6), which arises only for a multi-range
/// request; this client always asks for exactly one range. A multipart body
/// would carry no top-level `Content-Range` anyway, so it is rejected by the
/// missing-header branch below rather than silently accepted.
///
/// Returns the parsed [`ContentRange`] (never a silent acceptance) when the
/// status is 206, `Content-Range` is present and valid, and the enclosed span
/// matches what was requested — closed spans exactly, open-ended spans at
/// `first_pos` and, when a total is disclosed, at `last_pos` too. Returns
/// `Err` otherwise. Callers that need the enclosed span or its disclosed
/// total (currently the resume path, to avoid re-parsing the same header)
/// use the returned `ContentRange`; callers that only need the pass/fail
/// verdict (the chunk and fragment fetchers) discard it with `?;`.
pub(crate) fn validate_range_response(
    response: &wreq::Response,
    span: ExpectedSpan,
    url: &str,
) -> Result<ContentRange> {
    let redacted = || Some(rdlp_redact::RedactedUrlBuf::from(url));
    let requested = span.describe();

    // The sequential-resume path is the only caller that passes
    // `OpenEnded` (see the type's doc); a failure there means an
    // operator is staring at a stalled resume with a partial file on disk,
    // so its messages carry the recovery step. The chunk/fragment paths pass
    // `Closed` and retry or fail the whole download internally — an operator
    // never reads their message mid-flow, so it stays free of instructions
    // that don't apply to them.
    let resume_guidance = matches!(span, ExpectedSpan::OpenEnded { .. })
        .then_some(" Please delete the partial file and restart the download.")
        .unwrap_or_default();

    // §14.2 permits a server to ignore Range; the reply is then a 200 carrying
    // the WHOLE representation. Accepting it here is what wrote whole-file
    // content into a slot sized for one span.
    let status = response.status().as_u16();
    if status != HTTP_PARTIAL_CONTENT {
        // For a resume the operator-facing fact is that the server will not
        // continue from the partial file; for a chunk it is that the body
        // cannot be placed at its offset. Same defect, two audiences.
        let message = match span {
            ExpectedSpan::OpenEnded { .. } => format!(
                "Server does not support resume (expected HTTP {HTTP_PARTIAL_CONTENT}, got \
                 {status}). Cannot continue download without overwriting existing \
                 data.{resume_guidance}"
            ),
            ExpectedSpan::Closed(_) => format!(
                "ranged request for {requested} got HTTP {status}, expected \
                 {HTTP_PARTIAL_CONTENT} (Partial Content). The server ignored the Range \
                 header, so the body is the whole resource rather than the requested span \
                 and cannot be placed at this position in the output."
            ),
        };
        return Err(RdlpError::Download {
            url: redacted(),
            message,
        });
    }

    // §15.3.7.1: a single-part 206 MUST carry Content-Range. Without it there
    // is no way to confirm which span arrived.
    let Some(range) = ContentRange::from_headers(response.headers()) else {
        return Err(RdlpError::Download {
            url: redacted(),
            message: format!(
                "ranged request for {requested} got a {HTTP_PARTIAL_CONTENT} response \
                 with a missing, malformed, or invalid Content-Range header; the enclosed \
                 span cannot be verified.{resume_guidance}",
            ),
        });
    };

    // Closed spans must match exactly. Open-ended spans must at least start
    // where requested, and — only when the server discloses a total — must
    // also reach its end; a shorter tail is legal per §14.2 ("may only be
    // possible to send a portion... expecting the client to re-request the
    // remainder") but is not this client's re-request protocol, so it is
    // refused rather than silently appended as if it were the whole tail.
    let span_matches = match span {
        ExpectedSpan::Closed(closed) => {
            range.first_pos == closed.start && range.last_pos == closed.end
        }
        ExpectedSpan::OpenEnded { start } => {
            range.first_pos == start
                && range
                    .complete_length
                    .is_none_or(|total| range.last_pos == total - 1)
        }
    };
    if !span_matches {
        // A wrong span is a per-response anomaly rather than a statement
        // about what the server supports — a retry against another CDN node
        // plausibly gets the right bytes. Reported as `Network` so
        // `is_retryable_error` accepts it: the chunk path's per-chunk
        // `with_retry` re-fetches the chunk, and the resume path's own
        // `with_retry` (this error propagates unrewrapped — see
        // `trait_impl::download_with_resume_with_cancel`) re-issues the
        // resume request, instead of either failing a multi-gigabyte
        // download over one bad response.
        return Err(RdlpError::Network {
            url: redacted(),
            message: format!(
                "ranged request for {requested} got Content-Range bytes {}-{}{}; the \
                 response encloses a different span than requested and would corrupt the \
                 output at this position.{resume_guidance}",
                range.first_pos,
                range.last_pos,
                range
                    .complete_length
                    .map_or_else(String::new, |total| format!("/{total}")),
            ),
        });
    }

    Ok(range)
}

/// A byte-counted transfer's known-in-advance length and human-readable name.
///
/// Shared by every ranged/full-body fetch that must catch a truncated or
/// over-long response before it corrupts the output: the parallel chunk fetch,
/// the sequential-resume append, and the sequential fresh download (#674) all
/// stream a body whose exact length is known before the first byte arrives
/// (from a `Content-Range` span or a `Content-Length` header), so the check
/// and its error wording live once here rather than once per call site.
pub(crate) struct ExpectedTransfer<'a> {
    /// Exact byte count the transfer is expected to deliver.
    pub(crate) expected_len: u64,
    /// Human-readable description of the transfer, for error messages (e.g.
    /// `"ranged chunk for bytes 0-1023"`, `"sequential download"`).
    pub(crate) context: &'a str,
}

impl ExpectedTransfer<'_> {
    /// Reject an incoming frame that would push `downloaded` past
    /// `expected_len`, BEFORE the frame is written to disk — so an over-long
    /// body never lands on disk at all, rather than being written and only
    /// caught once the stream ends.
    pub(crate) fn reject_overlong(
        &self,
        downloaded: u64,
        incoming_len: u64,
        url: &str,
    ) -> Result<()> {
        if downloaded.saturating_add(incoming_len) > self.expected_len {
            return Err(RdlpError::Network {
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
                message: format!(
                    "{} delivered more than the {} bytes it promised; aborting to avoid an \
                     incomplete or corrupted output.",
                    self.context, self.expected_len
                ),
            });
        }
        Ok(())
    }

    /// Confirm the finished transfer delivered exactly `expected_len` bytes.
    ///
    /// The stream ending is not proof the whole span arrived — hyper does
    /// normally surface an interrupted body as an error, but that is a
    /// property of the current implementation rather than a guarantee the
    /// type system enforces (hyperium/hyper#3253 is a case where an
    /// interrupted chunked body's error was swallowed and the stream simply
    /// ended). A short body shifts every later byte in a merged output, or
    /// leaves a resumed/fresh file silently truncated, so the byte count is
    /// verified independently rather than trusted to the transport.
    pub(crate) fn confirm_exact(&self, downloaded: u64, url: &str) -> Result<()> {
        if downloaded != self.expected_len {
            return Err(RdlpError::Network {
                url: Some(rdlp_redact::RedactedUrlBuf::from(url)),
                message: format!(
                    "{} ended after {downloaded} of {} bytes; the transfer is incomplete.",
                    self.context, self.expected_len
                ),
            });
        }
        Ok(())
    }
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
        validate_range_response(&response, ExpectedSpan::Closed(span), &url)?;
        let expected_len = span.len();
        let context = format!("ranged chunk for bytes {start}-{end}");
        let transfer = ExpectedTransfer {
            expected_len,
            context: &context,
        };

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
                    let chunk_len = chunk.len() as u64;
                    transfer.reject_overlong(downloaded, chunk_len, &url)?;

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

        // A 206 enclosing less than was requested is also legal on its own
        // terms (§14.2: a server "may only be possible (or efficient) to send a
        // portion of the requested ranges first, while expecting the client to
        // re-request the remaining portions later"). Re-requesting only the
        // remainder is the spec's answer; `confirm_exact` returns a RETRYABLE
        // error so `download_chunk_with_retry` re-fetches the whole chunk
        // instead, which is correct but wasteful. Tracked as a follow-up.
        transfer.confirm_exact(downloaded, &url)?;

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
        // #674: when the server discloses a length, hold the sequential body
        // to it exactly — the same over-long/short guard the parallel chunk
        // path has always had, extended to the single-file path that never
        // had it. A chunked-encoding response with no `Content-Length` can't
        // be bounded this way and downloads unchecked, as before.
        let context = "sequential download";
        let transfer = total_size.map(|expected_len| ExpectedTransfer {
            expected_len,
            context,
        });
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

            if let Some(ref transfer) = transfer {
                transfer.reject_overlong(downloaded, chunk.len() as u64, url_string.as_ref())?;
            }

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

        // #674: `reject_overlong` bounds each frame but the stream can still
        // end short (see `ExpectedTransfer::confirm_exact`'s doc comment for
        // why the end of the stream is not itself proof of completeness).
        if let Some(ref transfer) = transfer {
            transfer.confirm_exact(downloaded, url_string.as_ref())?;
        }
        if let Some(total) = total_size {
            parallel::verify_output_size(path, total, url_string.as_ref()).await?;
        }

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
    //! Tests for `ContentRange`'s parsing and header-reading behavior. The
    //! standalone `parse_content_range_total` wrapper this module used to
    //! cover was removed once the resume path started consuming the
    //! `ContentRange` `validate_range_response` already parses (#674 review)
    //! instead of re-parsing the header itself; `from_headers` remains
    //! covered indirectly through the mockito-driven `validate_range_response`
    //! tests in `tests.rs`.
    use super::*;

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

    /// A `complete-length` that parses as neither a digit string nor `*` is
    /// unparseable, distinct from the `*` (genuinely unknown) case above.
    #[test]
    fn rejects_unparseable_complete_length() {
        assert!(ContentRange::parse("bytes 0-1023/notanumber").is_none());
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
