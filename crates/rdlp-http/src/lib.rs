//! HTTP client factory and configuration for rdlp
//!
//! This crate provides a centralized HTTP client factory to eliminate
//! duplication across the codebase. It offers:
//!
//! - `HttpClientConfig`: Configuration for HTTP client behavior
//! - `HttpClientFactory`: Builder for creating configured reqwest clients
//!
//! # Example
//!
//! ```rust,no_run
//! use rdlp_http::{HttpClientConfig, HttpClientFactory};
//!
//! // Create with defaults
//! let client = HttpClientFactory::default().build();
//!
//! // Create with custom config
//! let config = HttpClientConfig::default()
//!     .with_user_agent("MyApp/1.0")
//!     .with_connect_timeout_secs(30);
//!
//! let client = HttpClientFactory::from_config(&config).build();
//! ```

mod client;
mod config;

pub use client::HttpClientFactory;
pub use config::HttpClientConfig;

/// Default user agent string for HTTP requests
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
