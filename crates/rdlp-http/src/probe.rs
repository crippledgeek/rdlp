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

use std::time::Duration;

use wreq::header::HeaderMap;

use crate::request::{RangeSpec, RangedRequest, download_request};
use crate::validator::StrongValidator;

/// Default probe window (256 KiB). Matches the rdlp-downloader F3 probe
/// constant. Callers MAY pass a smaller window (e.g. `1`) for
/// header-only probes where body bandwidth is constrained.
pub const DEFAULT_PROBE_WINDOW_BYTES: u64 = 256 * 1_024;

/// Everything one `probe_size` call needs — grouped so the function stays
/// under the project's 3-positional-parameter ceiling.
pub struct ProbeSpec<'a> {
    /// The URL to probe.
    pub url: &'a str,
    /// Headers applied verbatim — no same-origin gating (callers gate at
    /// their own boundary).
    pub headers: Option<&'a HeaderMap>,
    /// How much body the server is asked for (see [`probe_size`]).
    pub window_bytes: u64,
    /// Bounds the entire request.
    pub timeout: Duration,
    /// When given, sent as `If-Range` alongside the probe's `Range`
    /// (§13.1.5), so a resume probe can confirm the resource is unchanged.
    pub validator: Option<&'a StrongValidator>,
}

/// Outcome of a single HTTP probe.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Total file size: the 206 `Content-Range` total, or the 200
    /// `Content-Length`. `None` if neither was parseable.
    pub size: Option<u64>,
    /// Whether the server honoured the `Range` header (HTTP 206).
    /// `false` on 200 (range ignored), 4xx, 5xx, or network failure.
    pub supports_ranges: bool,
    /// The strong validator the response offered (§8.8), if any — captured
    /// so a caller can send it back as `If-Range` on a later resume.
    pub validator: Option<StrongValidator>,
    /// The 206 `Content-Range` total only; `None` on a 200 (a 200 body is
    /// not partial, so it has no "complete length" distinct from `size`).
    pub complete_length: Option<u64>,
}

/// Errors that can arise during a single probe.
///
/// The helper never retries; callers compose retry policy as needed.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// Underlying wreq error (DNS, TLS, connect, etc.).
    #[error("probe network error: {0}")]
    Network(#[from] wreq::Error),
}

/// Probe a URL with a single `GET Range: bytes=0-{window_bytes - 1}` request.
///
/// Status handling:
/// - 206 → parse total from `Content-Range`, `supports_ranges = true`,
///   `complete_length` set.
/// - 200 → server ignored Range; parse total from `Content-Length`,
///   `supports_ranges = false`, `complete_length = None`.
/// - other (4xx/5xx) → `size`/`validator`/`complete_length` all `None`,
///   `supports_ranges = false`. Non-2xx is "no info", not an error; caller
///   decides next step.
///
/// `spec.window_bytes` controls how much body the server is asked for; the
/// response body is dropped without being read. Smaller windows trade off
/// slightly less wasted bandwidth on cancelled streams; larger windows may
/// align with a downstream caller's chunk-0 boundary.
///
/// # Errors
///
/// Returns [`ProbeError::Network`] on DNS, TLS, connect, or send failure.
pub async fn probe_size(
    client: &wreq::Client,
    spec: ProbeSpec<'_>,
) -> Result<ProbeResult, ProbeError> {
    // Clamp window_bytes to >= 1 (security-review LOW). Passing 0 would
    // wrap saturating_sub to u64::MAX, producing `bytes=0-18446744073709551615`
    // — effectively a full-body GET, defeating the probe's intent.
    let window = spec.window_bytes.max(1);
    // `window >= 1` (the `.max(1)` above), so `end = window - 1 >= start = 0`
    // always — constructing the variant directly instead of going through
    // the fallible `RangeSpec::span` avoids an unreachable `expect`.
    let range = RangeSpec::Span {
        start: 0,
        end: window - 1,
    };
    let resp = download_request(client, spec.url, spec.headers)
        .timeout(spec.timeout)
        .ranged(range, spec.validator)
        .send()
        .await?;
    Ok(match resp.status().as_u16() {
        206 => {
            let complete_length = parse_content_range_total(resp.headers());
            ProbeResult {
                size: complete_length,
                supports_ranges: true,
                validator: StrongValidator::from_headers(resp.headers()),
                complete_length,
            }
        }
        200 => ProbeResult {
            size: resp.content_length(),
            supports_ranges: false,
            validator: StrongValidator::from_headers(resp.headers()),
            complete_length: None,
        },
        _ => ProbeResult {
            size: None,
            supports_ranges: false,
            validator: None,
            complete_length: None,
        },
    })
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
        let result = probe_size(
            &client,
            ProbeSpec {
                url: &url,
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
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
        let result = probe_size(
            &client,
            ProbeSpec {
                url: &url,
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
        .await
        .expect("probe should succeed");

        assert_eq!(result.size, Some(BODY_LEN as u64));
        assert!(!result.supports_ranges);
        mock.assert_async().await;
    }

    /// Non-2xx response → `ProbeResult { size: None, supports_ranges: false }`, no error.
    #[tokio::test]
    async fn probe_non_2xx_returns_none_no_error() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(403)
            .with_body("Forbidden")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let result = probe_size(
            &client,
            ProbeSpec {
                url: &url,
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
        .await
        .expect("probe should succeed even on 403");

        assert_eq!(result.size, None);
        assert!(!result.supports_ranges);
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
        let result = probe_size(
            &client,
            ProbeSpec {
                url: &url,
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
        .await
        .expect("probe should succeed");

        assert_eq!(result.size, None);
        // Server returned 206 so supports_ranges must be true even if we
        // could not parse the total.
        assert!(result.supports_ranges);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn probe_captures_strong_etag_and_complete_length() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .match_header("accept-encoding", "identity")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/1234")
            .with_header("etag", "\"v1\"")
            .with_body("x")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let r = probe_size(
            &client,
            ProbeSpec {
                url: &format!("{}/f", server.url()),
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(r.complete_length, Some(1234));
        assert_eq!(r.size, Some(1234));
        assert!(matches!(r.validator, Some(StrongValidator::ETag(_))));
    }

    #[tokio::test]
    async fn probe_ignores_weak_etag() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/10")
            .with_header("etag", "W/\"v1\"")
            .with_body("x")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let r = probe_size(
            &client,
            ProbeSpec {
                url: &format!("{}/f", server.url()),
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(r.validator, None);
    }

    #[tokio::test]
    async fn probe_sends_if_range_when_given_a_validator() {
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("if-range", "\"v1\"")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/10")
            .with_body("x")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let v = StrongValidator::ETag(crate::validator::StrongEntityTag::parse("\"v1\"").unwrap());
        probe_size(
            &client,
            ProbeSpec {
                url: &format!("{}/f", server.url()),
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: Some(&v),
            },
        )
        .await
        .unwrap();
        m.assert_async().await;
    }

    #[tokio::test]
    async fn probe_200_has_no_complete_length_but_may_have_validator() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .with_status(200)
            .with_header("content-length", "5")
            .with_header("etag", "\"v1\"")
            .with_body("hello")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let r = probe_size(
            &client,
            ProbeSpec {
                url: &format!("{}/f", server.url()),
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(r.size, Some(5));
        assert_eq!(r.complete_length, None);
        assert!(r.validator.is_some());
        assert!(!r.supports_ranges);
    }
}
