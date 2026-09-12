//! What a response to a ranged request means — decided once, here, for the
//! sequential resume, both parallel chunk paths and the ranged fragment path.
//!
//! RFC 9110 §13.2.2 step 5 and §14.2 leave a server three answers to
//! `Range` + `If-Range`: 206 (condition true, range applicable), 200
//! (condition false, or Range ignored, or a zero-length representation — all
//! indistinguishable and all meaning "the body is the whole current
//! representation"), and 416 (range unsatisfiable; only reachable when the
//! precondition held). Everything else is an error before any byte is written.

use rdlp_core::{RdlpError, Result};
use rdlp_http::{RangeSpec, StrongValidator};
use rdlp_redact::RedactedUrlBuf;

use super::ContentRange;

/// HTTP status a single-part ranged response carries (RFC 9110 §15.3.7).
pub(crate) const HTTP_PARTIAL_CONTENT: u16 = 206;
/// RFC 9110 §15.5.17.
const HTTP_RANGE_NOT_SATISFIABLE: u16 = 416;
const HTTP_OK: u16 = 200;

/// What was asked, so the answer can be checked against it.
pub(crate) struct RangedRequestMeta<'a> {
    pub range: RangeSpec,
    /// The validator sent as `If-Range`, if any.
    pub sent_validator: Option<&'a StrongValidator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeVerdict {
    /// 206 enclosing exactly the requested span, same representation.
    Partial { range: ContentRange },
    /// 200: the body is the entire current representation (§13.2.2 step 5).
    Replaced,
    /// 416, with `bytes */complete-length` when the server sent it (§14.4).
    Unsatisfiable { complete_length: Option<u64> },
}

fn download_err(url: &str, message: String) -> RdlpError {
    RdlpError::Download {
        url: Some(RedactedUrlBuf::from(url)),
        message,
    }
}

pub(crate) fn range_verdict(
    response: &wreq::Response,
    meta: &RangedRequestMeta<'_>,
    url: &str,
) -> Result<RangeVerdict> {
    let headers = response.headers();
    let status = response.status();

    // §14.1.2 / §8.4: offsets are over the coded form; a coded body can never
    // be spliced at an offset computed for the identity form. Checked on every
    // status because a 200 written from zero is equally wrong when coded.
    if let Some(coding) = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        && !coding.trim().eq_ignore_ascii_case("identity")
    {
        return Err(download_err(
            url,
            format!(
                "response is content-coded ({coding}); byte offsets are over the coded form \
                 (RFC 9110 §14.1.2) and the body cannot be placed at a byte position"
            ),
        ));
    }

    match status.as_u16() {
        HTTP_PARTIAL_CONTENT => {
            let Some(range) = ContentRange::from_headers(headers) else {
                return Err(download_err(
                    url,
                    format!(
                        "ranged request for {} got a 206 response with a missing, malformed, or \
                         invalid Content-Range header; the enclosed span cannot be verified.",
                        meta.range.header_value()
                    ),
                ));
            };
            match meta.range {
                RangeSpec::Span { start, end }
                    if range.first_pos() != start || range.last_pos() != end =>
                {
                    // One bad response, not a server capability: `Network` so the
                    // chunk retry re-fetches (#526).
                    return Err(RdlpError::Network {
                        url: Some(RedactedUrlBuf::from(url)),
                        message: format!(
                            "ranged request for bytes {start}-{end} got Content-Range bytes {}-{}; the \
                             response encloses a different span than requested and would corrupt the \
                             output at this position.",
                            range.first_pos(),
                            range.last_pos()
                        ),
                    });
                }
                RangeSpec::From(start) if range.first_pos() != start => {
                    return Err(download_err(
                        url,
                        format!(
                            "resume response starts at byte {} but the partial file ends at {start}; \
                             appending it would corrupt the file.",
                            range.first_pos()
                        ),
                    ));
                }
                _ => {}
            }
            if let Some(sent) = meta.sent_validator
                && let Err(mismatch) = sent.verify_partial(headers)
            {
                return Err(download_err(
                    url,
                    format!(
                        "ranged response for {} is not the representation this download started \
                         with: {mismatch}",
                        meta.range.header_value()
                    ),
                ));
            }
            Ok(RangeVerdict::Partial { range })
        }
        HTTP_OK if meta.sent_validator.is_some() => Ok(RangeVerdict::Replaced),
        HTTP_OK => Err(download_err(
            url,
            format!(
                "ranged request for {} got HTTP 200, expected 206 (Partial Content). The server \
                 ignored the Range header, so the body is the whole resource rather than the \
                 requested span and cannot be placed at this position in the output.",
                meta.range.header_value()
            ),
        )),
        HTTP_RANGE_NOT_SATISFIABLE => Ok(RangeVerdict::Unsatisfiable {
            complete_length: ContentRange::unsatisfied_from_headers(headers),
        }),
        code => Err(RdlpError::Http {
            status: code,
            reason: status.canonical_reason().unwrap_or("Unknown").to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Mock, Server, ServerGuard};
    use rdlp_http::validator::StrongEntityTag;

    async fn served(
        server: &mut ServerGuard,
        status: usize,
        headers: &[(&str, &str)],
    ) -> (Mock, wreq::Response) {
        let mut m = server.mock("GET", "/r").with_status(status).with_body("x");
        for (k, v) in headers.iter().copied() {
            m = m.with_header(k, v);
        }
        let m = m.create_async().await;
        let client =
            rdlp_http::HttpClientFactory::from_config(&rdlp_http::HttpClientConfig::default())
                .build();
        let resp = rdlp_http::download_request(&client, &format!("{}/r", server.url()), None)
            .send()
            .await
            .unwrap();
        (m, resp)
    }
    fn etag(s: &str) -> StrongValidator {
        StrongValidator::ETag(StrongEntityTag::parse(s).unwrap())
    }
    fn span_meta(start: u64, end: u64, v: Option<&StrongValidator>) -> RangedRequestMeta<'_> {
        RangedRequestMeta {
            range: RangeSpec::span(start, end).unwrap(),
            sent_validator: v,
        }
    }

    #[tokio::test]
    async fn partial_with_matching_span_and_etag() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(
            &mut s,
            206,
            &[("content-range", "bytes 0-0/10"), ("etag", "\"a\"")],
        )
        .await;
        let v = etag("\"a\"");
        assert!(matches!(
            range_verdict(&r, &span_meta(0, 0, Some(&v)), "u").unwrap(),
            RangeVerdict::Partial { .. }
        ));
    }
    #[tokio::test]
    async fn partial_without_sent_validator_needs_no_etag() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 0-0/10")]).await;
        assert!(matches!(
            range_verdict(&r, &span_meta(0, 0, None), "u").unwrap(),
            RangeVerdict::Partial { .. }
        ));
    }
    #[tokio::test]
    async fn partial_missing_etag_when_one_was_sent_is_an_error() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 0-0/10")]).await;
        let v = etag("\"a\"");
        assert!(matches!(
            range_verdict(&r, &span_meta(0, 0, Some(&v)), "u"),
            Err(RdlpError::Download { .. })
        ));
    }
    #[tokio::test]
    async fn partial_with_different_etag_is_an_error() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(
            &mut s,
            206,
            &[("content-range", "bytes 0-0/10"), ("etag", "\"b\"")],
        )
        .await;
        let v = etag("\"a\"");
        let err = range_verdict(&r, &span_meta(0, 0, Some(&v)), "u").unwrap_err();
        assert!(matches!(err, RdlpError::Download { .. }));
        assert!(err.to_string().contains("validator changed"));
    }
    #[tokio::test]
    async fn partial_wrong_span_is_retryable_network_error() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 1-1/10")]).await;
        assert!(matches!(
            range_verdict(&r, &span_meta(0, 0, None), "u"),
            Err(RdlpError::Network { .. })
        ));
    }
    #[tokio::test]
    async fn open_ended_range_checks_first_pos_only() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 5-9/10")]).await;
        let meta = RangedRequestMeta {
            range: RangeSpec::From(5),
            sent_validator: None,
        };
        assert!(matches!(
            range_verdict(&r, &meta, "u").unwrap(),
            RangeVerdict::Partial { .. }
        ));
        let bad = RangedRequestMeta {
            range: RangeSpec::From(4),
            sent_validator: None,
        };
        assert!(matches!(
            range_verdict(&r, &bad, "u"),
            Err(RdlpError::Download { .. })
        ));
    }
    #[tokio::test]
    async fn partial_without_content_range_is_an_error() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[]).await;
        assert!(matches!(
            range_verdict(&r, &span_meta(0, 0, None), "u"),
            Err(RdlpError::Download { .. })
        ));
    }
    #[tokio::test]
    async fn content_coded_response_is_rejected_on_any_status() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(
            &mut s,
            206,
            &[
                ("content-range", "bytes 0-0/10"),
                ("content-encoding", "gzip"),
            ],
        )
        .await;
        let err = range_verdict(&r, &span_meta(0, 0, None), "u").unwrap_err();
        assert!(err.to_string().contains("content-coded"));
        let (_m2, r2) = served(&mut s, 200, &[("content-encoding", "br")]).await;
        let v = etag("\"a\"");
        assert!(range_verdict(&r2, &span_meta(0, 0, Some(&v)), "u").is_err());
    }
    #[tokio::test]
    async fn content_encoding_identity_is_accepted() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(
            &mut s,
            206,
            &[
                ("content-range", "bytes 0-0/10"),
                ("content-encoding", "identity"),
            ],
        )
        .await;
        assert!(range_verdict(&r, &span_meta(0, 0, None), "u").is_ok());
    }
    #[tokio::test]
    async fn ok_after_if_range_is_replaced() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 200, &[]).await;
        let v = etag("\"a\"");
        assert_eq!(
            range_verdict(&r, &span_meta(0, 0, Some(&v)), "u").unwrap(),
            RangeVerdict::Replaced
        );
    }
    #[tokio::test]
    async fn ok_without_if_range_means_server_ignored_range() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 200, &[]).await;
        let err = range_verdict(&r, &span_meta(0, 0, None), "u").unwrap_err();
        assert!(matches!(err, RdlpError::Download { .. }));
        assert!(err.to_string().contains("ignored the Range"));
    }
    #[tokio::test]
    async fn range_not_satisfiable_with_and_without_length() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 416, &[("content-range", "bytes */1234")]).await;
        assert_eq!(
            range_verdict(&r, &span_meta(0, 0, None), "u").unwrap(),
            RangeVerdict::Unsatisfiable {
                complete_length: Some(1234)
            }
        );
        let (_m2, r2) = served(&mut s, 416, &[]).await;
        assert_eq!(
            range_verdict(&r2, &span_meta(0, 0, None), "u").unwrap(),
            RangeVerdict::Unsatisfiable {
                complete_length: None
            }
        );
        let (_m3, r3) = served(&mut s, 416, &[("content-range", "bytes 0-1/1234")]).await;
        assert_eq!(
            range_verdict(&r3, &span_meta(0, 0, None), "u").unwrap(),
            RangeVerdict::Unsatisfiable {
                complete_length: None
            }
        );
    }
    #[tokio::test]
    async fn other_statuses_become_http_errors() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 503, &[]).await;
        assert!(matches!(
            range_verdict(&r, &span_meta(0, 0, None), "u"),
            Err(RdlpError::Http { status: 503, .. })
        ));
    }
    #[test]
    fn parse_unsatisfied() {
        assert_eq!(ContentRange::parse_unsatisfied("bytes */1234"), Some(1234));
        assert_eq!(ContentRange::parse_unsatisfied("bytes 0-1/1234"), None);
        assert_eq!(ContentRange::parse_unsatisfied("items */5"), None);
        assert_eq!(ContentRange::parse_unsatisfied("bytes */*"), None);
    }
}
