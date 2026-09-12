//! Shared HTTP-probe helper.
//!
//! Performs a single F3-style `GET Range` request to determine
//! `Content-Range` total (or `Content-Length` fallback) without
//! downloading the full body. Used by both the downloader
//! (`HttpDownloader::probe`) and the extractor
//! (`BaseExtractor::detect_file_size`).
//!
//! Threat model: this helper is leaf-level — no retry, no header
//! gating. Callers compose retry / cancel / header-trust as needed.
//! A non-2xx status is a typed [`ProbeError::Status`] — this crate does
//! not know which statuses are worth retrying, so callers classify and
//! retry (or fall back) using their own retry policy (issue #735).

use std::time::Duration;

/// Default probe window (256 KiB). Matches the rdlp-downloader F3 probe
/// constant. Callers MAY pass a smaller window (e.g. `1`) for
/// header-only probes where body bandwidth is constrained.
pub const DEFAULT_PROBE_WINDOW_BYTES: u64 = 256 * 1_024;

/// Outcome of a single HTTP probe that produced a usable response.
///
/// Only 206 and 200 yield a value. Any other status, and any transport
/// failure, is a [`ProbeError`] so a caller's retry policy can see it.
#[derive(Debug, Clone, Copy)]
pub struct ProbeResult {
    /// Total file size, parsed from `Content-Range` (206) or
    /// `Content-Length` (200). `None` if neither was parseable.
    pub size: Option<u64>,
    /// Whether the server honoured the `Range` header (HTTP 206).
    /// `false` on 200 (range ignored).
    pub supports_ranges: bool,
}

/// Errors that can arise during a single probe.
///
/// The helper never retries; callers compose retry policy as needed.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// Underlying wreq error (DNS, TLS, connect, etc.).
    #[error("probe network error: {0}")]
    Network(#[from] wreq::Error),
    /// Non-2xx HTTP status. Unclassified — this crate does not decide
    /// what is retryable; callers map `status` through their own retry
    /// policy (e.g. `rdlp_core::is_retryable_error`).
    #[error("probe HTTP {status}")]
    Status {
        /// The response status code.
        status: u16,
    },
}

/// Probe a URL with a single `GET Range: bytes=0-{window_bytes - 1}` request.
///
/// Status handling:
/// - 206 → parse total from `Content-Range`, `supports_ranges = true`.
/// - 200 → server ignored Range; parse total from `Content-Length`,
///   `supports_ranges = false`.
/// - other (4xx/5xx) → `Err(ProbeError::Status { status })`. Non-2xx is
///   a typed error, not a value; the caller decides whether it is
///   retryable and what to fall back to (issue #735).
///
/// `window_bytes` controls how much body the server is asked for; the
/// response body is dropped without being read. Smaller windows trade
/// off slightly less wasted bandwidth on cancelled streams; larger
/// windows may align with a downstream caller's chunk-0 boundary.
///
/// `headers` are applied verbatim — no same-origin gating (callers
/// gate at their own boundary). `timeout` bounds the entire request.
///
/// # Errors
///
/// Returns [`ProbeError::Network`] on DNS, TLS, connect, or send failure,
/// and [`ProbeError::Status`] on any non-2xx response.
pub async fn probe_size(
    client: &wreq::Client,
    url: &str,
    headers: Option<&wreq::header::HeaderMap>,
    window_bytes: u64,
    timeout: Duration,
) -> Result<ProbeResult, ProbeError> {
    // Clamp window_bytes to >= 1 (security-review LOW). Passing 0 would
    // wrap saturating_sub to u64::MAX, producing `bytes=0-18446744073709551615`
    // — effectively a full-body GET, defeating the probe's intent.
    let window = window_bytes.max(1);
    let range = format!("bytes=0-{}", window - 1);
    let mut req = client.get(url).timeout(timeout).header("Range", &range);
    if let Some(h) = headers {
        req = req.headers(h.clone());
    }
    let resp = req.send().await?;
    match resp.status().as_u16() {
        206 => Ok(ProbeResult {
            size: parse_content_range_total(resp.headers()),
            supports_ranges: true,
        }),
        200 => Ok(ProbeResult {
            size: resp.content_length(),
            supports_ranges: false,
        }),
        status => Err(ProbeError::Status { status }),
    }
}

fn parse_content_range_total(headers: &wreq::header::HeaderMap) -> Option<u64> {
    headers
        .get("content-range")?
        .to_str()
        .ok()?
        .split('/')
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
#[allow(
    clippy::significant_drop_tightening,
    reason = "mockito::Server is a temporary owned by each test fn and dropped at end of scope; tightening would require restructuring every test"
)]
mod tests {
    use super::*;
    use mockito::Server;

    fn make_client() -> wreq::Client {
        wreq::Client::new()
    }

    /// 206 response with Content-Range header → `size` parsed, `supports_ranges = true`.
    #[tokio::test]
    async fn probe_206_parses_content_range() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(206)
            .with_header("Content-Range", "bytes 0-0/123456")
            .with_body("")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let result = probe_size(&client, &url, None, 1, Duration::from_secs(5))
            .await
            .expect("probe should succeed");

        assert_eq!(result.size, Some(123_456));
        assert!(result.supports_ranges);
        mock.assert_async().await;
    }

    /// 200 response → `Content-Length` used as size, `supports_ranges = false`.
    ///
    /// mockito overrides the Content-Length header to match the actual body
    /// byte length, so the test body length IS the asserted size. (A 64 KiB
    /// body keeps the test fast while still exercising a real multi-byte
    /// length parse.)
    #[tokio::test]
    async fn probe_200_falls_back_to_content_length() {
        const BODY_LEN: usize = 64 * 1024;
        let body = vec![0u8; BODY_LEN];
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(200)
            .with_body(&body[..])
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let result = probe_size(&client, &url, None, 1, Duration::from_secs(5))
            .await
            .expect("probe should succeed");

        assert_eq!(result.size, Some(BODY_LEN as u64));
        assert!(!result.supports_ranges);
        mock.assert_async().await;
    }

    /// Non-2xx response → `Err(ProbeError::Status { status })`, not a value.
    #[tokio::test]
    async fn probe_403_returns_typed_status_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(403)
            .with_body("Forbidden")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let err = probe_size(&client, &url, None, 1, Duration::from_secs(5))
            .await
            .expect_err("403 must be a typed error, not a value");

        assert!(matches!(err, ProbeError::Status { status: 403 }));
        mock.assert_async().await;
    }

    /// A retryable-shaped (5xx) status is ALSO a typed error at this layer —
    /// this crate does not classify retryability, callers do (issue #735).
    #[tokio::test]
    async fn probe_503_returns_typed_status_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(503)
            .with_body("Service Unavailable")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let err = probe_size(&client, &url, None, 1, Duration::from_secs(5))
            .await
            .expect_err("503 must be a typed error");

        assert!(matches!(err, ProbeError::Status { status: 503 }));
        mock.assert_async().await;
    }

    /// 404 is a typed error too, distinct status carried through.
    #[tokio::test]
    async fn probe_404_returns_typed_status_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(404)
            .with_body("Not Found")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let err = probe_size(&client, &url, None, 1, Duration::from_secs(5))
            .await
            .expect_err("404 must be a typed error");

        assert!(matches!(err, ProbeError::Status { status: 404 }));
        mock.assert_async().await;
    }

    /// Malformed Content-Range (no `/`) → `size = None`, `supports_ranges = true`.
    #[tokio::test]
    async fn probe_206_malformed_content_range_returns_none_size() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(206)
            .with_header("Content-Range", "bytes 0-0-garbage")
            .with_body("")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let result = probe_size(&client, &url, None, 1, Duration::from_secs(5))
            .await
            .expect("probe should succeed");

        assert_eq!(result.size, None);
        // Server returned 206 so supports_ranges must be true even if we
        // could not parse the total.
        assert!(result.supports_ranges);
        mock.assert_async().await;
    }
}
