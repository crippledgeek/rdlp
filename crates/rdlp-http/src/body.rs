//! Streaming, size-capped HTTP response body reader.
//!
//! Shared by `rdlp-extractor` (webpage/JSON bodies), `rdlp-downloader`
//! (HLS/DASH fragment and segment bodies), `rdlp-api` (thumbnail and
//! subtitle downloads), and `rdlp-plugin` (the `host:fetch` capability).
//! All four previously buffered a response with `response.bytes().await`
//! (or a private hand-rolled streaming loop) before any size check, letting
//! an adversarial or misbehaving server exhaust host memory with an
//! arbitrarily large body (issue #569). This module is the single streaming
//! implementation every caller converges onto.

use bytes::Bytes;
use futures_util::StreamExt as _;

/// A validated, non-zero ceiling on a single HTTP response body, in bytes.
///
/// Constructed only via [`BodyCap::new`], which rejects `0` — a zero cap
/// would reject every body outright, including an empty one, which is never
/// the intent of a memory-exhaustion guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyCap(u64);

impl BodyCap {
    /// Construct a cap from a byte count. Returns `None` for `0`.
    #[must_use]
    pub const fn new(limit_bytes: u64) -> Option<Self> {
        if limit_bytes == 0 {
            None
        } else {
            Some(Self(limit_bytes))
        }
    }

    /// The cap's byte value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A response body could not be read within its cap, or the transport failed.
#[derive(Debug, thiserror::Error)]
pub enum BodyCapError {
    /// The body exceeded `limit` bytes. `seen_at_least` is a lower bound on
    /// how much arrived before the read was aborted — either the declared
    /// `Content-Length` (caught before any byte was read) or the running
    /// total at the moment the streaming check tripped.
    #[error("response body exceeds {limit}-byte cap (observed at least {seen_at_least} bytes)")]
    Oversized {
        /// The cap that was exceeded.
        limit: u64,
        /// Lower bound on the body size that triggered the rejection.
        seen_at_least: u64,
    },
    /// Underlying wreq/transport error (DNS, TLS, connect, mid-stream reset).
    #[error("failed to read response body: {0}")]
    Transport(#[from] wreq::Error),
}

/// Read `response`'s body into memory, aborting the moment it would exceed
/// `cap`.
///
/// Two checks compose to cover both an honest and a misbehaving server:
/// - **Pre-check**: when the response carries a known length (`Content-Length`
///   framing), a body already declared larger than `cap` is refused before a
///   single byte is read.
/// - **Streaming check**: the body is read via `bytes_stream()` with a
///   running total, so a response with no declared length (`Transfer-Encoding:
///   chunked`, or any framing `wreq` cannot pre-size) is still bounded — the
///   read aborts the instant the running total would exceed `cap`, rather
///   than buffering the whole thing first.
///
/// # Errors
///
/// Returns [`BodyCapError::Oversized`] if the body's declared or actual size
/// exceeds `cap`, or [`BodyCapError::Transport`] if the underlying read
/// fails.
pub async fn read_body_capped(
    response: wreq::Response,
    cap: BodyCap,
) -> Result<Vec<u8>, BodyCapError> {
    let declared = response.content_length();
    if let Some(declared) = declared
        && declared > cap.get()
    {
        return Err(BodyCapError::Oversized {
            limit: cap.get(),
            seen_at_least: declared,
        });
    }

    // Bounded by `cap`, not just `declared`, so a body with no declared
    // length never over-reserves; bounded by `declared` (once past the
    // pre-check above, always <= cap) so a small body doesn't pay for a
    // full `cap`-sized reservation. Without this, `Vec`'s amortized
    // doubling can leave `buf` holding up to ~2x the bytes actually stored.
    //
    // A server that lies with `Content-Length: <cap>` while sending a tiny
    // body still gets a cap-sized reservation up front — `declared` trusts
    // the header, not the eventual byte count. This cost-shift is bounded,
    // not new: it is still inside the documented `concurrent_fragments ×
    // max_fragment_bytes` peak-memory bound (see `Config::concurrent_fragments`),
    // and the reserved-but-never-written pages of an over-large `Vec` stay
    // non-resident under Linux's default memory overcommit — a virtual
    // reservation, not physical RSS, until actually written.
    // `unwrap_or(0)`, not `usize::MAX`: this is a capacity *hint*, not a
    // bound — `Vec::with_capacity(usize::MAX)` panics, and on a 32-bit
    // target a `u64` byte count can exceed `usize::MAX` for a value that is
    // otherwise fine to stream. Falling back to 0 just forgoes the
    // pre-reservation optimization for that (rare, huge) case.
    let capacity_hint = declared.unwrap_or(0).min(cap.get());
    let mut buf: Vec<u8> = Vec::with_capacity(usize::try_from(capacity_hint).unwrap_or(0));
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk: Bytes = chunk?;
        let total = buf.len() as u64 + chunk.len() as u64;
        if total > cap.get() {
            return Err(BodyCapError::Oversized {
                limit: cap.get(),
                seen_at_least: total,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
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

    #[test]
    fn body_cap_rejects_zero() {
        assert!(BodyCap::new(0).is_none());
    }

    #[test]
    fn body_cap_accepts_positive() {
        assert_eq!(BodyCap::new(5).map(BodyCap::get), Some(5));
    }

    /// Oversized body with a truthful `Content-Length` is refused by the
    /// pre-check, before the streaming loop ever runs.
    #[tokio::test]
    async fn oversized_with_truthful_content_length_rejected_before_read() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/big")
            .with_status(200)
            .with_body(vec![0u8; 100])
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/big", server.url());
        let resp = client.get(&url).send().await.expect("request must send");
        let cap = BodyCap::new(10).expect("10 is a valid cap");

        let err = read_body_capped(resp, cap)
            .await
            .expect_err("100-byte body must be rejected by a 10-byte cap");
        match err {
            BodyCapError::Oversized {
                limit,
                seen_at_least,
            } => {
                assert_eq!(limit, 10);
                assert_eq!(
                    seen_at_least, 100,
                    "pre-check must report the declared Content-Length"
                );
            }
            BodyCapError::Transport(e) => panic!("expected Oversized, got Transport({e})"),
        }
        mock.assert_async().await;
    }

    /// A chunked body (no `Content-Length`, so the pre-check cannot fire)
    /// that exceeds the cap must still be caught by the streaming check.
    #[tokio::test]
    async fn oversized_chunked_body_rejected_mid_stream() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/chunked")
            .with_status(200)
            .with_chunked_body(|w| w.write_all(&[0u8; 100]))
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/chunked", server.url());
        let resp = client.get(&url).send().await.expect("request must send");
        assert_eq!(
            resp.content_length(),
            None,
            "chunked framing must not report a known length"
        );
        let cap = BodyCap::new(10).expect("10 is a valid cap");

        let err = read_body_capped(resp, cap)
            .await
            .expect_err("100-byte chunked body must be rejected by a 10-byte cap");
        assert!(
            matches!(err, BodyCapError::Oversized { limit: 10, .. }),
            "expected Oversized{{limit: 10, ..}}, got {err:?}"
        );
        mock.assert_async().await;
    }

    /// Boundary: a body exactly at the cap is accepted.
    #[tokio::test]
    async fn body_exactly_at_cap_is_accepted() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/exact")
            .with_status(200)
            .with_body(vec![7u8; 10])
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/exact", server.url());
        let resp = client.get(&url).send().await.expect("request must send");
        let cap = BodyCap::new(10).expect("10 is a valid cap");

        let body = read_body_capped(resp, cap)
            .await
            .expect("a body exactly at the cap must be accepted");
        assert_eq!(body, vec![7u8; 10]);
        mock.assert_async().await;
    }

    /// Boundary: cap + 1 is rejected.
    #[tokio::test]
    async fn body_one_byte_over_cap_is_rejected() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/over")
            .with_status(200)
            .with_body(vec![7u8; 11])
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/over", server.url());
        let resp = client.get(&url).send().await.expect("request must send");
        let cap = BodyCap::new(10).expect("10 is a valid cap");

        let err = read_body_capped(resp, cap)
            .await
            .expect_err("cap + 1 bytes must be rejected");
        assert!(matches!(err, BodyCapError::Oversized { limit: 10, .. }));
        mock.assert_async().await;
    }

    /// A normal body well under the cap is read through unaffected.
    #[tokio::test]
    async fn normal_body_under_cap_is_unaffected() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/small")
            .with_status(200)
            .with_body("hello")
            .create_async()
            .await;

        let client = make_client();
        let url = format!("{}/small", server.url());
        let resp = client.get(&url).send().await.expect("request must send");
        let cap = BodyCap::new(1024).expect("1024 is a valid cap");

        let body = read_body_capped(resp, cap)
            .await
            .expect("a small body must pass through unaffected");
        assert_eq!(body, b"hello");
        mock.assert_async().await;
    }

    /// A minimal raw HTTP/1.1 server that declares a `Content-Length`
    /// independent of the actual body it sends. mockito cannot produce this:
    /// `respond_with_mock` appends its own `Content-Length` header, sized to
    /// the real body, whenever the INCOMING REQUEST lacks one (mockito 1.7.2
    /// `server.rs:587`, `request.has_header("content-length")` — checked on
    /// the client's request, not the mock's response) — which is always true
    /// for a plain GET. A `.with_header("Content-Length", "N")` mock would
    /// therefore end up with the user-set header AND mockito's appended one
    /// on the wire, a hyper-framing coin-flip rather than the deterministic
    /// mismatch this test needs, so a declared/actual mismatch requires
    /// scripting the bytes on the wire directly.
    fn spawn_scripted_server(declared_content_length: u64, actual_body_len: usize) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("listener has a local addr");
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            // Drain (not parse) the request; a bare GET fits comfortably.
            let mut request_buf = [0u8; 4096];
            let _ = stream.read(&mut request_buf);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {declared_content_length}\r\n\
                     Connection: close\r\n\r\n"
                )
                .as_bytes(),
            );
            let _ = stream.write_all(&vec![0u8; actual_body_len]);
            let _ = stream.shutdown(std::net::Shutdown::Write);
        });
        format!("http://{addr}/")
    }

    /// Acceptance test for "oversized with an understated Content-Length":
    /// the server declares 60 bytes while actually sending 100, and 60 is
    /// itself already over the 50-byte cap. The declared value is what the
    /// pre-check reads, so it fires — `Oversized`, not `Transport` — without
    /// this layer ever needing to see the real (further understated) 100.
    #[tokio::test]
    async fn understated_content_length_still_over_cap_is_rejected_by_precheck() {
        let url = spawn_scripted_server(60, 100);
        let client = make_client();
        let resp = client.get(&url).send().await.expect("request must send");
        let cap = BodyCap::new(50).expect("50 is a valid cap");

        let err = read_body_capped(resp, cap).await.expect_err(
            "a declared Content-Length of 60 must be rejected by a 50-byte cap, \
             even though the real body (100) is understated further",
        );
        assert!(
            matches!(
                err,
                BodyCapError::Oversized {
                    limit: 50,
                    seen_at_least: 60
                }
            ),
            "expected Oversized{{limit: 50, seen_at_least: 60}}, got {err:?}"
        );
    }

    /// Documents the other side of "understated": a declared Content-Length
    /// (10) UNDER the cap (50), while the server actually sends 100 bytes.
    /// HTTP/1.1 framing is strict — hyper treats Content-Length as the
    /// authoritative end of the message and stops reading at the declared
    /// count, so `read_body_capped` never observes the remaining 90 bytes.
    /// The result is `Ok` with exactly the declared 10 bytes, not an
    /// `Oversized` error and not a 100-byte body. This is the honest
    /// empirical answer, not the acceptance test above: a Content-Length
    /// smaller than the cap cannot be used to smuggle a larger body past
    /// this reader, because the transport layer never delivers the extra
    /// bytes in the first place — the streaming check exists for framings
    /// with no declared length at all (chunked / connection-close), not for
    /// this case.
    #[tokio::test]
    async fn understated_content_length_under_cap_is_truncated_by_http_framing() {
        let url = spawn_scripted_server(10, 100);
        let client = make_client();
        let resp = client.get(&url).send().await.expect("request must send");
        let cap = BodyCap::new(50).expect("50 is a valid cap");

        let body = read_body_capped(resp, cap)
            .await
            .expect("HTTP/1.1 framing must truncate to the declared 10 bytes, well under the cap");
        assert_eq!(
            body.len(),
            10,
            "the reader must see exactly the declared length, not the 100 bytes sent"
        );
    }
}
