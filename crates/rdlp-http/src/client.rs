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
        // NOTE: Do NOT call `.user_agent(...)` here — the emulation profile
        // owns User-Agent. Overriding it desyncs the JA4H fingerprint.
        // (spec §6.4)
        //
        // wreq disables redirect-following by default (unlike reqwest). Many
        // CDN-backed extractors (e.g. ABXXX `get_file` → signed CDN) depend
        // on 302 redirects, so install the limited(10) policy as a baseline.
        let mut builder = wreq::Client::builder()
            .emulation(self.config.emulation.resolve())
            .redirect(wreq::redirect::Policy::limited(10))
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

        let client = HttpClientFactory::default().build();
        let resp = client
            .get(format!("{base}/redir"))
            .send()
            .await
            .expect("request must succeed");
        assert_eq!(resp.status().as_u16(), 200, "redirect was not followed");
        let body = resp.text().await.unwrap();
        assert_eq!(body, "followed!");
    }
}
