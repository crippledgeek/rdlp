//! Whether a resume sidecar's anchor validator still names the current
//! representation — decided once here for the HLS and DASH paths (#746).
//!
//! The mechanism is #565's: a one-byte `GET Range` with `If-Range`
//! (RFC 9110 §13.1.5), sent through `HttpDownloader::probe_answered_at` so
//! it rides the shared retry seam and the caller's same-origin header gate.
//! A 206 is judged with `StrongValidator::verify_partial` (§15.3.7: a 206
//! must repeat `ETag`); a 200 — `If-Range` false, or a server that ignores
//! `Range` — is confirmed only if it repeats the same strong validator.
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
/// mismatch is the one `StrongValidator::verify_partial` already owns
/// (`Display` delegates), not a second copy of it.
pub(crate) enum ChangedReason {
    /// The current response's validator does not match the recorded anchor
    /// (a 206 that failed [`StrongValidator::verify_partial`], or a 200
    /// whose validator differs — built as [`ValidatorMismatch::Different`]
    /// the same way `verify_partial` builds it).
    Validator(ValidatorMismatch),
    /// A 200 (or a server that ignored `Range`) offered no strong validator
    /// at all — nothing to compare the anchor against.
    NoValidatorOnResponse,
    /// The probe came back 416 (`Content-Range: bytes */N`, RFC 9110
    /// §14.4's `unsatisfied-range`): the representation the anchor named no
    /// longer has a byte 0, so it is gone or replaced.
    RangeUnsatisfiable,
}

impl std::fmt::Display for ChangedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validator(mismatch) => write!(f, "{mismatch}"),
            Self::NoValidatorOnResponse => {
                f.write_str("current response offers no strong validator")
            }
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

/// The raw bytes `StrongValidator` was built from — the same bytes
/// `verify_partial` lossy-decodes into a [`ValidatorMismatch::Different`],
/// used here to report a mismatch found outside a 206 in the identical
/// wording.
fn validator_bytes(v: &StrongValidator) -> &[u8] {
    match v {
        StrongValidator::ETag(tag) => tag.as_bytes(),
        StrongValidator::LastModified(date) => date.as_bytes(),
    }
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
        return Ok(match probe.validator.verify_partial(&result.headers) {
            Ok(()) => AnchorVerdict::Confirmed,
            Err(mismatch) => AnchorVerdict::Changed(ChangedReason::Validator(mismatch)),
        });
    }
    // A 416's `Content-Range: bytes */N` is otherwise indistinguishable from
    // "no info" in `ProbeResult` (`probe_answered_at`'s doc comment): folding
    // it into "no strong validator" would misreport the reason.
    if ContentRange::unsatisfied_from_headers(&result.headers).is_some() {
        return Ok(AnchorVerdict::Changed(ChangedReason::RangeUnsatisfiable));
    }
    Ok(match result.validator.as_ref() {
        Some(got) if got == probe.validator => AnchorVerdict::Confirmed,
        Some(got) => {
            AnchorVerdict::Changed(ChangedReason::Validator(ValidatorMismatch::Different {
                expected: String::from_utf8_lossy(validator_bytes(probe.validator)).into_owned(),
                got: String::from_utf8_lossy(validator_bytes(got)).into_owned(),
            }))
        }
        None => AnchorVerdict::Changed(ChangedReason::NoValidatorOnResponse),
    })
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
