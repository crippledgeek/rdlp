//! Error types for orchestration operations

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during orchestration
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// No extractor found for the given URL
    #[error("No extractor found for URL: {url}")]
    NoExtractor { url: String },

    /// Video extraction failed
    #[error("Failed to extract video information: {0}")]
    ExtractionFailed(#[source] anyhow::Error),

    /// User cancelled the operation
    #[error("Operation cancelled by user")]
    UserCancelled,

    /// No suitable format found
    #[error("No suitable format found matching criteria")]
    NoFormat,

    /// Format selector parsing failed
    #[error("Invalid format selector: {0}")]
    InvalidFormatSelector(#[source] anyhow::Error),

    /// No downloader found for the URL
    #[error("No downloader found for URL: {url}")]
    NoDownloader { url: String },

    /// Download failed
    #[error("Download failed: {0}")]
    DownloadFailed(#[source] anyhow::Error),

    /// Resume detection failed
    #[error("Failed to detect resume point: {0}")]
    ResumeDetectionFailed(#[source] anyhow::Error),

    /// Missing chunk file during merge
    #[error("Missing chunk file: {path}")]
    MissingChunk { path: PathBuf },

    /// Chunk merge failed
    #[error("Failed to merge chunk files: {0}")]
    ChunkMergeFailed(#[source] std::io::Error),

    /// Failed to generate output path
    #[error("Failed to generate output path: {0}")]
    PathGenerationFailed(String),

    /// Progress bar creation failed
    #[error("Failed to create progress bar: {0}")]
    ProgressBarFailed(#[source] anyhow::Error),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for orchestrator operations
pub type Result<T> = std::result::Result<T, OrchestratorError>;
