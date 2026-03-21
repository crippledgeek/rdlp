//! Core traits defining the plugin architecture for rdlp.
//!
//! This module contains the fundamental traits that enable the modular design:
//! - [`InfoExtractor`] - Extract video metadata from URLs
//! - [`Downloader`] - Download content via various protocols

/// Download protocol traits and progress tracking
pub mod downloader;
/// Video metadata extraction traits
pub mod extractor;
/// Post-processing configuration and callbacks
pub mod postprocessor;

pub use downloader::{DownloadProgress, DownloadStats, Downloader, ProgressCallback};
pub use extractor::{CookieJar, ExtractionContext, InfoExtractor, JsEngine, SearchExtractor};
pub use postprocessor::{PostProcessCallback, PostProcessCallbackFactory, PostProcessConfig};
