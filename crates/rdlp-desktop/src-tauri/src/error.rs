//! Desktop application error types.
//!
//! [`AppError`] wraps [`RdlpApiError`] into a frontend-friendly enum that
//! serializes as externally-tagged JSON (`{ "kind": "...", "data": { ... } }`)
//! for consistent IPC error handling in the Tauri frontend.

use std::fmt;

use rdlp_api::RdlpApiError;
use serde::Serialize;

/// Frontend-facing error type for Tauri IPC commands.
///
/// Each variant carries structured data that the React frontend can
/// pattern-match on to render appropriate UI (retry buttons, field
/// validation hints, rate-limit backoff, etc.).
///
/// # Serialization
///
/// Uses Serde's internally-tagged representation so JSON output
/// always contains a `"kind"` discriminator and a `"data"` payload:
///
/// ```json
/// {
///   "kind": "NetworkError",
///   "data": { "message": "connection reset", "retryable": true }
/// }
/// ```
#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum AppError {
    /// A search operation failed.
    SearchFailed {
        /// Human-readable error message.
        message: String,
        /// Whether the frontend should offer a retry button.
        retryable: bool,
    },
    /// A network-level failure (timeout, DNS, HTTP 5xx, etc.).
    NetworkError {
        /// Human-readable error message.
        message: String,
        /// Whether the frontend should offer a retry button.
        retryable: bool,
    },
    /// Metadata extraction failed for a URL.
    ExtractionFailed {
        /// Human-readable error message.
        message: String,
    },
    /// A download job failed.
    DownloadFailed {
        /// The UUID of the failed download job.
        job_id: String,
        /// Human-readable error message.
        message: String,
        /// Whether the frontend should offer a retry button.
        retryable: bool,
    },
    /// Invalid user input (URL, settings field, etc.).
    InvalidInput {
        /// Which input field is invalid.
        field: String,
        /// What is wrong with the value.
        message: String,
    },
    /// Server returned HTTP 429; frontend should back off.
    RateLimited {
        /// Suggested wait time before retrying, in milliseconds.
        retry_after_ms: Option<u64>,
    },
    /// An unexpected internal error.
    Internal {
        /// Human-readable error message.
        message: String,
    },
}

impl From<RdlpApiError> for AppError {
    fn from(err: RdlpApiError) -> Self {
        match &err {
            RdlpApiError::InvalidInput { message } => Self::InvalidInput {
                field: "url".to_owned(),
                message: message.clone(),
            },
            RdlpApiError::UnsupportedUrl { url } => Self::InvalidInput {
                field: "url".to_owned(),
                message: format!("Unsupported URL: {url}"),
            },
            RdlpApiError::ExtractError { .. }
            | RdlpApiError::NetworkError {
                status: Some(404), ..
            } => Self::ExtractionFailed {
                message: err.user_message().into_owned(),
            },
            RdlpApiError::NetworkError {
                status: Some(429), ..
            } => Self::RateLimited {
                retry_after_ms: Some(5000),
            },
            RdlpApiError::NetworkError { .. } => Self::NetworkError {
                message: err.user_message().into_owned(),
                retryable: err.is_retryable(),
            },
            _ => Self::Internal {
                message: err.user_message().into_owned(),
            },
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SearchFailed { message, .. } => {
                write!(f, "Search failed: {message}")
            }
            Self::NetworkError { message, .. } => {
                write!(f, "Network error: {message}")
            }
            Self::ExtractionFailed { message } => {
                write!(f, "Extraction failed: {message}")
            }
            Self::DownloadFailed {
                job_id, message, ..
            } => {
                write!(f, "Download {job_id} failed: {message}")
            }
            Self::InvalidInput { field, message } => {
                write!(f, "Invalid {field}: {message}")
            }
            Self::RateLimited { retry_after_ms } => match retry_after_ms {
                Some(ms) => write!(f, "Rate limited, retry after {ms} ms"),
                None => write!(f, "Rate limited"),
            },
            Self::Internal { message } => {
                write!(f, "Internal error: {message}")
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::similar_names, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_from_network_429() {
        let api_err = RdlpApiError::NetworkError {
            message: "Too Many Requests".into(),
            status: Some(429),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::RateLimited { retry_after_ms } => {
                assert_eq!(retry_after_ms, Some(5000));
            }
            other => panic!("Expected RateLimited, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_network_404_maps_to_extraction_failed() {
        let api_err = RdlpApiError::NetworkError {
            message: "Not Found".into(),
            status: Some(404),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::ExtractionFailed { message } => {
                assert!(message.contains("not found"), "message: {message}");
                assert!(message.contains("removed"), "message: {message}");
            }
            other => panic!("Expected ExtractionFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_network_503() {
        let api_err = RdlpApiError::NetworkError {
            message: "Service Unavailable".into(),
            status: Some(503),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::NetworkError { retryable, message } => {
                assert!(retryable);
                assert!(message.contains("503"));
            }
            other => panic!("Expected NetworkError, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_unsupported_url() {
        let api_err = RdlpApiError::UnsupportedUrl {
            url: "https://unknown.example.com/video".into(),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::InvalidInput { field, message } => {
                assert_eq!(field, "url");
                assert!(message.contains("Unsupported URL"));
                assert!(message.contains("https://unknown.example.com/video"));
            }
            other => panic!("Expected InvalidInput, got: {other:?}"),
        }
    }

    #[test]
    fn test_serializes_as_tagged_json() {
        let err = AppError::NetworkError {
            message: "connection reset".into(),
            retryable: true,
        };
        let json = serde_json::to_value(&err).expect("serialization should succeed");
        assert_eq!(json["kind"], "NetworkError");
        assert_eq!(json["data"]["message"], "connection reset");
        assert_eq!(json["data"]["retryable"], true);
    }

    #[test]
    fn test_from_extract_error() {
        use rdlp_redact::RedactedUrlBuf;
        let api_err = RdlpApiError::ExtractError {
            message: "page not found".into(),
            source_url: RedactedUrlBuf::from("https://example.com"),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::ExtractionFailed { message } => {
                assert!(message.contains("page not found"));
            }
            other => panic!("Expected ExtractionFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_invalid_input() {
        let api_err = RdlpApiError::InvalidInput {
            message: "empty URL".into(),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::InvalidInput { field, message } => {
                assert_eq!(field, "url");
                assert_eq!(message, "empty URL");
            }
            other => panic!("Expected InvalidInput, got: {other:?}"),
        }
    }

    #[test]
    fn test_from_io_error_maps_to_internal() {
        let api_err = RdlpApiError::IoError {
            message: "disk full".into(),
        };
        let app_err = AppError::from(api_err);
        match app_err {
            AppError::Internal { message } => {
                assert!(message.contains("disk full"));
            }
            other => panic!("Expected Internal, got: {other:?}"),
        }
    }

    #[test]
    fn test_display_rate_limited_with_ms() {
        let err = AppError::RateLimited {
            retry_after_ms: Some(3000),
        };
        assert_eq!(err.to_string(), "Rate limited, retry after 3000 ms");
    }

    #[test]
    fn test_display_rate_limited_without_ms() {
        let err = AppError::RateLimited {
            retry_after_ms: None,
        };
        assert_eq!(err.to_string(), "Rate limited");
    }

    #[test]
    fn test_display_download_failed() {
        let err = AppError::DownloadFailed {
            job_id: "abc-123".into(),
            message: "timeout".into(),
            retryable: true,
        };
        assert_eq!(err.to_string(), "Download abc-123 failed: timeout");
    }
}
