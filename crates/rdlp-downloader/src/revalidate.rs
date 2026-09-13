//! Whether a resume sidecar's anchor validator still names the current
//! representation — decided once here for the HLS and DASH paths (#746).
//!
//! The mechanism is #565's: a one-byte `GET Range` with `If-Range`
//! (RFC 9110 §13.1.5), sent through `HttpDownloader::probe_answered_at` so
//! it rides the shared retry seam and the caller's same-origin header gate.
//! A 206 is judged with `StrongValidator::verify_partial` (§15.3.7: a 206
//! must repeat `ETag`); a 200 — `If-Range` false, or a server that ignores
//! `Range` — with `StrongValidator::verify_full`, which compares only the
//! kind that was sent and treats an absent header as a mismatch.
//! "No answer" (transport failure or a non-answer status after retries) is
//! an error, never `Changed`: discarding a partial on an outage is the
//! failure #565 fixed for HTTP, and it is not repeated here.

use std::sync::atomic::AtomicU64;

use rdlp_core::Result;
use rdlp_http::{ContentRange, StrongValidator, ValidatorMismatch};
use wreq::header::HeaderMap;

use crate::http::{HttpDownloader, ProbeTarget};

/// A revalidation reads headers only; one byte is the smallest body a
/// `Range` request can ask for (`probe_request` clamps to >= 1 anyway).
pub(crate) const ANCHOR_PROBE_WINDOW_BYTES: u64 = 1;

pub(crate) struct AnchorProbe<'a> {
    /// The CURRENT list's URL for the anchor (fresh CDN token), not the one
    /// the sidecar was written under.
    pub url: &'a str,
    /// Already gated by the caller's origin rule (#273).
    pub headers: HeaderMap,
    pub validator: &'a StrongValidator,
}

/// Why [`AnchorVerdict::Changed`] fired — typed rather than a formatted
/// string so a caller can match on it, and so the wording for a validator
/// mismatch is the one `ValidatorMismatch` already owns (`Display`
/// delegates), not a second copy of it.
pub(crate) enum ChangedReason {
    /// The current response's validator does not match the recorded anchor:
    /// a 206 that failed [`StrongValidator::verify_partial`], or a 200 that
    /// failed [`StrongValidator::verify_full`] (a 200 offering no header of
    /// the anchor's kind is [`ValidatorMismatch::Missing`]).
    Validator(ValidatorMismatch),
    /// The probe came back 416 (`Content-Range: bytes */N`, RFC 9110
    /// §14.4's `unsatisfied-range`): the representation the anchor named no
    /// longer has a byte 0, so it is gone or replaced.
    RangeUnsatisfiable,
}

impl std::fmt::Display for ChangedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validator(mismatch) => write!(f, "{mismatch}"),
            Self::RangeUnsatisfiable => {
                f.write_str("current response reports the range as unsatisfiable (416)")
            }
        }
    }
}

pub(crate) enum AnchorVerdict {
    Confirmed,
    /// Why the anchor no longer matches — for the caller's warn line.
    Changed(ChangedReason),
}

pub(crate) async fn revalidate_anchor(
    http: &HttpDownloader,
    probe: AnchorProbe<'_>,
    retries: &AtomicU64,
) -> Result<AnchorVerdict> {
    let result = http
        .probe_answered_at(
            ProbeTarget {
                url: probe.url,
                headers: &probe.headers,
                validator: Some(probe.validator),
                window_bytes: ANCHOR_PROBE_WINDOW_BYTES,
            },
            retries,
        )
        .await?;
    // `complete_length` is `Some` only on a 206 (`ProbeResult::from_response`).
    if result.complete_length.is_some() {
        return Ok(verdict(probe.validator.verify_partial(&result.headers)));
    }
    // A 416's `Content-Range: bytes */N` is otherwise indistinguishable from
    // "no info" in `ProbeResult` (`probe_answered_at`'s doc comment): folding
    // it into a validator mismatch would misreport the reason.
    if ContentRange::unsatisfied_from_headers(&result.headers).is_some() {
        return Ok(AnchorVerdict::Changed(ChangedReason::RangeUnsatisfiable));
    }
    Ok(verdict(probe.validator.verify_full(&result.headers)))
}

fn verdict(checked: std::result::Result<(), ValidatorMismatch>) -> AnchorVerdict {
    match checked {
        Ok(()) => AnchorVerdict::Confirmed,
        Err(mismatch) => AnchorVerdict::Changed(ChangedReason::Validator(mismatch)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn etag(s: &str) -> StrongValidator {
        StrongValidator::try_from(format!("etag:{s}")).expect("strong etag")
    }

    #[tokio::test]
    async fn confirmed_on_206_carrying_the_same_etag() {
        let mut server = mockito::Server::new_async().await;
        let m = server
            .mock("GET", "/a.ts")
            .match_header("if-range", "\"e1\"")
            .match_header("range", "bytes=0-0")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/9")
            .with_header("etag", "\"e1\"")
            .with_body(b"Q")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(matches!(verdict, AnchorVerdict::Confirmed));
        m.assert_async().await;
    }

    #[tokio::test]
    async fn changed_on_200_with_a_different_etag() {
        // If-Range false => the server answers 200 with the whole current
        // representation (§13.1.5); its ETag names the new one.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(200)
            .with_header("etag", "\"e2\"")
            .with_body(b"NEW")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(matches!(verdict, AnchorVerdict::Changed(_)));
    }

    #[tokio::test]
    async fn confirmed_on_200_that_ignores_range_but_repeats_the_same_etag() {
        // A server that does not do ranges still identifies the representation.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(200)
            .with_header("etag", "\"e1\"")
            .with_body(b"SAME")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(matches!(verdict, AnchorVerdict::Confirmed));
    }

    /// The recorded anchor is a `Last-Modified` (the origin offered no strong
    /// `ETag` when the parts were written). A 200 answer that now ALSO carries
    /// a strong `ETag`, alongside the SAME `Last-Modified`, still names the
    /// same representation: only the kind that was sent in `If-Range` is
    /// compared. Comparing whole `StrongValidator`s (whose `from_headers`
    /// prefers `ETag`) misreported this as `Changed`.
    #[tokio::test]
    async fn confirmed_on_200_when_last_modified_anchor_is_repeated_beside_a_new_etag() {
        const LAST_MODIFIED: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(200)
            .with_header("etag", "\"brand-new\"")
            .with_header("last-modified", LAST_MODIFIED)
            .with_header("date", "Sun, 06 Nov 1994 09:00:00 GMT")
            .with_body(b"SAME")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = StrongValidator::try_from(format!("last-modified:{LAST_MODIFIED}"))
            .expect("imf-fixdate");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(
            matches!(verdict, AnchorVerdict::Confirmed),
            "a repeated Last-Modified confirms the anchor regardless of a newly offered ETag"
        );
    }

    /// A 200 that offers no validator of the anchor's kind leaves nothing to
    /// compare: `verify_full` reports it as `Missing` (no §15.3.7 leniency
    /// on a full response), and the anchor is not confirmed.
    #[tokio::test]
    async fn changed_on_200_that_offers_no_validator() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(200)
            .with_body(b"WHO KNOWS")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(matches!(
            verdict,
            AnchorVerdict::Changed(ChangedReason::Validator(ValidatorMismatch::Missing))
        ));
    }

    #[tokio::test]
    async fn changed_when_the_206_carries_no_etag() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(206)
            .with_header("content-range", "bytes 0-0/9")
            .with_body(b"Q")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(
            matches!(verdict, AnchorVerdict::Changed(_)),
            "§15.3.7: a 206 must repeat ETag"
        );
    }

    #[tokio::test]
    async fn changed_on_416_range_unsatisfiable() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(416)
            .with_header("content-range", "bytes */0")
            .with_header("etag", "\"e1\"")
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let verdict = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await
        .expect("answered");
        assert!(
            matches!(
                verdict,
                AnchorVerdict::Changed(ChangedReason::RangeUnsatisfiable)
            ),
            "a 416 must report RangeUnsatisfiable, not a validator mismatch"
        );
    }

    #[tokio::test]
    async fn unanswered_probe_is_an_error_not_a_verdict() {
        // 404 is non-retryable => typed Http error on the first response. The
        // caller must NOT read this as "changed" and discard a partial.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/a.ts")
            .with_status(404)
            .create_async()
            .await;
        let http = HttpDownloader::with_client(wreq::Client::new());
        let url = format!("{}/a.ts", server.url());
        let v = etag("\"e1\"");
        let res = revalidate_anchor(
            &http,
            AnchorProbe {
                url: &url,
                headers: HeaderMap::new(),
                validator: &v,
            },
            &AtomicU64::new(0),
        )
        .await;
        assert!(res.is_err());
    }
}
