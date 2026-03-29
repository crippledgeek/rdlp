//! # rdlp-core
//!
//! Core types, traits, and utilities for rdlp (Rust Download Program).
//!
//! This crate provides the foundational building blocks for the rdlp video downloader:
//! - **Traits**: `InfoExtractor`, `Downloader`, `PostProcessCallback`, `JsEngine`, `CookieJar`
//! - **Data Structures**: `InfoDict`, `Format`, `Config` (live in `rdlp-types`; import directly)
//! - **Error Types**: `RdlpError` and `Result`
//!
//! ## Example
//!
//! ```rust,no_run
//! use rdlp_types::{InfoDict, Format, Config};
//!
//! // Create a new InfoDict (constructor accepts impl Into<String>)
//! let info = InfoDict::new(
//!     "video123",
//!     "My Video",
//!     "YouTube",
//!     "https://youtube.com/watch?v=video123",
//! );
//!
//! // Create a default configuration
//! let config = Config::default();
//! ```

#![warn(missing_docs)]

/// HLS codec parsing utilities
pub mod codecs;
/// Configuration file I/O utilities
pub mod config_io;
/// Error types and result aliases
pub mod error;
/// Retry logic with exponential backoff
pub mod retry;
/// Core traits for extractors, downloaders, and post-processors
pub mod traits;

// Re-export codec utilities
pub use codecs::parse_hls_codecs;

// Re-export error types and utilities
pub use error::{RdlpError, Result, check_http_response};

// Re-export retry utilities (backon-based)
pub use retry::{ExponentialBuilder, RetryConfig, Retryable, is_retryable_error};

// Re-export traits
pub use traits::{
    CookieJar, DownloadProgress, DownloadStats, Downloader, ExtractionContext, InfoExtractor,
    JsEngine, PostProcessCallback, PostProcessCallbackFactory, ProgressCallback,
    SearchExtractor,
};
