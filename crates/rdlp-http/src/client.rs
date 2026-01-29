//! HTTP client factory

use crate::config::HttpClientConfig;
use std::sync::Arc;
use std::time::Duration;

/// Factory for creating configured HTTP clients
///
/// This factory provides a consistent way to create reqwest clients
/// with optimized settings for download operations.
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_http::HttpClientFactory;
///
/// // Create with defaults
/// let client = HttpClientFactory::default().build();
///
/// // Create with custom config
/// let config = rdlp_http::HttpClientConfig::default()
///     .with_user_agent("MyApp/1.0");
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

    /// Create a factory from an HttpClientConfig
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

    /// Build a reqwest::Client with the configured settings
    #[must_use]
    pub fn build(&self) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            .user_agent(&self.config.user_agent)
            .pool_max_idle_per_host(self.config.pool_max_idle_per_host)
            .pool_idle_timeout(Duration::from_secs(self.config.pool_idle_timeout_secs))
            .tcp_keepalive(Duration::from_secs(self.config.tcp_keepalive_secs))
            .tcp_nodelay(self.config.tcp_nodelay)
            .connect_timeout(Duration::from_secs(self.config.connect_timeout_secs))
            .read_timeout(Duration::from_secs(self.config.read_timeout_secs));

        if let Some(ref proxy_url) = self.config.proxy {
            if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }

        builder.build().unwrap_or_else(|_| reqwest::Client::new())
    }

    /// Build and wrap in Arc for sharing across async tasks
    #[must_use]
    pub fn build_arc(&self) -> Arc<reqwest::Client> {
        Arc::new(self.build())
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
        let config = HttpClientConfig::new()
            .with_user_agent("Test/1.0")
            .with_connect_timeout_secs(15);

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
}
