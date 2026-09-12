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

    /// Post-processing was required, and the `FFmpeg` on this system cannot be
    /// called because its ABI disagrees with this binary's bindings.
    ///
    /// Distinct from [`Self::PostProcessingFailed`] because no stage ran: the
    /// refusal happens before any FFI, which is the whole point of the check
    /// (rdlp#656). It fails the run rather than degrading silently — `FFmpeg`
    /// is installed, the operator asked for work that needs it, and returning
    /// an unmerged file as if it had succeeded is what rdlp#727 reported.
    // Through `redact` like every other interpolated error text: a no-op for
    // version numbers and a build prefix, and the gate that keeps it uniform
    // (`scripts/check-error-attr-redaction.sh`) is worth more than the
    // exception this variant could argue for.
    #[error("Post-processing requires FFmpeg, which cannot be used.\n{}", redact(&_0.to_string()))]
    FFmpegAbiMismatch(rdlp_ffmpeg::ffmpeg::abi::AbiMismatches),

    /// Resume detection failed
    #[error("Failed to detect resume point: {}", redact(_0))]
    ResumeDetectionFailed(String),

    /// Missing chunk file during merge
    #[error("Missing chunk file: {path}")]
    MissingChunk {
        /// Path to the missing chunk file
        path: PathBuf,
    },

    /// Another rdlp process already holds the output path's advisory lock.
    ///
    /// Two processes downloading the same target to the same path is a
    /// conflict worth reporting, not one worth silently making safe
    /// (rdlp#572) — surfaced instead of letting a second writer share the
    /// same `.rdlp-part` chunk files.
    #[error(
        "another rdlp process is downloading to {path}; wait for it to finish or choose a different output"
    )]
    OutputBusy {
        /// The output path already claimed by another process.
        path: PathBuf,
    },

    /// The output path's exclusive ownership claim could not even be
    /// ATTEMPTED — the `.lock` sidecar couldn't be created or locked (a
    /// pre-existing directory at the sidecar path, permissions, a
    /// read-only/full filesystem). Distinct from [`Self::OutputBusy`]: that
    /// means "someone else owns it, checked and confirmed"; this means "the
    /// check itself couldn't run", so treating it as success would hand out
    /// an unverified claim (rdlp#572, security review MEDIUM).
    // `redact(&source.to_string())` rather than a bare `{source}`: the
    // check-error-attr-redaction gate's typed-source exemption only covers
    // TUPLE-variant positional placeholders (`{0}` paired with `#[source]`);
    // a named struct field has no such carve-out, so this mirrors `Other`'s
    // `redact(&_0.to_string())` pattern below rather than fighting the gate.
    #[error("cannot verify exclusive ownership of {path}: {}", redact(&source.to_string()))]
    OutputUnclaimable {
        /// The output path the claim was for.
        path: PathBuf,
        /// The underlying I/O failure. OS-generated text, not
        /// caller-assembled — routed through `redact` anyway for uniformity
        /// with every other interpolated error text in this enum.
        #[source]
        source: std::io::Error,
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
    /// Catch-all for `anyhow`-carried failures.
    ///
    /// NOT `#[error(transparent)]`: `anyhow::Error` is a container whose text
    /// is our own `.context(...)` strings, assembled by `format!` over values
    /// that can include a URL — unlike `io::Error`, whose message is
    /// OS-generated and inert. Rendering it explicitly is what lets it be
    /// redacted.
    #[error("{}", redact(&_0.to_string()))]
    Other(#[from] anyhow::Error),
}

/// Debug redacts the free text while keeping the structure.
///
/// The derived Debug printed each payload verbatim, so `{e:?}` leaked what
/// Display now strips. `RdlpError` and `io::Error` keep the derive's shape —
/// the first redacts in its own Debug, the second's message is OS-generated.
/// `anyhow::Error` does NOT: its text is our own `.context(...)` strings, so
/// it is redacted here like any other free text.
impl std::fmt::Debug for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoExtractor { url } => f.debug_struct("NoExtractor").field("url", url).finish(),
            Self::NoDownloader { url } => f.debug_struct("NoDownloader").field("url", url).finish(),
            Self::MissingChunk { path } => {
                f.debug_struct("MissingChunk").field("path", path).finish()
            }
            Self::OutputBusy { path } => f.debug_struct("OutputBusy").field("path", path).finish(),
            Self::OutputUnclaimable { path, source } => f
                .debug_struct("OutputUnclaimable")
                .field("path", path)
                .field("source", source)
                .finish(),
            Self::ExtractionFailed(e) => f.debug_tuple("ExtractionFailed").field(e).finish(),
            Self::DownloadFailed(e) => f.debug_tuple("DownloadFailed").field(e).finish(),
            Self::ChunkMergeFailed(e) => f.debug_tuple("ChunkMergeFailed").field(e).finish(),
            Self::Io(e) => f.debug_tuple("Io").field(e).finish(),
            Self::Other(e) => f
                .debug_tuple("Other")
                .field(&redact(&e.to_string()))
                .finish(),
            Self::InvalidFormatSelector(t) => {
                rdlp_redact::redacted_debug_tuple!(f, "InvalidFormatSelector", t)
            }
            Self::PostProcessingFailed(t) => {
                rdlp_redact::redacted_debug_tuple!(f, "PostProcessingFailed", t)
            }
            Self::ResumeDetectionFailed(t) => {
                rdlp_redact::redacted_debug_tuple!(f, "ResumeDetectionFailed", t)
            }
            Self::PathGenerationFailed(t) => {
                rdlp_redact::redacted_debug_tuple!(f, "PathGenerationFailed", t)
            }
            Self::IoError(t) => rdlp_redact::redacted_debug_tuple!(f, "IoError", t),
            Self::Configuration(t) => rdlp_redact::redacted_debug_tuple!(f, "Configuration", t),
            // Not redacted: every field is a version number or the build
            // prefix baked in at compile time, none of it caller-supplied.
            Self::FFmpegAbiMismatch(mismatches) => f
                .debug_tuple("FFmpegAbiMismatch")
                .field(mismatches)
                .finish(),
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

#[cfg(test)]
mod redact_tests {
    use super::*;

    const LEAKY: &str = "failed for uri (https://admin:hunter2@cdn.example.com/v.mp4)";

    #[test]
    fn display_and_debug_redact_free_text() {
        let e = OrchestratorError::PostProcessingFailed(LEAKY.to_string());
        let shown = e.to_string();
        let dbg = format!("{e:?}");
        assert!(!shown.contains("hunter2"), "Display leaked: {shown}");
        assert!(!dbg.contains("hunter2"), "Debug leaked: {dbg}");
        // Positive assertions: an impl that writes nothing passes the two
        // above, which is how `cargo mutants` found this test missing.
        assert!(
            dbg.contains("PostProcessingFailed"),
            "variant missing: {dbg}"
        );
        assert!(
            dbg.contains("cdn.example.com"),
            "redacted text missing: {dbg}"
        );
    }

    #[test]
    fn the_anyhow_catch_all_is_redacted_too() {
        // `Other` was `#[error(transparent)]`, which forwarded anyhow's text —
        // our own `.context(...)` strings — with no redaction and no
        // placeholder for the gate to see.
        let e = OrchestratorError::Other(anyhow::anyhow!("{LEAKY}"));
        let shown = e.to_string();
        let dbg = format!("{e:?}");
        assert!(!shown.contains("hunter2"), "Display leaked: {shown}");
        assert!(!dbg.contains("hunter2"), "Debug leaked: {dbg}");
        assert!(shown.contains("cdn.example.com"), "over-redacted: {shown}");
    }
}
