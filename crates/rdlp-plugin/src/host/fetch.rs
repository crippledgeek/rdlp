//! `host:fetch` capability — HTTPS via rdlp's shared `wreq` client (TLS impersonation preserved).
//!
//! Plugin requests run through:
//!
//! 1. `rdlp_security::validate_url_security` — rejects loopback / private-network targets,
//!    enforces https in production, etc.
//! 2. The shared `wreq::Client` — same TLS fingerprint as the rest of rdlp.
//! 3. `tokio::select!` against the per-call `CancellationToken` so Ctrl+C aborts in-flight requests.
//! 4. Body size cap (32 MB) — prevents adversarial servers from exhausting plugin memory.

use crate::PluginError;
use crate::host::fetch_fixtures::SharedFixtures;
use crate::instance::PluginStoreData;
use rdlp_http::wreq;
use rdlp_security::validate_url_security;
use std::time::Duration;
use wasmtime::component::Linker;

const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 60_000;

/// Per-plugin fetch context. Owns a clone of the shared `wreq::Client`
/// plus an optional fixture-replay map for golden tests.
#[derive(Clone)]
pub struct FetchCtx {
    /// The HTTP client used for all plugin fetch requests.
    pub client: wreq::Client,
    /// Optional URL → canned-response map. When `Some` and a request URL
    /// matches an entry, the canned response is returned WITHOUT making
    /// a real HTTP call. Production callers leave this `None`; it's
    /// populated by the test harness for plugin golden tests against
    /// geo-restricted or otherwise non-CI-friendly endpoints.
    pub fixtures: SharedFixtures,
}

impl FetchCtx {
    /// Build a context with a fresh default `wreq::Client` (impersonation
    /// disabled, plain HTTPS — the loader will replace this with the shared
    /// client at instantiation time).
    pub fn with_default_client() -> Result<Self, PluginError> {
        let client = wreq::Client::builder()
            .build()
            .map_err(|e| PluginError::Internal(format!("wreq client init: {e}")))?;
        Ok(Self {
            client,
            fixtures: None,
        })
    }
}

/// Wire `host:fetch` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_fetch::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_fetch::Host for PluginStoreData {
    async fn fetch(
        &mut self,
        req: crate::bindings::rdlp::plugin::host_fetch::Request,
    ) -> Result<
        crate::bindings::rdlp::plugin::host_fetch::Response,
        crate::bindings::rdlp::plugin::host_fetch::FetchError,
    > {
        use crate::bindings::rdlp::plugin::host_fetch::{FetchError, Response};

        let Some(ctx) = self.fetch.as_ref() else {
            return Err(FetchError::Network("fetch capability not granted".into()));
        };

        // Fixture-replay: in tests, an injected URL → canned-response
        // map intercepts requests BEFORE any network call. Production
        // builds leave `fixtures` as None so this is a single Option
        // check on the hot path. We deliberately bypass
        // `validate_url_security` for fixture matches because tests
        // routinely use private-network or loopback URLs.
        if let Some(fixtures) = ctx.fixtures.as_ref()
            && let Some(canned) = fixtures.get(&req.url)
        {
            return Ok(Response {
                status: canned.status,
                headers: canned.headers.clone(),
                body: canned.body.clone(),
                final_url: req.url.clone(),
            });
        }

        if let Err(e) = validate_url_security(&req.url) {
            return Err(FetchError::Network(format!("url security: {e}")));
        }

        let method = match req.method.parse::<wreq::Method>() {
            Ok(m) => m,
            Err(_) => {
                return Err(FetchError::Network(format!(
                    "invalid HTTP method: {}",
                    req.method
                )));
            }
        };
        let mut rb = ctx.client.request(method, &req.url);
        for (k, v) in &req.headers {
            rb = rb.header(k, v);
        }
        if let Some(body) = req.body {
            rb = rb.body(body);
        }
        if let Some(t) = req.timeout_ms {
            let clamped = (t as u64).min(MAX_TIMEOUT_MS);
            rb = rb.timeout(Duration::from_millis(clamped));
        }

        let cancel = self.cancel.clone();
        let send_fut = rb.send();
        let resp = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(FetchError::Cancelled),
            r = send_fut => r,
        };
        let resp = match resp {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(FetchError::Timeout),
            Err(e) if e.is_redirect() => return Err(FetchError::TooManyRedirects),
            Err(e) => return Err(FetchError::Network(e.to_string())),
        };

        let status = resp.status().as_u16();
        let final_url = resp.uri().to_string();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        // Streaming body cap: abort the moment the cumulative byte count
        // exceeds MAX_BODY_BYTES so an adversarial server cannot exhaust
        // plugin memory before the cap fires (the previous `resp.bytes()`
        // path buffered the entire response BEFORE checking).
        use futures_util::StreamExt;
        let cancel2 = cancel.clone();
        let mut stream = resp.bytes_stream();
        let mut body: Vec<u8> = Vec::new();
        loop {
            tokio::select! {
                biased;
                () = cancel2.cancelled() => return Err(FetchError::Cancelled),
                chunk = stream.next() => match chunk {
                    Some(Ok(bytes)) => {
                        if body.len().saturating_add(bytes.len()) > MAX_BODY_BYTES {
                            return Err(FetchError::BodyTooLarge);
                        }
                        body.extend_from_slice(&bytes);
                    }
                    Some(Err(e)) => return Err(FetchError::Network(e.to_string())),
                    None => break,
                }
            }
        }
        Ok(Response {
            status,
            headers,
            body,
            final_url,
        })
    }
}
