//! What a response to a ranged request means — decided once, here, for the
//! sequential resume, both parallel chunk paths and the ranged fragment path.
//!
//! RFC 9110 §13.2.2 step 5 and §14.2 leave a server three answers to
//! `Range` + `If-Range`: 206 (condition true, range applicable), 200
//! (condition false, or Range ignored, or a zero-length representation — all
//! indistinguishable and all meaning "the body is the whole current
//! representation"), and 416 (range unsatisfiable; only reachable when the
//! precondition held). Everything else is an error before any byte is written.

use rdlp_core::{RdlpError, Result, check_http_response};
use rdlp_http::{RangeSpec, StrongValidator, ValidatorMismatch};
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
    /// The representation's complete length as recorded under
    /// `sent_validator` (the resume sidecar's, or the probe's), if known.
    /// §8.8.1 makes a strong validator unique across versions, so a 206
    /// naming that validator has this length even when its `Content-Range`
    /// says `*` — an open-ended tail is held to it either way.
    pub known_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangeVerdict {
    /// 206 enclosing exactly the requested span, same representation.
    Partial { range: ContentRange },
    /// 200: the body is the entire current representation (§13.2.2 step 5).
    Replaced,
    /// 416, with `bytes */complete-length` when the server sent it (§14.4).
    Unsatisfiable { complete_length: Option<u64> },
    /// 206 at the requested offset whose validator is absent, not strong, or
    /// different — produced ONLY for an open-ended resume (`RangeSpec::From`).
    /// §15.3.7.3 forbids combining without a shared strong validator, and a
    /// resume's only exit is the fresh download; an error would send the
    /// orchestrator straight back into the same resume. A bounded chunk
    /// (`Span`) has no partial to restart and keeps erroring inside
    /// [`range_verdict`], so this variant never reaches the chunk paths.
    Mismatched { mismatch: ValidatorMismatch },
}

fn download_err(url: &str, message: String) -> RdlpError {
    RdlpError::Download {
        url: Some(RedactedUrlBuf::from(url)),
        message,
    }
}

/// A bounded span's length, or a typed error naming the caller bug.
///
/// Both `download_range_with_progress` and `fetch_with_optional_range` ask
/// for the length of a span they just built via `RangeSpec::span`, which
/// never yields `RangeSpec::From` — so `byte_len()` returning `None` here
/// would be a caller bug (an open-ended range slipped into a bounded-span
/// call site), never a legitimate runtime condition. One helper so neither
/// site re-derives the arithmetic `byte_len` already owns.
pub(crate) fn bounded_len(span: RangeSpec, url: &str) -> Result<u64> {
    span.byte_len().ok_or_else(|| {
        download_err(
            url,
            "internal error: open-ended range where a bounded span was required".to_string(),
        )
    })
}

/// Let a verdict-bearing status through untouched; everything else still
/// goes through [`check_http_response`] so 5xx/429 stay retryable at the
/// caller's retry layer exactly as before this function existed.
///
/// 200 (§13.2.2 step 5), 206 (§15.3.7) and 416 (§15.5.17) are not failures —
/// they are the three answers [`range_verdict`] interprets. Filtering them
/// out here (as a bare `check_http_response` call did) makes 416 and any
/// other non-2xx status unreachable in `range_verdict`, discarding a 416's
/// `complete_length` and reporting the generic `Http` shape for the other
/// two even though `range_verdict` already builds the identical shape for
/// truly-unhandled statuses.
pub(crate) fn admit_for_verdict(response: wreq::Response) -> Result<wreq::Response> {
    match response.status().as_u16() {
        HTTP_OK | HTTP_PARTIAL_CONTENT | HTTP_RANGE_NOT_SATISFIABLE => Ok(response),
        _ => {
            check_http_response(&response)?;
            Ok(response)
        }
    }
}

/// [`admit_for_verdict`] plus the verdict, read once: a 206 enclosing the
/// wrong span for `meta` is the retryable `Network` error [`range_verdict`]
/// reports, and running inside the resume request's retry loop turns it
/// into a fresh request (as the chunk path's outer loop does for a chunk)
/// instead of an error at the orchestrator. Every other outcome — the other
/// verdict errors included — comes back with the response for the caller to
/// act on.
///
/// # Errors
/// `admit_for_verdict`'s non-answer statuses and the wrong-span `Network`
/// error; both are for the retry layer.
pub(crate) fn admit_partial_span(
    response: wreq::Response,
    meta: &RangedRequestMeta<'_>,
    url: &str,
) -> Result<(wreq::Response, Result<RangeVerdict>)> {
    let response = admit_for_verdict(response)?;
    match range_verdict(&response, meta, url) {
        Err(e @ RdlpError::Network { .. }) => Err(e),
        verdict => Ok((response, verdict)),
    }
}

/// The only `Content-Encoding` a download may accept is `identity`.
///
/// Byte offsets are over the coded form (RFC 9110 §14.1.2; §8.4 defines the
/// representation "in terms of the coded form"), so a coded body can never be
/// placed at an offset computed for the identity form — and a plain GET
/// written from zero is equally wrong, because the offset a later resume
/// computes from the file on disk would then be over gzip bytes. The request
/// pins `Accept-Encoding: identity`, but honouring it is only a SHOULD
/// (§12.5.3), so this response-side check is load-bearing on every status
/// and every path: `range_verdict` for ranged requests, and the sequential
/// GET directly.
///
/// Fails closed on every uncertainty. `get_all` sees every `Content-Encoding`
/// line, so `identity` followed by `gzip` on a second line is coded; a value
/// that is not exactly `identity` (ASCII case-insensitive, surrounding
/// whitespace ignored) is coded, including a non-UTF-8 one — an unparsable
/// coding is not evidence of the identity form.
///
/// Applied to the statuses whose body is placed (200, 206) and never to a
/// 416, whose body is an error page no path writes: a gzipped 416 must
/// still yield its `*/L` verdict, not a `Download` error.
pub(crate) fn reject_content_coding(headers: &wreq::header::HeaderMap, url: &str) -> Result<()> {
    let coded = headers
        .get_all("content-encoding")
        .iter()
        .find(|v| !v.as_bytes().trim_ascii().eq_ignore_ascii_case(b"identity"));
    coded.map_or(Ok(()), |coding| {
        Err(download_err(
            url,
            format!(
                "response is content-coded ({}); byte offsets are over the coded form \
                 (RFC 9110 §14.1.2) and the body cannot be placed at a byte position",
                String::from_utf8_lossy(coding.as_bytes())
            ),
        ))
    })
}

pub(crate) fn range_verdict(
    response: &wreq::Response,
    meta: &RangedRequestMeta<'_>,
    url: &str,
) -> Result<RangeVerdict> {
    let headers = response.headers();
    let status = response.status();

    match status.as_u16() {
        HTTP_PARTIAL_CONTENT => {
            reject_content_coding(headers, url)?;
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
                // An open-ended resume must at least start where the partial
                // ends, and — when a total is known, from this response or
                // from the record made under the same validator — must also
                // reach its end (#674): a shorter tail is legal per §14.2
                // ("may only be possible to send a portion... expecting the
                // client to re-request the remainder") but is not this
                // client's re-request protocol, so it is refused rather than
                // silently appended as if it were the whole tail. Retryable
                // for the same reason as a chunk's wrong span: one bad
                // response, not a server capability — the resume request is
                // re-issued (`admit_partial_span`) before the partial is
                // given up on.
                RangeSpec::From(start)
                    if range.first_pos() != start
                        || range
                            .complete_length()
                            .or(meta.known_total)
                            // A recorded total of 0 has no last byte at all
                            // (`checked_sub` yields `None`), so any enclosed
                            // span is a wrong span rather than an underflow.
                            .is_some_and(|total| {
                                total.checked_sub(1) != Some(range.last_pos())
                            }) =>
                {
                    return Err(RdlpError::Network {
                        url: Some(RedactedUrlBuf::from(url)),
                        message: format!(
                            "resume request for bytes {start}- got Content-Range bytes {}-{}{}; \
                             the response encloses a different span than the partial file's \
                             remainder and would corrupt the file if appended.",
                            range.first_pos(),
                            range.last_pos(),
                            range
                                .complete_length()
                                .map_or_else(String::new, |total| format!("/{total}")),
                        ),
                    });
                }
                _ => {}
            }
            if let Some(sent) = meta.sent_validator
                && let Err(mismatch) = sent.verify_partial(headers)
            {
                return match meta.range {
                    RangeSpec::From(_) => Ok(RangeVerdict::Mismatched { mismatch }),
                    RangeSpec::Span { .. } => Err(download_err(
                        url,
                        format!(
                            "ranged response for {} is not the representation this download \
                             started with: {mismatch}",
                            meta.range.header_value()
                        ),
                    )),
                };
            }
            Ok(RangeVerdict::Partial { range })
        }
        HTTP_OK if meta.sent_validator.is_some() => {
            reject_content_coding(headers, url)?;
            Ok(RangeVerdict::Replaced)
        }
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
    use crate::http::state::etag;
    use mockito::{Mock, Server, ServerGuard};

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
    fn span_meta(start: u64, end: u64, v: Option<&StrongValidator>) -> RangedRequestMeta<'_> {
        RangedRequestMeta {
            range: RangeSpec::span(start, end).unwrap(),
            sent_validator: v,
            known_total: None,
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
    /// The same mismatch on an open-ended resume is data, not an error: the
    /// resume path restarts on it. Both the absent and the different shape.
    #[tokio::test]
    async fn open_ended_partial_with_mismatched_validator_is_a_verdict() {
        let mut s = Server::new_async().await;
        let v = etag("\"a\"");
        let meta = RangedRequestMeta {
            range: RangeSpec::From(0),
            sent_validator: Some(&v),
            known_total: None,
        };
        let (_m, r) = served(
            &mut s,
            206,
            &[("content-range", "bytes 0-9/10"), ("etag", "\"b\"")],
        )
        .await;
        assert!(matches!(
            range_verdict(&r, &meta, "u").unwrap(),
            RangeVerdict::Mismatched {
                mismatch: ValidatorMismatch::Different { .. }
            }
        ));
        let (_m2, r2) = served(&mut s, 206, &[("content-range", "bytes 0-9/10")]).await;
        assert_eq!(
            range_verdict(&r2, &meta, "u").unwrap(),
            RangeVerdict::Mismatched {
                mismatch: ValidatorMismatch::Missing
            }
        );
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
    /// An open-ended resume is checked at both ends (#674): it must start
    /// where the partial ends and, when a total is disclosed, reach `total -
    /// 1` exactly. Either miss is the retryable wrong-span shape, the same
    /// `Network` class a bounded chunk's wrong span gets.
    #[tokio::test]
    async fn open_ended_range_checks_first_pos_and_declared_end() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 5-9/10")]).await;
        let meta = RangedRequestMeta {
            range: RangeSpec::From(5),
            sent_validator: None,
            known_total: None,
        };
        assert!(matches!(
            range_verdict(&r, &meta, "u").unwrap(),
            RangeVerdict::Partial { .. }
        ));
        let bad = RangedRequestMeta {
            range: RangeSpec::From(4),
            sent_validator: None,
            known_total: None,
        };
        assert!(matches!(
            range_verdict(&r, &bad, "u"),
            Err(RdlpError::Network { .. })
        ));

        // Starts where asked but stops one byte short of the declared total.
        let (_m, short) = served(&mut s, 206, &[("content-range", "bytes 5-8/10")]).await;
        assert!(matches!(
            range_verdict(&short, &meta, "u"),
            Err(RdlpError::Network { .. })
        ));
        // An undisclosed total (`*`) leaves nothing to compare the end against.
        let (_m, unknown) = served(&mut s, 206, &[("content-range", "bytes 5-8/*")]).await;
        assert!(matches!(
            range_verdict(&unknown, &meta, "u").unwrap(),
            RangeVerdict::Partial { .. }
        ));
    }

    /// `admit_partial_span` hands a wrong-span 206 back as the retryable
    /// error (so the send loop re-issues the request) and every verdict-
    /// bearing status (200/206/416) back as the response paired with its
    /// verdict, read once — including a verdict that is itself an error
    /// (here a 200 with no `If-Range` sent, which `range_verdict` refuses
    /// as `Download`), which the caller surfaces rather than the retry layer.
    #[tokio::test]
    async fn admit_partial_span_retries_only_a_wrong_span() {
        let mut s = Server::new_async().await;
        let meta = RangedRequestMeta {
            range: RangeSpec::From(5),
            sent_validator: None,
            known_total: None,
        };
        let (_m, wrong) = served(&mut s, 206, &[("content-range", "bytes 6-9/10")]).await;
        assert!(matches!(
            admit_partial_span(wrong, &meta, "u"),
            Err(RdlpError::Network { .. })
        ));
        let (_m, replaced) = served(&mut s, 200, &[]).await;
        let (admitted, verdict) = admit_partial_span(replaced, &meta, "u").unwrap();
        assert_eq!(admitted.status().as_u16(), 200);
        assert!(matches!(verdict, Err(RdlpError::Download { .. })));
        let (_m, unsatisfiable) = served(&mut s, 416, &[]).await;
        let (admitted, verdict) = admit_partial_span(unsatisfiable, &meta, "u").unwrap();
        assert_eq!(admitted.status().as_u16(), 416);
        assert_eq!(
            verdict.unwrap(),
            RangeVerdict::Unsatisfiable {
                complete_length: None
            }
        );
    }

    /// A `bytes N-M/*` 206 is held to the length recorded under the same
    /// validator (§8.8.1): short of it is the retryable wrong-span error,
    /// exactly at it is `Partial`, and with nothing recorded it is
    /// unchecked as before.
    #[tokio::test]
    async fn open_ended_range_is_held_to_the_known_total_when_the_206_says_star() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 5-8/*")]).await;
        let short = RangedRequestMeta {
            range: RangeSpec::From(5),
            sent_validator: None,
            known_total: Some(10),
        };
        assert!(matches!(
            range_verdict(&r, &short, "u"),
            Err(RdlpError::Network { .. })
        ));
        let exact = RangedRequestMeta {
            range: RangeSpec::From(5),
            sent_validator: None,
            known_total: Some(9),
        };
        assert!(matches!(
            range_verdict(&r, &exact, "u").unwrap(),
            RangeVerdict::Partial { .. }
        ));
    }
    /// A sidecar can record `Content-Length: 0` verbatim; against such a
    /// record any 206 span is wrong, and the check must say so without an
    /// arithmetic underflow on `total - 1`.
    #[tokio::test]
    async fn open_ended_range_against_a_zero_length_record_is_a_wrong_span() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(&mut s, 206, &[("content-range", "bytes 0-0/*")]).await;
        let zero = RangedRequestMeta {
            range: RangeSpec::From(0),
            sent_validator: None,
            known_total: Some(0),
        };
        assert!(matches!(
            range_verdict(&r, &zero, "u"),
            Err(RdlpError::Network { .. })
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
    async fn content_coded_response_is_rejected_when_its_body_would_be_placed() {
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
    /// A 416's body is an error page that is never placed, so its coding is
    /// irrelevant; the verdict must still carry the `*/L` length rather than
    /// turning a gzipped error page into a `Download` error.
    #[tokio::test]
    async fn content_coded_416_is_still_unsatisfiable() {
        let mut s = Server::new_async().await;
        let (_m, r) = served(
            &mut s,
            416,
            &[
                ("content-range", "bytes */1234"),
                ("content-encoding", "gzip"),
            ],
        )
        .await;
        assert_eq!(
            range_verdict(&r, &span_meta(0, 0, None), "u").unwrap(),
            RangeVerdict::Unsatisfiable {
                complete_length: Some(1234)
            }
        );
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
    fn coding_headers(values: &[&[u8]]) -> wreq::header::HeaderMap {
        let mut h = wreq::header::HeaderMap::new();
        for v in values {
            h.append(
                "content-encoding",
                wreq::header::HeaderValue::from_bytes(v).unwrap(),
            );
        }
        h
    }
    /// A second `Content-Encoding` line after an `identity` one is a coding
    /// the first-line-only check used to wave through.
    #[test]
    fn content_coding_rejects_a_second_line_after_identity() {
        let err = reject_content_coding(&coding_headers(&[b"identity", b"gzip"]), "u").unwrap_err();
        assert!(err.to_string().contains("content-coded"), "{err}");
    }
    /// A value `to_str` cannot read is not evidence of the identity form.
    #[test]
    fn content_coding_rejects_a_non_utf8_value() {
        let err = reject_content_coding(&coding_headers(&[b"\xff"]), "u").unwrap_err();
        assert!(err.to_string().contains("content-coded"), "{err}");
    }
    #[test]
    fn content_coding_accepts_identity_in_any_case_and_absence() {
        assert!(reject_content_coding(&coding_headers(&[]), "u").is_ok());
        assert!(reject_content_coding(&coding_headers(&[b" Identity "]), "u").is_ok());
        assert!(reject_content_coding(&coding_headers(&[b"identity", b"IDENTITY"]), "u").is_ok());
    }
}
