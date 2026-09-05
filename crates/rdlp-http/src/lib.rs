// Lint-tightening for LIBRARY code only. `pedantic` / `nursery` are
// stylistic; `indexing_slicing` is enforced here because production code must
// not panic on out-of-bounds. Integration tests under `tests/` deliberately
// use `vec[0]` after a length assertion as the assertion form — see
// `Cargo.toml` `[lints.clippy]` for the rationale.
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]
//! HTTP client factory and configuration for rdlp
//!
//! This crate provides a centralized HTTP client factory to eliminate
//! duplication across the codebase. It offers:
//!
//! - `HttpClientConfig`: Configuration for HTTP client behavior
//! - `HttpClientFactory`: Builder for creating configured wreq clients
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

#![warn(missing_docs)]

mod client;
mod config;
pub mod probe;
mod redirect;

pub use client::HttpClientFactory;
pub use config::HttpClientConfig;
pub use probe::{DEFAULT_PROBE_WINDOW_BYTES, ProbeError, ProbeResult, probe_size};

/// Re-export `wreq` for downstream crates so they can consume the HTTP
/// client library via a single facade (`rdlp_http::wreq::Client`, etc).
pub use wreq;

/// Default user agent string for HTTP requests
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
