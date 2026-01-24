//! Error types for orchestration operations
//!
//! This module provides a consistent error handling strategy for the CLI orchestrator.
//!
//! # Error Wrapping Strategy
//!
//! - Domain errors from `rdlp-core`, `rdlp-extractor`, and `rdlp-downloader` are wrapped
//!   as `RdlpError` directly to preserve type information.
//! - External library errors (e.g., indicatif) are wrapped in descriptive variants.
//! - I/O errors are preserved directly via `#[from]`.

use rdlp_core::RdlpError;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during orchestration
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// No extractor found for the given URL
    #[error("No extractor found for URL: {url}")]
    NoExtractor { url: String },

    /// Video extraction failed (wraps domain RdlpError)
    #[error("Failed to extract video information: {0}")]
    ExtractionFailed(#[source] RdlpError),

    /// User cancelled the operation
    #[error("Operation cancelled by user")]
    UserCancelled,

    /// No suitable format found
    #[error("No suitable format found matching criteria")]
    NoFormat,

    /// Format selector parsing failed
    #[error("Invalid format selector: {0}")]
    InvalidFormatSelector(String),

    /// No downloader found for the URL
    #[error("No downloader found for URL: {url}")]
    NoDownloader { url: String },

    /// Download failed (wraps domain RdlpError)
    #[error("Download failed: {0}")]
    DownloadFailed(#[source] RdlpError),

    /// Resume detection failed
    #[error("Failed to detect resume point: {0}")]
    ResumeDetectionFailed(String),

    /// Missing chunk file during merge
    #[error("Missing chunk file: {path}")]
    MissingChunk { path: PathBuf },

    /// Chunk merge failed
    #[error("Failed to merge chunk files: {0}")]
    ChunkMergeFailed(#[source] std::io::Error),

    /// Failed to generate output path
    #[error("Failed to generate output path: {0}")]
    PathGenerationFailed(String),

    /// Progress bar template error (external library)
    #[error("Failed to create progress bar: {0}")]
    ProgressBarFailed(String),

    /// I/O error with custom message
    #[error("{0}")]
    IoError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for orchestrator operations
pub type Result<T> = std::result::Result<T, OrchestratorError>;
