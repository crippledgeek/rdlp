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
// A `mockito::ServerGuard` in this crate's tests must outlive the mock
// asserts that follow its last request use — the assertion is what proves
// the mock was hit, so holding the guard there is the point. Clippy's
// `significant_drop_tightening` reads that as "drop it earlier," which would
// tear the mock server down before the assertion runs: a false positive for
// every mockito test in this crate. One crate-wide, test-only decision
// instead of a per-module copy (`probe.rs`/`request.rs` each carried an
// identical `#[allow]` before this).
#![cfg_attr(test, allow(clippy::significant_drop_tightening))]

mod client;
mod config;
pub mod probe;
mod redirect;
pub mod request;
pub mod validator;

pub use client::HttpClientFactory;
pub use config::HttpClientConfig;
pub use probe::{DEFAULT_PROBE_WINDOW_BYTES, ProbeError, ProbeResult, ProbeSpec, probe_size};
pub use request::{RangeSpec, RangedRequest, download_request};
pub use validator::{StrongValidator, ValidatorMismatch};

/// Re-export `wreq` for downstream crates so they can consume the HTTP
/// client library via a single facade (`rdlp_http::wreq::Client`, etc).
pub use wreq;

/// Default user agent string for HTTP requests
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
