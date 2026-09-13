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

use wreq::header::HeaderMap;

use crate::content_range::ContentRange;
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
///
/// [`probe_size`] yields one only for a 206 or a 200 — any other status,
/// and any transport failure, is a [`ProbeError`] so a caller's retry
/// policy can see it. [`ProbeResult::from_response`] is the parse half on
/// its own and reads every status as data; a caller that sends through its
/// own gate uses that.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Total file size: the 206 `Content-Range` total, or the 200
    /// `Content-Length`. `None` if neither was parseable.
    pub size: Option<u64>,
    /// Whether the server honoured the `Range` header (HTTP 206).
    /// `false` on 200 (range ignored).
    pub supports_ranges: bool,
    /// The strong validator the response offered (§8.8), if any — captured
    /// so a caller can send it back as `If-Range` on a later resume.
    pub validator: Option<StrongValidator>,
    /// The 206 `Content-Range` total only; `None` on a 200 (a 200 body is
    /// not partial, so it has no "complete length" distinct from `size`).
    pub complete_length: Option<u64>,
    /// The response headers as received (empty on a network failure a
    /// caller folds into this shape), so a resume can judge a probe's 206
    /// with [`StrongValidator::verify_partial`] — which is lenient about an
    /// absent `Last-Modified` (§15.3.7: a 206 to `If-Range` SHOULD NOT
    /// repeat it) where comparing [`Self::validator`] would not be.
    pub headers: HeaderMap,
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
/// - 206 → parse total from `Content-Range`, `supports_ranges = true`,
///   `complete_length` set.
/// - 200 → server ignored Range; parse total from `Content-Length`,
///   `supports_ranges = false`, `complete_length = None`.
/// - other (4xx/5xx) → `Err(ProbeError::Status { status })`. Non-2xx is
///   a typed error, not a value; the caller decides whether it is
///   retryable and what to fall back to (issue #735).
///
/// `spec.window_bytes` controls how much body the server is asked for; the
/// response body is dropped without being read. Smaller windows trade off
/// slightly less wasted bandwidth on cancelled streams; larger windows may
/// align with a downstream caller's chunk-0 boundary.
///
/// # Errors
///
/// Returns [`ProbeError::Network`] on DNS, TLS, connect, or send failure,
/// and [`ProbeError::Status`] on any non-2xx response.
pub async fn probe_size(
    client: &wreq::Client,
    spec: ProbeSpec<'_>,
) -> Result<ProbeResult, ProbeError> {
    let resp = probe_request(client, &spec).send().await?;
    match resp.status().as_u16() {
        200 | 206 => Ok(ProbeResult::from_response(&resp)),
        status => Err(ProbeError::Status { status }),
    }
}

/// The probe's request, unsent: `GET Range: bytes=0-{window_bytes - 1}`
/// with the identity pin and, when given, `If-Range`.
///
/// Split from [`probe_size`] so a caller with its own retry policy can send
/// it through that policy and parse with [`ProbeResult::from_response`] —
/// one request half and one parse half, whichever seam sends.
pub fn probe_request(client: &wreq::Client, spec: &ProbeSpec<'_>) -> wreq::RequestBuilder {
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
    download_request(client, spec.url, spec.headers)
        .timeout(spec.timeout)
        .ranged(range, spec.validator)
}

impl ProbeResult {
    /// Read a probe's answer off its status and headers; every status is
    /// data (see [`probe_size`] for the per-status mapping), never an error.
    #[must_use]
    pub fn from_response(resp: &wreq::Response) -> Self {
        let headers = resp.headers().clone();
        match resp.status().as_u16() {
            206 => {
                // One §14.4 grammar for the probe and the ranged download
                // paths: a length the probe reports is exactly one a later
                // 206 would be verified against, never a laxer reading.
                let complete_length =
                    ContentRange::from_headers(&headers).and_then(ContentRange::complete_length);
                Self {
                    size: complete_length,
                    supports_ranges: true,
                    validator: StrongValidator::from_headers(&headers),
                    complete_length,
                    headers,
                }
            }
            200 => Self {
                size: resp.content_length(),
                supports_ranges: false,
                validator: StrongValidator::from_headers(&headers),
                complete_length: None,
                headers,
            },
            _ => Self {
                size: None,
                supports_ranges: false,
                validator: None,
                complete_length: None,
                headers,
            },
        }
    }

    /// What a caller reports when the probe never produced a response
    /// (transport failure after its retries): no size, no ranges, no
    /// validator, no headers — the shape that sends a download sequential.
    #[must_use]
    pub fn unanswered() -> Self {
        Self {
            size: None,
            supports_ranges: false,
            validator: None,
            complete_length: None,
            headers: HeaderMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    fn make_client() -> wreq::Client {
        wreq::Client::new()
    }

    /// A header-only probe with no validator — the shape every status test
    /// sends.
    fn plain_spec(url: &str) -> ProbeSpec<'_> {
        ProbeSpec {
            url,
            headers: None,
            window_bytes: 1,
            timeout: Duration::from_secs(5),
            validator: None,
        }
    }

    /// The parse half reads a non-2xx as "no info" — `size`, `validator`
    /// and `complete_length` all `None`, `supports_ranges = false` — so a
    /// caller sending through its own gate (the downloader's retry seam)
    /// can treat a 416 as data while [`probe_size`] itself still types it.
    #[tokio::test]
    async fn from_response_reads_a_non_2xx_as_no_info() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/file")
            .with_status(416)
            .with_header("ETag", "\"v1\"")
            .with_body("")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/file", server.url());
        let resp = probe_request(&client, &plain_spec(&url))
            .send()
            .await
            .expect("send");
        let result = ProbeResult::from_response(&resp);

        assert_eq!(result.size, None);
        assert!(!result.supports_ranges);
        assert_eq!(result.validator, None);
        assert_eq!(result.complete_length, None);
        assert_eq!(
            result
                .headers
                .get("etag")
                .map(wreq::header::HeaderValue::as_bytes),
            Some(&b"\"v1\""[..])
        );
        mock.assert_async().await;
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
        let err = probe_size(&client, plain_spec(&url))
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
        let err = probe_size(&client, plain_spec(&url))
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
        let err = probe_size(&client, plain_spec(&url))
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

    /// The probe reads `Content-Range` through the one §14.4 grammar: a
    /// non-`bytes` unit is not a length the resume path may compare its
    /// sidecar against ("MUST NOT attempt to recombine"), so it yields no
    /// `complete_length` — where a lax `split('/')` would have said 10.
    #[tokio::test]
    async fn probe_206_with_non_bytes_unit_has_no_complete_length() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .with_status(206)
            .with_header("content-range", "items 0-0/10")
            .with_body("x")
            .create_async()
            .await;
        let client = make_client();
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
        assert_eq!(r.complete_length, None);
        assert_eq!(r.size, None);
        assert!(r.supports_ranges);
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

    /// `from_response` is the parse half `probe_size` composes; a caller
    /// that sends the request through its own retry seam must read the same
    /// fields off the same response.
    #[tokio::test]
    async fn from_response_yields_what_probe_size_yields() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/1234")
            .with_header("etag", "\"v1\"")
            .with_body("x")
            .expect(2)
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let url = format!("{}/f", server.url());
        let spec = || ProbeSpec {
            url: &url,
            headers: None,
            window_bytes: 1,
            timeout: Duration::from_secs(5),
            validator: None,
        };
        let composed = probe_size(&client, spec()).await.unwrap();
        let resp = probe_request(&client, &spec()).send().await.unwrap();
        let parsed = ProbeResult::from_response(&resp);
        assert_eq!(parsed.size, composed.size);
        assert_eq!(parsed.size, Some(1234));
        assert_eq!(parsed.supports_ranges, composed.supports_ranges);
        assert!(parsed.supports_ranges);
        assert_eq!(parsed.validator, composed.validator);
        assert!(matches!(parsed.validator, Some(StrongValidator::ETag(_))));
        assert_eq!(parsed.complete_length, composed.complete_length);
        assert_eq!(parsed.complete_length, Some(1234));
        assert_eq!(parsed.headers, composed.headers);
        assert_eq!(parsed.headers.get("etag").unwrap(), "\"v1\"");
    }

    /// A 206 to `If-Range` that omits `Last-Modified` (§15.3.7 SHOULD NOT
    /// repeat it) still confirms a `Last-Modified` validator through the
    /// carried headers, where `validator` alone would read as "none offered".
    #[tokio::test]
    async fn probe_carries_headers_so_a_last_modified_validator_can_be_confirmed() {
        const LAST_MODIFIED: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/10")
            .with_body("x")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let lm = StrongValidator::LastModified(
            crate::validator::ImfFixdate::parse(LAST_MODIFIED).unwrap(),
        );
        let r = probe_size(
            &client,
            ProbeSpec {
                url: &format!("{}/f", server.url()),
                headers: None,
                window_bytes: 1,
                timeout: Duration::from_secs(5),
                validator: Some(&lm),
            },
        )
        .await
        .unwrap();
        assert_eq!(r.validator, None);
        assert_eq!(r.headers.get("content-range").unwrap(), "bytes 0-0/10");
        assert_eq!(lm.verify_partial(&r.headers), Ok(()));
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
