//! # rdlp-core
//!
//! Core types, traits, and utilities for rdlp (Rust Download Program).
//!
//! This crate provides the foundational building blocks for the rdlp video downloader:
//! - **Traits**: `InfoExtractor`, `Downloader`, `PostProcessor`, `JsEngine`, `CookieJar`
//! - **Data Structures**: `InfoDict`, `Format`, `Config`
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

pub mod config;
pub mod error;
pub mod format;
pub mod info_dict;
pub mod retry;
pub mod traits;

// Re-export commonly used types
pub use config::Config;
pub use error::{check_http_response, Result, RdlpError};
pub use format::{Format, FormatSelector, Fragment};
pub use info_dict::{Chapter, InfoDict, Subtitle, Thumbnail};
pub use retry::{is_retryable_error, retry_with_backoff, RetryConfig};
pub use traits::{
    CookieJar, Downloader, DownloadProgress, DownloadStats, ExtractionContext, InfoExtractor,
    JsEngine, PostProcessConfig, PostProcessResult, PostProcessor, ProgressCallback,
};
