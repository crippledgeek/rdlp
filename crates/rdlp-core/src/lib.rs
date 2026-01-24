//! # rdlp-core
//!
//! Core types, traits, and utilities for rdlp (Rust Download Program).
//!
//! This crate provides the foundational building blocks for the rdlp video downloader:
//! - **Traits**: `InfoExtractor`, `Downloader`, `PostProcessor`, `JsEngine`, `CookieJar`
//! - **Data Structures**: `InfoDict`, `Format`, `Config` (re-exported from `rdlp-types`)
//! - **Error Types**: `RdlpError` and `Result`
//!
//! ## Example
//!
//! ```rust,no_run
//! use rdlp_core::{InfoDict, Format, Config};
//!
//! // Create a new InfoDict
//! let info = InfoDict::new(
//!     "video123".to_string(),
//!     "My Video".to_string(),
//!     "YouTube".to_string(),
//!     "https://youtube.com/watch?v=video123".to_string(),
//! );
//!
//! // Create a default configuration
//! let config = Config::default();
//! ```

#![warn(missing_docs)]

/// Configuration file I/O utilities
pub mod config_io;
/// Error types and result aliases
pub mod error;
/// Retry logic with exponential backoff
pub mod retry;
/// Core traits for extractors, downloaders, and post-processors
pub mod traits;

// Re-export types from rdlp-types for convenience
pub use rdlp_types::{
    Chapter, Config, Format, FormatSelector, Fragment, InfoDict, Subtitle, Thumbnail,
};

// Re-export error types and utilities
pub use error::{check_http_response, Result, RdlpError};

// Re-export retry utilities
pub use retry::{is_retryable_error, retry_with_backoff, RetryConfig};

// Re-export traits
pub use traits::{
    CookieJar, DownloadProgress, DownloadStats, Downloader, ExtractionContext, InfoExtractor,
    JsEngine, PostProcessConfig, PostProcessResult, PostProcessor, ProgressCallback,
};
