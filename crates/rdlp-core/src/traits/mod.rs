//! Core traits defining the plugin architecture for rdlp.
//!
//! This module contains the fundamental traits that enable the modular design:
//! - [`InfoExtractor`] - Extract video metadata from URLs
//! - [`Downloader`] - Download content via various protocols
//! - [`PostProcessor`] - Transform downloaded files

/// Download protocol traits and progress tracking
pub mod downloader;
/// Video metadata extraction traits
pub mod extractor;
/// Post-processing pipeline traits
pub mod postprocessor;

pub use downloader::{DownloadProgress, DownloadStats, Downloader, ProgressCallback};
pub use extractor::{CookieJar, ExtractionContext, InfoExtractor, JsEngine};
pub use postprocessor::{PostProcessConfig, PostProcessResult, PostProcessor};
