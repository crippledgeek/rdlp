//! HTTP client configuration

use crate::DEFAULT_USER_AGENT;
use rdlp_types::BrowserEmulation;

/// Default idle keep-alive socket eviction timeout, in seconds.
///
/// Set intentionally **below** common server keep-alive timeouts (nginx default
/// `keepalive_timeout` is 75s; many CDN edges use 60s) so the pool evicts an
/// idle connection *before* the server silently closes it. Otherwise hyper hands
/// back a dead socket on reuse and returns "connection closed before message
/// completed" (`IncompleteMessage`), forcing an avoidable retry.
///
/// This is the common-case trigger only; it is not the sole defence. hyper-util
/// auto-retries once when a pooled connection is found closed before any bytes
/// are written, and rdlp's `with_retry` / `download_chunk_with_retry` layers
/// cover residual mid-transfer truncation — those MUST stay in place. Operators
/// can override per `Config::pool_idle_timeout` (CLI: `--pool-idle-timeout`).
///
/// Reference: reqwest/Go both default to 90s; lowering below the server's
/// keep-alive is the nginx-documented client-side mitigation. PRD 2026-06-02
/// item 3 / research §F-E.
const DEFAULT_POOL_IDLE_TIMEOUT_SECS: u64 = 60;

/// Maximum redirect hops followed before a request fails.
///
/// Matches `wreq::redirect::Policy::default()`, which is `limited(10)`, so
/// replacing that policy with the SSRF-guarded one leaves chain-length
/// behaviour unchanged.
const DEFAULT_MAX_REDIRECTS: usize = 10;

/// Compile-time regression guard for the stale-connection race: the default
/// MUST stay below nginx's default `keepalive_timeout` (75s) so the pool evicts
/// an idle socket before the server closes it. Bumping the default past a server
/// keep-alive re-opens the "connection closed before message completed" race, so
/// this fails the build rather than shipping the regression.
const _: () = assert!(
    DEFAULT_POOL_IDLE_TIMEOUT_SECS < 75,
    "default pool-idle timeout must stay below nginx's 75s keepalive_timeout"
);

/// Configuration for HTTP client behavior
///
/// This struct contains all configurable options for the HTTP client.
/// Use the builder methods to customize behavior.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// User agent string.
    ///
    /// Retained for backwards compatibility and diagnostic logging.
    /// The wreq client does NOT set User-Agent from this field — the
    /// emulation profile owns it. Setting this has no observable effect
    /// on outgoing requests.
    pub user_agent: String,

    /// Browser emulation profile applied to every request.
    ///
    /// Drives TLS handshake fingerprint, HTTP/2 SETTINGS frame shape,
    /// header order, and User-Agent.
    pub emulation: BrowserEmulation,

    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,

    /// Read timeout in seconds (idle timeout, not total)
    pub read_timeout_secs: u64,

    /// Maximum idle connections per host
    pub pool_max_idle_per_host: usize,

    /// Idle connection timeout in seconds. `None` disables idle eviction
    /// entirely (wired to `wreq::ClientBuilder::pool_idle_timeout(None)`).
    /// Defaults to `DEFAULT_POOL_IDLE_TIMEOUT_SECS` (60s) — see that const for
    /// why it sits below typical server keep-alive timeouts.
    pub pool_idle_timeout_secs: Option<u64>,

    /// TCP keepalive interval in seconds
    pub tcp_keepalive_secs: u64,

    /// Disable Nagle's algorithm for lower latency
    pub tcp_nodelay: bool,

    /// Optional HTTP/HTTPS proxy URL
    pub proxy: Option<String>,

    /// Maximum redirect hops followed before a request fails.
    ///
    /// Bounds chain length only. It is not what stops a redirect reaching a
    /// private host — every hop is re-validated against
    /// `rdlp_security::validate_url_security` (see `client.rs`).
    pub max_redirects: usize,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            emulation: BrowserEmulation::default(),
            connect_timeout_secs: 30,
            read_timeout_secs: 60,
            pool_max_idle_per_host: 10,
            pool_idle_timeout_secs: Some(DEFAULT_POOL_IDLE_TIMEOUT_SECS),
            tcp_keepalive_secs: 60,
            tcp_nodelay: true,
            proxy: None,
            max_redirects: DEFAULT_MAX_REDIRECTS,
        }
    }
}

impl HttpClientConfig {
    /// Create a new config with default values
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create config from rdlp-types Config
    #[must_use]
    pub fn from_rdlp_config(config: &rdlp_types::Config) -> Self {
        let mut http_config = Self::default();

        if let Some(user_agent) = &config.user_agent {
            http_config.user_agent.clone_from(user_agent);
        }

        if let Some(timeout) = config.socket_timeout {
            http_config.connect_timeout_secs = timeout;
        }
        if let Some(timeout) = config.read_timeout {
            http_config.read_timeout_secs = timeout;
        }
        if let Some(timeout) = config.pool_idle_timeout {
            // 0 is the user-facing sentinel for "disable eviction".
            http_config.pool_idle_timeout_secs = if timeout == 0 { None } else { Some(timeout) };
        }

        http_config.emulation = config.browser_emulation.clone();
        http_config.proxy.clone_from(&config.proxy);

        http_config
    }

    /// Set the user agent string.
    ///
    /// NOTE: wreq's emulation profile owns User-Agent — this setter
    /// only updates the diagnostic-use field on the config.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Set the browser emulation profile.
    #[must_use]
    pub fn with_emulation(mut self, emulation: BrowserEmulation) -> Self {
        self.emulation = emulation;
        self
    }

    /// Set the connection timeout in seconds
    #[must_use]
    pub const fn with_connect_timeout_secs(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    /// Set the read timeout in seconds
    #[must_use]
    pub const fn with_read_timeout_secs(mut self, secs: u64) -> Self {
        self.read_timeout_secs = secs;
        self
    }

    /// Set the maximum idle connections per host
    #[must_use]
    pub const fn with_pool_max_idle_per_host(mut self, count: usize) -> Self {
        self.pool_max_idle_per_host = count;
        self
    }

    /// Set the idle connection timeout in seconds.
    #[must_use]
    pub const fn with_pool_idle_timeout_secs(mut self, secs: u64) -> Self {
        self.pool_idle_timeout_secs = Some(secs);
        self
    }

    /// Disable idle connection eviction entirely.
    #[must_use]
    pub const fn with_pool_idle_disabled(mut self) -> Self {
        self.pool_idle_timeout_secs = None;
        self
    }

    /// Set the TCP keepalive interval in seconds
    #[must_use]
    pub const fn with_tcp_keepalive_secs(mut self, secs: u64) -> Self {
        self.tcp_keepalive_secs = secs;
        self
    }

    /// Enable or disable `TCP_NODELAY`
    #[must_use]
    pub const fn with_tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Set a proxy URL
    #[must_use]
    pub fn with_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = HttpClientConfig::default();
        assert_eq!(config.user_agent, DEFAULT_USER_AGENT);
        assert_eq!(config.connect_timeout_secs, 30);
        assert_eq!(config.read_timeout_secs, 60);
        assert!(config.tcp_nodelay);
        assert!(config.proxy.is_none());
        assert!(matches!(config.emulation, BrowserEmulation::ChromeLatest));
    }

    #[test]
    fn test_builder_pattern() {
        let config = HttpClientConfig::new()
            .with_user_agent("Test/1.0")
            .with_connect_timeout_secs(15)
            .with_proxy("http://proxy.example.com:8080");

        assert_eq!(config.user_agent, "Test/1.0");
        assert_eq!(config.connect_timeout_secs, 15);
        assert_eq!(
            config.proxy.as_deref(),
            Some("http://proxy.example.com:8080")
        );
    }

    #[test]
    fn test_with_emulation() {
        let config = HttpClientConfig::new().with_emulation(BrowserEmulation::FirefoxLatest);
        assert!(matches!(config.emulation, BrowserEmulation::FirefoxLatest));
    }

    #[test]
    fn pool_idle_timeout_secs_can_be_disabled() {
        let config = HttpClientConfig::new().with_pool_idle_disabled();
        assert!(config.pool_idle_timeout_secs.is_none());
    }

    #[test]
    fn default_pool_idle_timeout_is_some_60() {
        let config = HttpClientConfig::default();
        assert_eq!(
            config.pool_idle_timeout_secs,
            Some(DEFAULT_POOL_IDLE_TIMEOUT_SECS)
        );
    }

    #[test]
    fn from_rdlp_config_maps_read_timeout() {
        let cfg = rdlp_types::Config {
            read_timeout: Some(120),
            ..rdlp_types::Config::default()
        };
        let http = HttpClientConfig::from_rdlp_config(&cfg);
        assert_eq!(http.read_timeout_secs, 120);
    }

    #[test]
    fn from_rdlp_config_maps_pool_idle_timeout_zero_to_none() {
        let cfg = rdlp_types::Config {
            pool_idle_timeout: Some(0),
            ..rdlp_types::Config::default()
        };
        let http = HttpClientConfig::from_rdlp_config(&cfg);
        assert!(
            http.pool_idle_timeout_secs.is_none(),
            "0 must map to None (disable)"
        );
    }

    #[test]
    fn from_rdlp_config_maps_pool_idle_timeout_positive() {
        let cfg = rdlp_types::Config {
            pool_idle_timeout: Some(60),
            ..rdlp_types::Config::default()
        };
        let http = HttpClientConfig::from_rdlp_config(&cfg);
        assert_eq!(http.pool_idle_timeout_secs, Some(60));
    }

    #[test]
    fn from_rdlp_config_with_no_timeout_overrides_uses_defaults() {
        let cfg = rdlp_types::Config {
            socket_timeout: None,
            read_timeout: None,
            pool_idle_timeout: None,
            ..rdlp_types::Config::default()
        };
        let http = HttpClientConfig::from_rdlp_config(&cfg);
        assert_eq!(http.connect_timeout_secs, 30);
        assert_eq!(http.read_timeout_secs, 60);
        assert_eq!(
            http.pool_idle_timeout_secs,
            Some(DEFAULT_POOL_IDLE_TIMEOUT_SECS)
        );
    }
}
