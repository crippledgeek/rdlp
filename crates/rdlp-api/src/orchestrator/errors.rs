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
use rdlp_redact::RedactedUrlBuf;
use rdlp_redact::redact_str as redact;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during orchestration
#[derive(Error)]
pub enum OrchestratorError {
    /// No extractor found for the given URL
    #[error("No extractor found for URL: {url}")]
    NoExtractor {
        /// The URL that no extractor was found for
        url: RedactedUrlBuf,
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
    #[error("Invalid format selector: {}", redact(_0))]
    InvalidFormatSelector(String),

    /// No downloader found for the URL
    #[error("No downloader found for URL: {url}")]
    NoDownloader {
        /// The URL that no downloader was found for
        url: RedactedUrlBuf,
    },

    /// Download failed (wraps domain `RdlpError`)
    #[error("Download failed: {0}")]
    DownloadFailed(#[source] RdlpError),

    /// Post-processing pipeline failed with a fatal, non-cancel error.
    ///
    /// Propagated from [`run_postprocessing`] when the pipeline returns a
    /// non-[`PipelineError::Cancelled`] failure. The caller receives
    /// [`Event::Failed`] rather than a silent fallback to the unprocessed files.
    #[error("Post-processing failed: {}", redact(_0))]
    PostProcessingFailed(String),

    /// Resume detection failed
    #[error("Failed to detect resume point: {}", redact(_0))]
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
    #[error("Failed to generate output path: {}", redact(_0))]
    PathGenerationFailed(String),

    /// I/O error with custom message
    #[error("{}", redact(_0))]
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
    #[error("{}", redact(_0))]
    Configuration(String),

    /// Interactive callback not configured but interactive mode was requested
    #[error("Interactive format selection requested but no interactive callback is configured")]
    InteractiveNotConfigured,

    /// Catch-all for errors with context chains from internal operations.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Debug redacts the free text while keeping the structure.
///
/// The derived Debug printed each payload verbatim, so `{e:?}` leaked what
/// Display now strips. Typed sources keep the derive's shape: `RdlpError`
/// redacts in its own Debug, and `io::Error`/`anyhow::Error` render their own
/// crate's text.
impl std::fmt::Debug for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// A newtype variant carrying operator-assembled free text.
        macro_rules! text {
            ($name:literal, $t:expr) => {
                f.debug_tuple($name).field(&redact($t)).finish()
            };
        }

        match self {
            Self::NoExtractor { url } => f.debug_struct("NoExtractor").field("url", url).finish(),
            Self::NoDownloader { url } => f.debug_struct("NoDownloader").field("url", url).finish(),
            Self::MissingChunk { path } => {
                f.debug_struct("MissingChunk").field("path", path).finish()
            }
            Self::ExtractionFailed(e) => f.debug_tuple("ExtractionFailed").field(e).finish(),
            Self::DownloadFailed(e) => f.debug_tuple("DownloadFailed").field(e).finish(),
            Self::ChunkMergeFailed(e) => f.debug_tuple("ChunkMergeFailed").field(e).finish(),
            Self::Io(e) => f.debug_tuple("Io").field(e).finish(),
            Self::Other(e) => f.debug_tuple("Other").field(e).finish(),
            Self::InvalidFormatSelector(t) => text!("InvalidFormatSelector", t),
            Self::PostProcessingFailed(t) => text!("PostProcessingFailed", t),
            Self::ResumeDetectionFailed(t) => text!("ResumeDetectionFailed", t),
            Self::PathGenerationFailed(t) => text!("PathGenerationFailed", t),
            Self::IoError(t) => text!("IoError", t),
            Self::Configuration(t) => text!("Configuration", t),
            Self::UserCancelled => f.write_str("UserCancelled"),
            Self::NoFormat => f.write_str("NoFormat"),
            Self::InteractiveNotConfigured => f.write_str("InteractiveNotConfigured"),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_redact::RedactedUrlBuf;

    /// `NoExtractor` display must redact credentials in the URL.
    ///
    /// Failing-first: with `url: String`, `to_string()` would produce the raw
    /// URL including "SECRET", causing `!contains("SECRET")` to fail.
    #[test]
    fn no_extractor_display_redacts() {
        let err = OrchestratorError::NoExtractor {
            url: RedactedUrlBuf::from("https://x.example.com/v?token=SECRET"),
        };
        let display = err.to_string();
        assert!(
            !display.contains("SECRET"),
            "Display must not contain raw credential; got: {display}"
        );
        assert!(
            display.contains("token=***"),
            "Display must contain redacted placeholder; got: {display}"
        );
    }

    /// `NoDownloader` display must redact credentials in the URL.
    ///
    /// Failing-first: with `url: String`, `to_string()` would produce the raw
    /// URL including "SECRET", causing `!contains("SECRET")` to fail.
    #[test]
    fn no_downloader_display_redacts() {
        let err = OrchestratorError::NoDownloader {
            url: RedactedUrlBuf::from("https://cdn.example.com/v?token=SECRET"),
        };
        let display = err.to_string();
        assert!(
            !display.contains("SECRET"),
            "Display must not contain raw credential; got: {display}"
        );
        assert!(
            display.contains("token=***"),
            "Display must contain redacted placeholder; got: {display}"
        );
    }
}
