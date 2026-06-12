//! Error types for orchestration operations
//!
//! This module provides a consistent error handling strategy for the orchestrator.
//!
//! # Error Wrapping Strategy
//!
//! - Domain errors from `rdlp-core`, `rdlp-extractor`, and `rdlp-downloader` are wrapped
//!   as `RdlpError` directly to preserve type information.
//! - I/O errors are preserved directly via `#[from]`.

use rdlp_core::RdlpError;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during orchestration
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// No extractor found for the given URL
    #[error("No extractor found for URL: {url}")]
    NoExtractor {
        /// The URL that no extractor was found for
        url: String,
    },

    /// Video extraction failed (wraps domain `RdlpError`)
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
    NoDownloader {
        /// The URL that no downloader was found for
        url: String,
    },

    /// Download failed (wraps domain `RdlpError`)
    #[error("Download failed: {0}")]
    DownloadFailed(#[source] RdlpError),

    /// Post-processing pipeline failed with a fatal, non-cancel error.
    ///
    /// Propagated from [`run_postprocessing`] when the pipeline returns a
    /// non-[`PipelineError::Cancelled`] failure. The caller receives
    /// [`Event::Failed`] rather than a silent fallback to the unprocessed files.
    #[error("Post-processing failed: {0}")]
    PostProcessingFailed(String),

    /// Resume detection failed
    #[error("Failed to detect resume point: {0}")]
    ResumeDetectionFailed(String),

    /// Missing chunk file during merge
    #[error("Missing chunk file: {path}")]
    MissingChunk {
        /// Path to the missing chunk file
        path: PathBuf,
    },

    /// Chunk merge failed
    #[error("Failed to merge chunk files: {0}")]
    ChunkMergeFailed(#[source] std::io::Error),

    /// Failed to generate output path
    #[error("Failed to generate output path: {0}")]
    PathGenerationFailed(String),

    /// I/O error with custom message
    #[error("{0}")]
    IoError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration or runtime capability error.
    ///
    /// Covers both static misconfiguration (invalid options, missing files)
    /// and runtime rejections that depend on the selected format (e.g.
    /// "HLS not yet supported with stdout", "Merge downloads not supported
    /// with stdout").
    #[error("{0}")]
    Configuration(String),

    /// Interactive callback not configured but interactive mode was requested
    #[error("Interactive format selection requested but no interactive callback is configured")]
    InteractiveNotConfigured,

    /// Catch-all for errors with context chains from internal operations.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Check if an error warrants re-extracting fresh URLs.
///
/// CDN failures (Cloudflare challenges, expired tokens, server errors)
/// return `Extraction` errors containing "invalid M3U8". These can be
/// resolved by calling `extract_lazy()` again for a fresh CDN assignment.
pub fn is_reextractable_error(err: &OrchestratorError) -> bool {
    match err {
        OrchestratorError::DownloadFailed(RdlpError::Extraction { message, .. }) => {
            message.contains("invalid M3U8")
        }
        OrchestratorError::DownloadFailed(RdlpError::Http {
            status: 403 | 503, ..
        }) => true,
        _ => false,
    }
}

/// Result type for orchestrator operations
pub type Result<T> = std::result::Result<T, OrchestratorError>;
