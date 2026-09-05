//! HTTP client factory

use crate::config::HttpClientConfig;
use std::sync::Arc;
use std::time::Duration;

/// Factory for creating configured HTTP clients
///
/// This factory provides a consistent way to create wreq clients
/// with optimized settings for download operations. Every client built
/// here carries the emulation profile selected by
/// [`HttpClientConfig::emulation`], so TLS handshake shape, HTTP/2
/// SETTINGS, and User-Agent match a real browser.
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_http::HttpClientFactory;
///
/// // Create with defaults (Chrome-latest emulation)
/// let client = HttpClientFactory::default().build();
///
/// // Create with custom config
/// let config = rdlp_http::HttpClientConfig::default()
///     .with_connect_timeout_secs(15);
/// let client = HttpClientFactory::from_config(&config).build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct HttpClientFactory {
    config: HttpClientConfig,
}

/// A redirect was refused because its target failed the SSRF gate.
///
/// Names the hop and the target it refused, so an operator can tell which
/// link in a CDN chain broke rather than only that one did. `target` is
/// redacted at construction — a `Location` may carry userinfo, and this
/// string reaches logs through `wreq::Error`'s Display.
///
/// The refused response's body is deliberately absent: it is never read.
#[derive(Debug)]
struct RedirectRefused {
    hop: usize,
    target: String,
    reason: String,
}

impl std::fmt::Display for RedirectRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "redirect hop {} to {} refused by SSRF validation: {}",
            self.hop, self.target, self.reason
        )
    }
}

impl std::error::Error for RedirectRefused {}

/// A redirect policy that re-validates every hop before following it.
///
/// `validate_url_security` inspects a URL as given, so a caller that checks a
/// public seed learns nothing about where a 302 sends the client. Validating
/// inside the policy is the only place that sees each hop — the call site
/// cannot, because the hops happen within a single `send()` (#662).
///
/// `validate` is a parameter rather than a direct call so tests can drive the
/// following, hop-limit and Location-resolution behaviour against a server on
/// loopback, which the real validator refuses by design. Production has one
/// caller and it passes `rdlp_security::validate_url_security`. A blanket
/// `#[cfg(test)]` loopback exemption — the pattern used at the two call-site
/// gates in `rdlp-api` and `rdlp-extractor` — would be wrong here: it would
/// disable this guard throughout this crate's own tests, including the test
/// that proves the guard bites.
fn ssrf_guarded_redirect_policy<V, E>(max_redirects: usize, validate: V) -> wreq::redirect::Policy
where
    V: Fn(&str) -> Result<(), E> + Send + Sync + 'static,
    E: std::fmt::Display,
{
    // `Policy::custom` does NOT apply a hop limit — wreq documents this
    // explicitly — so the limit is delegated to a `limited` policy rather
    // than counted again here.
    let hop_limit = wreq::redirect::Policy::limited(max_redirects);

    wreq::redirect::Policy::custom(move |attempt| {
        // `Attempt` exposes `uri`/`previous` as FIELDS. wreq's own doc
        // examples call `attempt.uri()`, which does not exist on this
        // version and does not compile.
        let target = attempt.uri.to_string();
        if let Err(e) = validate(&target) {
            let hop = attempt.previous.len();
            // `previous` holds the initial URI plus each followed hop, and
            // the initial one is not a redirect — the same adjustment
            // `Policy::limited` makes when counting.
            return attempt.error(RedirectRefused {
                hop,
                target: rdlp_security::sanitize_for_logging(&target),
                reason: rdlp_security::sanitize_for_logging(&e.to_string()),
            });
        }
        hop_limit.redirect(attempt)
    })
}

impl HttpClientFactory {
    /// Create a new factory with default configuration
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a factory from an [`HttpClientConfig`]
    #[must_use]
    pub fn from_config(config: &HttpClientConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Create a factory from rdlp-types Config
    #[must_use]
    pub fn from_rdlp_config(config: &rdlp_types::Config) -> Self {
        Self {
            config: HttpClientConfig::from_rdlp_config(config),
        }
    }

    /// Build a `wreq::Client` with the configured settings
    #[must_use]
    pub fn build(&self) -> wreq::Client {
        self.build_inner(None)
    }

    /// Build a `wreq::Client` with a cookie provider.
    ///
    /// Cookies in the jar are automatically sent with every request and
    /// `Set-Cookie` response headers are stored back into the jar.
    #[must_use]
    pub fn build_with_cookies(&self, jar: Arc<wreq::cookie::Jar>) -> wreq::Client {
        self.build_inner(Some(jar))
    }

    fn build_inner(&self, cookie_jar: Option<Arc<wreq::cookie::Jar>>) -> wreq::Client {
        self.build_with_redirect(
            cookie_jar,
            ssrf_guarded_redirect_policy(
                self.config.max_redirects,
                rdlp_security::validate_url_security,
            ),
        )
    }

    /// Shared construction path. The redirect policy is a parameter so tests
    /// can exercise following, the hop limit and Location resolution against a
    /// loopback server, which the production policy refuses by design.
    fn build_with_redirect(
        &self,
        cookie_jar: Option<Arc<wreq::cookie::Jar>>,
        redirect: wreq::redirect::Policy,
    ) -> wreq::Client {
        // NOTE: Do NOT call `.user_agent(...)` here — the emulation profile
        // owns User-Agent. Overriding it desyncs the JA4H fingerprint.
        // (spec §6.4)
        //
        // wreq disables redirect-following by default (unlike reqwest). Many
        // CDN-backed extractors (e.g. ABXXX `get_file` → signed CDN) depend
        // on 302 redirects, so a following policy is installed — the guarded
        // one, because the URL a caller validated is not the URL that gets
        // fetched once a hop intervenes (#662).
        let mut builder = wreq::Client::builder()
            .emulation(self.config.emulation.resolve())
            .redirect(redirect)
            .pool_max_idle_per_host(self.config.pool_max_idle_per_host)
            .pool_idle_timeout(self.config.pool_idle_timeout_secs.map(Duration::from_secs))
            .tcp_keepalive(Duration::from_secs(self.config.tcp_keepalive_secs))
            .tcp_nodelay(self.config.tcp_nodelay)
            .connect_timeout(Duration::from_secs(self.config.connect_timeout_secs))
            .read_timeout(Duration::from_secs(self.config.read_timeout_secs));

        if let Some(jar) = cookie_jar {
            builder = builder.cookie_provider(jar);
        }

        if let Some(proxy_url) = &self.config.proxy {
            match rdlp_security::validate_proxy_url(proxy_url) {
                Ok(()) => match wreq::Proxy::all(proxy_url) {
                    Ok(proxy) => builder = builder.proxy(proxy),
                    Err(e) => {
                        // Validator accepted but wreq couldn't parse the URL —
                        // log at ERROR (not WARN) since users who pass --proxy
                        // expecting anonymity must SEE this. The factory's
                        // public contract assumes a proxy in config is valid;
                        // CLI now hard-fails proxy at config-build time, so
                        // reaching this branch indicates a library consumer
                        // bypassing that gate.
                        tracing::error!(
                            "proxy URL accepted by security validator but rejected \
                             by wreq; the resulting client will NOT route through \
                             a proxy: {e}"
                        );
                    }
                },
                Err(e) => tracing::error!(
                    "rejecting proxy URL (failed security validation): {e}; \
                     the resulting client will NOT route through a proxy. \
                     CLI users: this should have been caught at config-build time."
                ),
            }
        }

        // Silent fallback to `wreq::Client::new()` would strip emulation, cookies,
        // proxy and timeouts — a JA4 leak and an authentication failure waiting to
        // happen. A misconfigured client at startup is fatal.
        #[allow(clippy::expect_used)]
        // startup-fatal: TLS/runtime misconfiguration must panic immediately
        builder
            .build()
            .expect("HttpClientFactory: wreq client build failed (TLS/runtime config invalid)")
    }

    /// Build and wrap in Arc for sharing across async tasks
    #[must_use]
    pub fn build_arc(&self) -> Arc<wreq::Client> {
        Arc::new(self.build())
    }

    /// Build with cookie provider and wrap in Arc for sharing across async tasks
    #[must_use]
    pub fn build_arc_with_cookies(&self, jar: Arc<wreq::cookie::Jar>) -> Arc<wreq::Client> {
        Arc::new(self.build_with_cookies(jar))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_factory() {
        let factory = HttpClientFactory::default();
        let _client = factory.build();
        // Client builds successfully
    }

    #[test]
    fn test_from_config() {
        let config = HttpClientConfig::new().with_connect_timeout_secs(15);

        let factory = HttpClientFactory::from_config(&config);
        let _client = factory.build();
        // Client builds successfully with custom config
    }

    #[test]
    fn test_build_arc() {
        let factory = HttpClientFactory::default();
        let client = factory.build_arc();
        assert_eq!(Arc::strong_count(&client), 1);
    }

    /// Regression guard for I-1: a proxy URL pointing at a private/internal
    /// host (SSRF risk) MUST NOT be installed on the resulting client. Prior
    /// behavior swallowed the URL via `wreq::Proxy::all` even though
    /// `rdlp_security::validate_proxy_url` would have rejected it.
    #[test]
    fn private_host_proxy_is_rejected_silently_without_panicking() {
        let config = HttpClientConfig::new().with_proxy("http://127.0.0.1:8080");
        let factory = HttpClientFactory::from_config(&config);
        // Build must succeed (validation failure → warn + skip, not panic).
        let _client = factory.build();
    }

    #[test]
    fn invalid_scheme_proxy_is_rejected_silently_without_panicking() {
        let config = HttpClientConfig::new().with_proxy("ftp://proxy.example.com");
        let factory = HttpClientFactory::from_config(&config);
        let _client = factory.build();
    }

    /// RED for #662: a public seed that redirects to a private host must be
    /// refused at the hop, not followed.
    ///
    /// The private target is a listener we control that answers 200, rather
    /// than a real link-local address. A redirect to 169.254.169.254 would
    /// also produce `Err` today — by connect timeout — so the test would pass
    /// against the unpatched client for the wrong reason.
    #[tokio::test(flavor = "current_thread")]
    async fn redirect_to_a_private_host_is_refused() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let private = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let private_addr = private.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = private.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecret")
                    .await;
                let _ = sock.shutdown().await;
            }
        });

        let seed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let seed_addr = seed.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = seed.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let redirect = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{private_addr}/latest/meta-data/\r\nContent-Length: 0\r\n\r\n"
                );
                let _ = sock.write_all(redirect.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let client = HttpClientFactory::default().build();
        let result = client
            .get(format!("http://{seed_addr}/m.m3u8"))
            .send()
            .await;

        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.contains("secret"),
                    "the refusal must not echo the private response: {msg}"
                );
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                panic!("redirect to a private host was FOLLOWED: status {status}, body {body:?}");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn factory_built_client_follows_302_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");

        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf);
                let response = if req.contains("GET /redir") {
                    "HTTP/1.1 302 Found\r\nLocation: /dest\r\nContent-Length: 0\r\n\r\n".to_string()
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nfollowed!".to_string()
                };
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        // Loopback is refused by the production validator by design, so the
        // following behaviour is driven through an injected permissive one.
        // What this proves is that the guarded policy still FOLLOWS.
        let client = HttpClientFactory::default()
            .build_with_redirect(None, ssrf_guarded_redirect_policy(10, ALLOW_ANYTHING));
        let resp = client
            .get(format!("{base}/redir"))
            .send()
            .await
            .expect("request must succeed");
        assert_eq!(resp.status().as_u16(), 200, "redirect was not followed");
        let body = resp.text().await.unwrap();
        assert_eq!(body, "followed!");
    }

    /// The refusal names the refused target, redacted.
    ///
    /// Both halves matter and each is asserted: the hop's URL must be there,
    /// because "which hop failed" is the whole diagnostic value, and the
    /// userinfo must not, because the string reaches logs through
    /// `wreq::Error`'s Display.
    ///
    /// An earlier version asserted only the absence and was a tautology —
    /// `SecurityError::PrivateHost` carries the host alone, so no credential
    /// was ever in the message and removing the redaction left the test
    /// green. Including the target is what gives the redaction something to
    /// do.
    #[tokio::test(flavor = "current_thread")]
    async fn a_refused_redirect_does_not_leak_userinfo() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let seed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = seed.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = seed.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://admin:hunter2@169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\n\r\n",
                ).await;
                let _ = sock.shutdown().await;
            }
        });

        let err = HttpClientFactory::default()
            .build()
            .get(format!("http://{addr}/seed"))
            .send()
            .await
            .expect_err("a redirect to a link-local host must be refused");

        // Two renderings, deliberately: `Display` is what an operator reads
        // in a log, and the derived `Debug` prints every field whatever
        // `Display` does — so asserting presence against Debug would hold even
        // if the target never reached the message.
        let displayed = err.to_string();
        let rendered = format!("{err} {err:?}");

        // Asserted on the PATH, not the host: the validator's own error text
        // already names the host, so a host assertion passes even with the
        // target dropped. The path exists only in the target.
        assert!(
            displayed.contains("/latest/meta-data/"),
            "the refusal must name the target it refused: {displayed}"
        );
        assert!(
            !rendered.contains("hunter2"),
            "the refusal leaked the password: {rendered}"
        );
        assert!(
            !rendered.contains("admin"),
            "the refusal leaked the username: {rendered}"
        );
    }

    /// A validator that accepts everything, for tests about following rather
    /// than about refusing. A `const` fn pointer rather than a `fn` item so
    /// its always-`Ok` return is not a `clippy::unnecessary_wraps` finding —
    /// the `Result` is required by `ssrf_guarded_redirect_policy`'s bound,
    /// not by this body.
    const ALLOW_ANYTHING: fn(&str) -> Result<(), std::convert::Infallible> = |_| Ok(());

    /// What the policy is handed for a RELATIVE `Location`.
    ///
    /// This is load-bearing and was not obvious from the source: the policy
    /// validates a string, and `validate_url_security` rejects anything it
    /// cannot parse as an absolute URL. If wreq passed `/dest` through
    /// unresolved, every relative redirect on the internet would be refused
    /// as malformed. It does not — it resolves against the previous hop —
    /// and this test is what says so.
    #[tokio::test(flavor = "current_thread")]
    async fn a_relative_location_reaches_the_policy_already_resolved() {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let req = String::from_utf8_lossy(&buf).to_string();
                let response = if req.contains("GET /redir") {
                    "HTTP/1.1 302 Found\r\nLocation: /dest\r\nContent-Length: 0\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
                };
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = {
            let seen = Arc::clone(&seen);
            move |url: &str| -> Result<(), std::convert::Infallible> {
                seen.lock().unwrap().push(url.to_string());
                Ok(())
            }
        };

        let client = HttpClientFactory::default()
            .build_with_redirect(None, ssrf_guarded_redirect_policy(10, recorder));
        let _ = client.get(format!("http://{addr}/redir")).send().await;

        // Cloned out so the guard drops before the assertions — a held
        // `MutexGuard` across a panicking assert is `significant_drop_tightening`.
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "policy should see exactly one hop: {seen:?}");
        assert_eq!(
            seen.first().map(String::as_str),
            Some(format!("http://{addr}/dest").as_str()),
            "a relative Location must reach the policy resolved against the \
             previous hop, or the validator would reject it as unparseable"
        );
    }

    /// `Policy::custom` does not apply a hop limit — wreq documents that the
    /// caller must handle it — so the guarded policy delegates to `limited`.
    ///
    /// The assertion is on the number of requests the server sees, not merely
    /// on getting an `Err`. An earlier version asserted only `expect_err`, and
    /// it passed with the cap removed: the endless chain ran until the 60s
    /// read timeout and returned an error that satisfied the assertion. A test
    /// for a bound has to count.
    #[tokio::test(flavor = "current_thread")]
    async fn the_hop_limit_still_applies_under_the_guarded_policy() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        const MAX_HOPS: usize = 3;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = Arc::clone(&hits);
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                server_hits.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n",
                    )
                    .await;
                let _ = sock.shutdown().await;
            }
        });

        let client = HttpClientFactory::default()
            .build_with_redirect(None, ssrf_guarded_redirect_policy(MAX_HOPS, ALLOW_ANYTHING));
        let err = client
            .get(format!("http://{addr}/loop"))
            .send()
            .await
            .expect_err("an endless redirect loop must fail, not hang");

        assert!(
            err.is_redirect(),
            "must fail on the redirect cap, not a timeout or the SSRF gate: {err}"
        );
        // The initial request plus MAX_HOPS follows; the next hop is refused.
        assert_eq!(
            hits.load(Ordering::SeqCst),
            MAX_HOPS + 1,
            "the chain must stop at the cap rather than run to a timeout"
        );
    }
}
