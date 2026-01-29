//! HTTP client configuration

use crate::DEFAULT_USER_AGENT;

/// Configuration for HTTP client behavior
///
/// This struct contains all configurable options for the HTTP client.
/// Use the builder methods to customize behavior.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// User agent string for requests
    pub user_agent: String,

    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,

    /// Read timeout in seconds (idle timeout, not total)
    pub read_timeout_secs: u64,

    /// Maximum idle connections per host
    pub pool_max_idle_per_host: usize,

    /// Idle connection timeout in seconds
    pub pool_idle_timeout_secs: u64,

    /// TCP keepalive interval in seconds
    pub tcp_keepalive_secs: u64,

    /// Disable Nagle's algorithm for lower latency
    pub tcp_nodelay: bool,

    /// Optional HTTP/HTTPS proxy URL
    pub proxy: Option<String>,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            connect_timeout_secs: 30,
            read_timeout_secs: 60,
            pool_max_idle_per_host: 10,
            pool_idle_timeout_secs: 90,
            tcp_keepalive_secs: 60,
            tcp_nodelay: true,
            proxy: None,
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

        if let Some(ref user_agent) = config.user_agent {
            http_config.user_agent = user_agent.clone();
        }

        if let Some(timeout) = config.socket_timeout {
            http_config.connect_timeout_secs = timeout;
        }

        if let Some(ref proxy) = config.proxy {
            http_config.proxy = Some(proxy.clone());
        }

        http_config
    }

    /// Set the user agent string
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Set the connection timeout in seconds
    #[must_use]
    pub fn with_connect_timeout_secs(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    /// Set the read timeout in seconds
    #[must_use]
    pub fn with_read_timeout_secs(mut self, secs: u64) -> Self {
        self.read_timeout_secs = secs;
        self
    }

    /// Set the maximum idle connections per host
    #[must_use]
    pub fn with_pool_max_idle_per_host(mut self, count: usize) -> Self {
        self.pool_max_idle_per_host = count;
        self
    }

    /// Set the idle connection timeout in seconds
    #[must_use]
    pub fn with_pool_idle_timeout_secs(mut self, secs: u64) -> Self {
        self.pool_idle_timeout_secs = secs;
        self
    }

    /// Set the TCP keepalive interval in seconds
    #[must_use]
    pub fn with_tcp_keepalive_secs(mut self, secs: u64) -> Self {
        self.tcp_keepalive_secs = secs;
        self
    }

    /// Enable or disable TCP_NODELAY
    #[must_use]
    pub fn with_tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Set a proxy URL
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
}
