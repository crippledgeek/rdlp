//! The one way a download request is built.
//!
//! Every request whose bytes may later be resumed or spliced pins
//! `Accept-Encoding: identity`: RFC 9110 §14.1.2 computes byte ranges over the
//! *encoded* sequence and §8.4 defines the representation in terms of its
//! coded form, so offsets are comparable between attempts only under one
//! coding. wreq 6.0.0-rc.28 inserts its own `Accept-Encoding` whenever the
//! header is vacant (the documented `Range` exemption is not implemented) and
//! decodes any `Content-Encoding` it recognises, stripping the header — so the
//! four toggles below disable both for this request only, leaving a rogue
//! `Content-Encoding` visible for the downloader to reject.

use std::sync::LazyLock;

use wreq::header::{ACCEPT_ENCODING, HeaderMap, HeaderValue, IF_RANGE, RANGE};

use crate::validator::StrongValidator;

/// The `Accept-Encoding: identity` pin, built once.
///
/// Applied through [`wreq::RequestBuilder::headers`], whose `replace_headers`
/// merge REPLACES an existing entry for a repeated key — unlike
/// `RequestBuilder::header`, which `HeaderMap::append`s despite its doc
/// comment claiming replacement (confirmed by reading wreq 6.0.0-rc.28's
/// `header_sensitive`). Applying it after the caller's headers is therefore
/// what makes this a pin rather than a second `Accept-Encoding` line when a
/// caller (e.g. `--header`) already set one.
static IDENTITY_PIN: LazyLock<HeaderMap> = LazyLock::new(|| {
    let mut map = HeaderMap::with_capacity(1);
    map.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    map
});

/// A GET with the encoding pin applied; `headers` are the operator's.
#[must_use = "a RequestBuilder does nothing until `.send()`d"]
pub fn download_request(
    client: &wreq::Client,
    url: &str,
    headers: Option<&HeaderMap>,
) -> wreq::RequestBuilder {
    let mut req = client.get(url);
    if let Some(h) = headers {
        req = req.headers(h.clone());
    }
    req.headers(IDENTITY_PIN.clone())
        .gzip(false)
        .brotli(false)
        .zstd(false)
        .deflate(false)
}

/// The byte range a request asks for; both bounds inclusive (§14.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// `bytes=start-end`.
    Span {
        /// First byte offset, inclusive.
        start: u64,
        /// Last byte offset, inclusive.
        end: u64,
    },
    /// `bytes=start-` — everything from `start` to the end.
    From(u64),
}

impl RangeSpec {
    /// `None` for an inverted span, so [`Self::byte_len`] can never underflow.
    #[must_use]
    pub const fn span(start: u64, end: u64) -> Option<Self> {
        if end < start {
            None
        } else {
            Some(Self::Span { start, end })
        }
    }

    /// The starting byte offset.
    #[must_use]
    pub const fn start(self) -> u64 {
        match self {
            Self::Span { start, .. } | Self::From(start) => start,
        }
    }

    /// Bytes covered; unknown for an open-ended range.
    ///
    /// Named `byte_len`, not `len` — this is not a container, and clippy's
    /// `len_without_is_empty` would otherwise want an `is_empty` that could
    /// never be true ([`Self::span`] rejects `end < start`, so every
    /// constructed `Span` has `byte_len() >= 1`) and so could never fail;
    /// that would document the invariant a second time rather than check it.
    #[must_use]
    pub const fn byte_len(self) -> Option<u64> {
        match self {
            Self::Span { start, end } => Some(end - start + 1),
            Self::From(_) => None,
        }
    }

    /// The `Range` header value (`bytes=a-b` / `bytes=a-`).
    #[must_use]
    pub fn header_value(self) -> String {
        match self {
            Self::Span { start, end } => format!("bytes={start}-{end}"),
            Self::From(start) => format!("bytes={start}-"),
        }
    }
}

/// Adds `Range` and, with a validator, `If-Range` — always together, so
/// §13.1.5's "no If-Range without Range" holds by construction.
///
/// Applied the same way as [`IDENTITY_PIN`]: through
/// [`wreq::RequestBuilder::headers`], which REPLACES a caller's `Range` or
/// `If-Range` line where `RequestBuilder::header` would append a second one
/// and leave the server free to honour either.
pub trait RangedRequest {
    /// Attach `range` (and `validator`'s `If-Range`, when given) to `self`.
    #[must_use]
    fn ranged(self, range: RangeSpec, validator: Option<&StrongValidator>) -> Self;
}

impl RangedRequest for wreq::RequestBuilder {
    fn ranged(self, range: RangeSpec, validator: Option<&StrongValidator>) -> Self {
        let mut map = HeaderMap::with_capacity(2);
        if let Some(v) = validator {
            map.insert(IF_RANGE, v.if_range_value());
        }
        let value = range.header_value();
        // `bytes=<digits>-[<digits>]` is visible ASCII, so `from_str` cannot
        // fail. The `Err` arm keeps that honest without a panic path: wreq's
        // `header` re-runs the same conversion and stores its failure as the
        // builder's pending error (`client/request.rs` `header_sensitive`),
        // so `send()` fails loudly rather than going out with no `Range`.
        match HeaderValue::from_str(&value) {
            Ok(v) => {
                map.insert(RANGE, v);
                self.headers(map)
            }
            Err(_) => self.headers(map).header(RANGE, value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    #[test]
    fn range_spec_header_values() {
        assert_eq!(RangeSpec::span(0, 9).unwrap().header_value(), "bytes=0-9");
        assert_eq!(RangeSpec::From(1000).header_value(), "bytes=1000-");
        assert!(RangeSpec::span(5, 4).is_none());
        assert_eq!(RangeSpec::span(5, 5).unwrap().byte_len(), Some(1));
        assert_eq!(RangeSpec::From(5).byte_len(), None);
    }

    #[tokio::test]
    async fn download_request_pins_identity_and_suppresses_wreq_auto_encoding() {
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("accept-encoding", "identity")
            .with_status(200)
            .with_body("ok")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let resp = download_request(&client, &format!("{}/f", server.url()), None)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        m.assert_async().await;
    }

    #[tokio::test]
    async fn caller_accept_encoding_is_replaced_not_appended() {
        // `Matcher::Exact` requires EVERY `accept-encoding` line to equal
        // "identity" (mockito's `matches_values` is an `all()` over each
        // distinct header line) — so this fails if the pin appended a second
        // line alongside the caller's "gzip" instead of replacing it.
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("accept-encoding", Matcher::Exact("identity".to_string()))
            .with_status(200)
            .with_body("ok")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let mut caller_headers = HeaderMap::new();
        caller_headers.insert("accept-encoding", HeaderValue::from_static("gzip"));
        let resp = download_request(
            &client,
            &format!("{}/f", server.url()),
            Some(&caller_headers),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        m.assert_async().await;
    }

    #[tokio::test]
    async fn an_unrelated_caller_header_is_forwarded() {
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("referer", "https://example.test/page")
            .with_status(200)
            .with_body("ok")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let mut caller_headers = HeaderMap::new();
        caller_headers.insert(
            "referer",
            HeaderValue::from_static("https://example.test/page"),
        );
        let resp = download_request(
            &client,
            &format!("{}/f", server.url()),
            Some(&caller_headers),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        m.assert_async().await;
    }

    #[tokio::test]
    async fn download_request_leaves_a_rogue_content_encoding_visible() {
        // With decoding disabled per request the header must survive so the
        // downloader can reject it (spec: the response check is load-bearing).
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/f")
            .with_status(200)
            .with_header("content-encoding", "gzip")
            .with_body("not really gzip")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let resp = download_request(&client, &format!("{}/f", server.url()), None)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.headers().get("content-encoding").unwrap(), "gzip");
        assert_eq!(resp.bytes().await.unwrap().as_ref(), b"not really gzip");
    }

    #[tokio::test]
    async fn ranged_sends_range_and_if_range_together() {
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("range", "bytes=10-")
            .match_header("if-range", "\"v1\"")
            .with_status(206)
            .with_header("content-range", "bytes 10-19/20")
            .with_body("0123456789")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let v = StrongValidator::ETag(crate::validator::StrongEntityTag::parse("\"v1\"").unwrap());
        let resp = download_request(&client, &format!("{}/f", server.url()), None)
            .ranged(RangeSpec::From(10), Some(&v))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 206);
        m.assert_async().await;
    }

    /// A caller-supplied `Range` (an operator `--header`, say) is REPLACED
    /// by the one this download computed, never joined by it: two `Range`
    /// lines would leave the server free to honour either, and the body
    /// would then be placed at the wrong offset. `Matcher::Exact` fails if
    /// any `range` line differs, so an appended second line is caught.
    #[tokio::test]
    async fn ranged_replaces_a_caller_range_rather_than_appending() {
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("range", Matcher::Exact("bytes=10-".to_string()))
            .match_header("if-range", Matcher::Exact("\"v1\"".to_string()))
            .with_status(206)
            .with_header("content-range", "bytes 10-19/20")
            .with_body("0123456789")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let mut caller_headers = HeaderMap::new();
        caller_headers.insert("range", HeaderValue::from_static("bytes=0-1"));
        caller_headers.insert("if-range", HeaderValue::from_static("\"stale\""));
        let v = StrongValidator::ETag(crate::validator::StrongEntityTag::parse("\"v1\"").unwrap());
        let resp = download_request(
            &client,
            &format!("{}/f", server.url()),
            Some(&caller_headers),
        )
        .ranged(RangeSpec::From(10), Some(&v))
        .send()
        .await
        .unwrap();
        assert_eq!(resp.status().as_u16(), 206);
        m.assert_async().await;
    }

    /// Both `RangeSpec` shapes reach the wire as exactly one `Range` line
    /// with the §14.1.2 value: `Matcher::Exact` fails on a missing line, a
    /// second line, or any other text.
    #[tokio::test]
    async fn ranged_sends_exactly_the_span_or_open_ended_value() {
        let mut server = Server::new_async().await;
        let span = server
            .mock("GET", "/s")
            .match_header("range", Matcher::Exact("bytes=3-7".to_string()))
            .with_status(206)
            .with_header("content-range", "bytes 3-7/8")
            .with_body("34567")
            .create_async()
            .await;
        let open = server
            .mock("GET", "/o")
            .match_header("range", Matcher::Exact("bytes=3-".to_string()))
            .with_status(206)
            .with_header("content-range", "bytes 3-7/8")
            .with_body("34567")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        let s = download_request(&client, &format!("{}/s", server.url()), None)
            .ranged(RangeSpec::span(3, 7).unwrap(), None)
            .send()
            .await
            .unwrap();
        let o = download_request(&client, &format!("{}/o", server.url()), None)
            .ranged(RangeSpec::From(3), None)
            .send()
            .await
            .unwrap();
        assert_eq!(s.status().as_u16(), 206);
        assert_eq!(o.status().as_u16(), 206);
        span.assert_async().await;
        open.assert_async().await;
    }

    #[tokio::test]
    async fn ranged_without_validator_sends_no_if_range() {
        let mut server = Server::new_async().await;
        let m = server
            .mock("GET", "/f")
            .match_header("range", "bytes=0-1")
            .match_header("if-range", Matcher::Missing)
            .with_status(206)
            .with_header("content-range", "bytes 0-1/2")
            .with_body("ab")
            .create_async()
            .await;
        let client =
            crate::HttpClientFactory::from_config(&crate::HttpClientConfig::default()).build();
        download_request(&client, &format!("{}/f", server.url()), None)
            .ranged(RangeSpec::span(0, 1).unwrap(), None)
            .send()
            .await
            .unwrap();
        m.assert_async().await;
    }
}
