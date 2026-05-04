//! Stable error types for the rdlp public API.
//!
//! [`RdlpApiError`] maps internal error types to a stable, frontend-friendly
//! enum with human-readable messages and retryability information.

use crate::orchestrator::OrchestratorError;
use rdlp_core::RdlpError;
use std::borrow::Cow;
use thiserror::Error;

/// Stable error enum for the public API.
///
/// Internal implementation details (regex errors, JSON parse errors) are wrapped
/// into the appropriate high-level variant with a user-friendly message.
#[derive(Debug, Clone, Error)]
pub enum RdlpApiError {
    /// Invalid URL or request parameters.
    #[error("Invalid input: {message}")]
    InvalidInput {
        /// Description of what's invalid.
        message: String,
    },

    /// No extractor found for the given URL.
    #[error("Unsupported URL: {url}")]
    UnsupportedUrl {
        /// The URL that no extractor was found for.
        url: String,
    },

    /// Extraction failed (metadata retrieval).
    #[error("Extraction failed for {source_url}: {message}")]
    ExtractError {
        /// What went wrong.
        message: String,
        /// The URL being extracted.
        source_url: String,
    },

    /// Network or HTTP failure.
    #[error("Network error: {message}")]
    NetworkError {
        /// Description of the failure.
        message: String,
        /// HTTP status code, if applicable.
        status: Option<u16>,
    },

    /// Filesystem I/O error.
    #[error("I/O error: {message}")]
    IoError {
        /// Description of the I/O failure.
        message: String,
    },

    /// `FFmpeg` processing failed.
    #[error("FFmpeg error: {message}")]
    FfmpegError {
        /// What went wrong during post-processing.
        message: String,
    },

    /// Feature not available on this platform.
    #[error("Unsupported platform for feature: {feature}")]
    UnsupportedPlatform {
        /// The feature that is not available.
        feature: String,
    },

    /// User cancelled the download.
    #[error("Operation cancelled by user")]
    UserCancelled,

    /// Non-fatal error (logged but not propagated as failure).
    #[error("Soft error: {message}")]
    Soft {
        /// Warning message.
        message: String,
    },

    /// Builder misconfiguration.
    #[error("Builder error: {message}")]
    BuilderError {
        /// What's wrong with the builder configuration.
        message: String,
    },
}

impl RdlpApiError {
    /// Human-friendly message suitable for UI display.
    #[must_use]
    pub fn user_message(&self) -> Cow<'static, str> {
        match self {
            Self::InvalidInput { message } => Cow::Owned(format!("Invalid input: {message}")),
            Self::UnsupportedUrl { url } => Cow::Owned(format!("This URL is not supported: {url}")),
            Self::ExtractError { message, .. } => {
                Cow::Owned(format!("Could not extract video info: {message}"))
            }
            Self::NetworkError {
                message,
                status: Some(429),
            } => Cow::Owned(format!("Rate limited by server. {message}")),
            Self::NetworkError {
                status: Some(404), ..
            } => Cow::Borrowed("Content not found — it may have been removed"),
            Self::NetworkError {
                message,
                status: Some(s),
            } if *s >= 500 => Cow::Owned(format!("Server error ({s}). {message}")),
            Self::NetworkError { message, .. } => Cow::Owned(format!("Network error: {message}")),
            Self::IoError { message } => Cow::Owned(format!("File error: {message}")),
            Self::FfmpegError { message } => Cow::Owned(format!("Processing error: {message}")),
            Self::UnsupportedPlatform { feature } => {
                Cow::Owned(format!("{feature} is not available on this platform"))
            }
            Self::UserCancelled => Cow::Borrowed("Download cancelled"),
            Self::Soft { message } => Cow::Owned(message.clone()),
            Self::BuilderError { message } => Cow::Owned(format!("Configuration error: {message}")),
        }
    }

    /// Whether retrying this operation might succeed.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::NetworkError {
                status: Some(status),
                ..
            } => *status == 429 || *status == 403 || *status >= 500,
            Self::NetworkError { status: None, .. } => true,
            Self::ExtractError { message, .. } => message.contains("invalid M3U8"),
            _ => false,
        }
    }

    /// Whether re-extracting fresh CDN URLs might resolve this error.
    ///
    /// CDN token expiry (403), server overload (503), and corrupted HLS
    /// playlists ("invalid M3U8") can be resolved by re-running extraction
    /// to obtain fresh URLs.
    #[must_use]
    pub fn is_reextractable(&self) -> bool {
        match self {
            Self::NetworkError {
                status: Some(403 | 503),
                ..
            } => true,
            Self::ExtractError { message, .. } => message.contains("invalid M3U8"),
            _ => false,
        }
    }
}

impl From<RdlpError> for RdlpApiError {
    /// Convert an internal [`RdlpError`] to a stable API error.
    ///
    /// **Note:** `RdlpError` variants do not carry the source URL, so
    /// `ExtractError::source_url` is left empty in this blanket conversion.
    /// Call sites that have the URL available should use
    /// `RdlpApiError::ExtractError { message, source_url }` directly instead
    /// of relying on this `From` impl.
    fn from(err: RdlpError) -> Self {
        match err {
            RdlpError::Network { message, .. } | RdlpError::Download { message, .. } => {
                Self::NetworkError {
                    message,
                    status: None,
                }
            }
            RdlpError::Http { status, reason } => Self::NetworkError {
                message: reason,
                status: Some(status),
            },
            RdlpError::Extraction { message, url } => Self::ExtractError {
                message,
                source_url: url.unwrap_or_default(),
            },
            RdlpError::NoExtractor(url) => Self::UnsupportedUrl { url },
            RdlpError::InvalidUrl(msg) | RdlpError::FormatSelection(msg) => {
                Self::InvalidInput { message: msg }
            }
            RdlpError::PostProcess(msg) | RdlpError::FFmpeg(msg) => {
                Self::FfmpegError { message: msg }
            }
            RdlpError::JavaScript(msg) => Self::ExtractError {
                message: format!("JavaScript error: {msg}"),
                source_url: String::new(),
            },
            RdlpError::Cookie(msg) => Self::IoError {
                message: format!("Cookie error: {msg}"),
            },
            RdlpError::Plugin(msg) => Self::InvalidInput {
                message: format!("Plugin error: {msg}"),
            },
            RdlpError::Config(msg) => Self::BuilderError { message: msg },
            RdlpError::Io(err) => Self::IoError {
                message: err.to_string(),
            },
            RdlpError::UrlParse(err) => Self::InvalidInput {
                message: err.to_string(),
            },
            RdlpError::Json(err) => Self::ExtractError {
                message: format!("JSON parse error: {err}"),
                source_url: String::new(),
            },
            RdlpError::Regex(err) => Self::ExtractError {
                message: format!("Regex error: {err}"),
                source_url: String::new(),
            },
            RdlpError::Unsupported(msg) => Self::UnsupportedPlatform { feature: msg },
            RdlpError::Other(msg) => Self::Soft { message: msg },
            RdlpError::Cancelled => Self::UserCancelled,
        }
    }
}

impl From<OrchestratorError> for RdlpApiError {
    fn from(err: OrchestratorError) -> Self {
        match err {
            OrchestratorError::NoExtractor { url } => Self::UnsupportedUrl { url },
            OrchestratorError::ExtractionFailed(rdlp_err)
            | OrchestratorError::DownloadFailed(rdlp_err) => Self::from(rdlp_err),
            OrchestratorError::UserCancelled => Self::UserCancelled,
            OrchestratorError::NoFormat => Self::InvalidInput {
                message: "No suitable format found".into(),
            },
            OrchestratorError::InvalidFormatSelector(msg)
            | OrchestratorError::Configuration(msg) => Self::InvalidInput { message: msg },
            OrchestratorError::NoDownloader { url } => Self::InvalidInput {
                message: format!("No downloader for: {url}"),
            },
            OrchestratorError::ResumeDetectionFailed(msg)
            | OrchestratorError::PathGenerationFailed(msg)
            | OrchestratorError::IoError(msg) => Self::IoError { message: msg },
            OrchestratorError::MissingChunk { path } => Self::IoError {
                message: format!("Missing chunk file: {}", path.display()),
            },
            OrchestratorError::ChunkMergeFailed(io_err) => Self::IoError {
                message: format!("Chunk merge failed: {io_err}"),
            },
            OrchestratorError::Io(io_err) => Self::IoError {
                message: io_err.to_string(),
            },
            OrchestratorError::InteractiveNotConfigured => Self::InvalidInput {
                message: "Interactive selection not configured".into(),
            },
            OrchestratorError::Other(e) => Self::Soft {
                message: format!("{e:#}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_message_cancelled() {
        assert_eq!(
            RdlpApiError::UserCancelled.user_message(),
            "Download cancelled"
        );
    }

    #[test]
    fn test_user_message_network_429() {
        let err = RdlpApiError::NetworkError {
            message: "slow down".into(),
            status: Some(429),
        };
        let msg = err.user_message();
        assert!(msg.contains("Rate limited"));
    }

    #[test]
    fn test_user_message_network_404() {
        let err = RdlpApiError::NetworkError {
            message: "Not Found".into(),
            status: Some(404),
        };
        let msg = err.user_message();
        assert!(msg.contains("not found"), "message: {msg}");
        assert!(msg.contains("removed"), "message: {msg}");
    }

    #[test]
    fn test_user_message_network_503() {
        let err = RdlpApiError::NetworkError {
            message: "unavailable".into(),
            status: Some(503),
        };
        let msg = err.user_message();
        assert!(msg.contains("Server error (503)"));
    }

    #[test]
    fn test_is_retryable_429() {
        assert!(
            RdlpApiError::NetworkError {
                message: "rate limited".into(),
                status: Some(429),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_is_retryable_5xx() {
        assert!(
            RdlpApiError::NetworkError {
                message: "server error".into(),
                status: Some(503),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_not_retryable_404() {
        assert!(
            !RdlpApiError::NetworkError {
                message: "not found".into(),
                status: Some(404),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_not_retryable_cancelled() {
        assert!(!RdlpApiError::UserCancelled.is_retryable());
    }

    #[test]
    fn test_not_retryable_ffmpeg() {
        assert!(
            !RdlpApiError::FfmpegError {
                message: "bad".into(),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_retryable_connection_error() {
        assert!(
            RdlpApiError::NetworkError {
                message: "connection reset".into(),
                status: None,
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_is_retryable_403() {
        assert!(
            RdlpApiError::NetworkError {
                message: "forbidden".into(),
                status: Some(403),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_is_retryable_invalid_m3u8() {
        assert!(
            RdlpApiError::ExtractError {
                message: "invalid M3U8 playlist".into(),
                source_url: String::new(),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_is_reextractable_403() {
        assert!(
            RdlpApiError::NetworkError {
                message: "Forbidden".into(),
                status: Some(403),
            }
            .is_reextractable()
        );
    }

    #[test]
    fn test_is_reextractable_503() {
        assert!(
            RdlpApiError::NetworkError {
                message: "Service Unavailable".into(),
                status: Some(503),
            }
            .is_reextractable()
        );
    }

    #[test]
    fn test_is_reextractable_invalid_m3u8() {
        assert!(
            RdlpApiError::ExtractError {
                message: "invalid M3U8 playlist".into(),
                source_url: String::new(),
            }
            .is_reextractable()
        );
    }

    #[test]
    fn test_not_reextractable_404() {
        assert!(
            !RdlpApiError::NetworkError {
                message: "Not Found".into(),
                status: Some(404),
            }
            .is_reextractable()
        );
    }

    #[test]
    fn test_not_reextractable_cancelled() {
        assert!(!RdlpApiError::UserCancelled.is_reextractable());
    }

    #[test]
    fn test_from_rdlp_http_error() {
        let err: RdlpApiError = RdlpError::Http {
            status: 429,
            reason: "Too Many Requests".into(),
        }
        .into();
        match err {
            RdlpApiError::NetworkError {
                status: Some(429), ..
            } => {}
            other => panic!("Expected NetworkError with 429, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_rdlp_no_extractor() {
        let err: RdlpApiError = RdlpError::NoExtractor("http://unknown.com".into()).into();
        match err {
            RdlpApiError::UnsupportedUrl { url } => {
                assert_eq!(url, "http://unknown.com");
            }
            other => panic!("Expected UnsupportedUrl, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_rdlp_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: RdlpApiError = RdlpError::Io(io_err).into();
        match err {
            RdlpApiError::IoError { message } => {
                assert!(message.contains("file missing"));
            }
            other => panic!("Expected IoError, got: {other:?}"),
        }
    }

    #[test]
    fn test_extraction_url_propagates_to_api_error() {
        let err: RdlpApiError = RdlpError::Extraction {
            message: "no formats".into(),
            url: Some("https://example.com/video".into()),
        }
        .into();
        match err {
            RdlpApiError::ExtractError { source_url, .. } => {
                assert_eq!(source_url, "https://example.com/video");
            }
            other => panic!("Expected ExtractError, got: {other:?}"),
        }
    }

    #[test]
    fn test_extraction_none_url_becomes_empty() {
        let err: RdlpApiError = RdlpError::Extraction {
            message: "parse failed".into(),
            url: None,
        }
        .into();
        match err {
            RdlpApiError::ExtractError { source_url, .. } => {
                assert!(source_url.is_empty());
            }
            other => panic!("Expected ExtractError, got: {other:?}"),
        }
    }

    #[test]
    fn test_network_url_propagates_to_api_error() {
        let err: RdlpApiError = RdlpError::Network {
            message: "timeout".into(),
            url: Some("https://cdn.example.com".into()),
        }
        .into();
        match err {
            RdlpApiError::NetworkError { message, .. } => {
                assert!(message.contains("timeout"));
            }
            other => panic!("Expected NetworkError, got: {other:?}"),
        }
    }

    #[test]
    fn test_download_url_propagates_to_api_error() {
        let err: RdlpApiError = RdlpError::Download {
            message: "chunk failed".into(),
            url: Some("https://cdn.example.com/seg.ts".into()),
        }
        .into();
        match err {
            RdlpApiError::NetworkError { message, .. } => {
                assert!(message.contains("chunk failed"));
            }
            other => panic!("Expected NetworkError, got: {other:?}"),
        }
    }

    #[test]
    fn test_anyhow_converts_to_orchestrator_other() {
        let anyhow_err = anyhow::anyhow!("inner").context("outer");
        let orch_err: OrchestratorError = anyhow_err.into();
        assert!(matches!(orch_err, OrchestratorError::Other(_)));
        assert!(orch_err.to_string().contains("outer"));
    }

    #[test]
    fn test_orchestrator_other_converts_to_api_error() {
        let anyhow_err = anyhow::anyhow!("root cause").context("operation failed");
        let orch_err: OrchestratorError = anyhow_err.into();
        let api_err: RdlpApiError = orch_err.into();
        assert!(matches!(api_err, RdlpApiError::Soft { .. }));
        let msg = api_err.to_string();
        assert!(msg.contains("operation failed"), "msg: {msg}");
        assert!(msg.contains("root cause"), "msg: {msg}");
    }

    #[test]
    fn test_orchestrator_other_not_reextractable() {
        use crate::orchestrator::errors::is_reextractable_error;
        let anyhow_err = anyhow::anyhow!("something");
        let orch_err: OrchestratorError = anyhow_err.into();
        assert!(!is_reextractable_error(&orch_err));
    }

    #[test]
    fn test_all_rdlp_error_variants_convert() {
        let variants: Vec<RdlpError> = vec![
            RdlpError::Network {
                message: "a".into(),
                url: None,
            },
            RdlpError::Http {
                status: 500,
                reason: "b".into(),
            },
            RdlpError::Extraction {
                message: "c".into(),
                url: None,
            },
            RdlpError::NoExtractor("d".into()),
            RdlpError::InvalidUrl("e".into()),
            RdlpError::Download {
                message: "f".into(),
                url: None,
            },
            RdlpError::PostProcess("g".into()),
            RdlpError::FFmpeg("h".into()),
            RdlpError::JavaScript("i".into()),
            RdlpError::Cookie("j".into()),
            RdlpError::Plugin("k".into()),
            RdlpError::FormatSelection("l".into()),
            RdlpError::Config("m".into()),
            RdlpError::Io(std::io::Error::other("n")),
            RdlpError::Unsupported("o".into()),
            RdlpError::Other("p".into()),
        ];
        for v in variants {
            let _api: RdlpApiError = v.into();
        }
    }
}
