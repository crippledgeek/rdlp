//! Desktop application error types.
//!
//! [`AppError`] wraps [`RdlpApiError`] into a frontend-friendly enum that
//! serializes as externally-tagged JSON (`{ "kind": "...", "data": { ... } }`)
//! for consistent IPC error handling in the Tauri frontend.

use std::fmt;

use rdlp_api::RdlpApiError;
use rdlp_redact::redact_str as redact;
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
#[derive(Serialize)]
#[serde(tag = "kind", content = "data")]
pub enum AppError {
    /// A search operation failed.
    SearchFailed {
        /// Human-readable error message.
        #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
        message: String,
        /// Whether the frontend should offer a retry button.
        retryable: bool,
    },
    /// A network-level failure (timeout, DNS, HTTP 5xx, etc.).
    NetworkError {
        /// Human-readable error message.
        #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
        message: String,
        /// Whether the frontend should offer a retry button.
        retryable: bool,
    },
    /// Metadata extraction failed for a URL.
    ExtractionFailed {
        /// Human-readable error message.
        #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
        message: String,
    },
    /// A download job failed.
    DownloadFailed {
        /// The UUID of the failed download job.
        job_id: String,
        /// Human-readable error message.
        #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
        message: String,
        /// Whether the frontend should offer a retry button.
        retryable: bool,
    },
    /// Invalid user input (URL, settings field, etc.).
    InvalidInput {
        /// Which input field is invalid.
        field: String,
        /// What is wrong with the value.
        #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
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
        #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
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
            // `url` is a `RedactedUrlBuf`, so Display already strips
            // credentials; bound as `safe_url` to say so at the use site.
            RdlpApiError::UnsupportedUrl { url: safe_url } => Self::InvalidInput {
                field: "url".to_owned(),
                message: format!("Unsupported URL: {safe_url}"),
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

/// Debug redacts the free text while keeping the structure.
///
/// The derived Debug printed every field verbatim, so `{e:?}` leaked what
/// `{e}` strips. Delegating to Display is not the fix — Display omits
/// `retryable` from three variants, so `{e:?}` would silently stop reporting
/// whether the frontend was told it could retry. That is the same trade
/// rejected for `RdlpApiError`, where a pre-existing test caught it.
///
/// So this mirrors the derive with `message` passed through `redact`. Adding a
/// variant means adding an arm — the compiler enforces that, which is why this
/// is a `match` with no catch-all.
impl fmt::Debug for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// A struct variant carrying `message` and `retryable`.
        macro_rules! retryable {
            ($name:literal, $message:expr, $retryable:expr) => {
                f.debug_struct($name)
                    .field("message", &redact($message))
                    .field("retryable", $retryable)
                    .finish()
            };
        }

        match self {
            Self::SearchFailed { message, retryable } => {
                retryable!("SearchFailed", message, retryable)
            }
            Self::NetworkError { message, retryable } => {
                retryable!("NetworkError", message, retryable)
            }
            Self::ExtractionFailed { message } => f
                .debug_struct("ExtractionFailed")
                .field("message", &redact(message))
                .finish(),
            Self::DownloadFailed {
                job_id,
                message,
                retryable,
            } => f
                .debug_struct("DownloadFailed")
                .field("job_id", job_id)
                .field("message", &redact(message))
                .field("retryable", retryable)
                .finish(),
            Self::InvalidInput { field, message } => f
                .debug_struct("InvalidInput")
                .field("field", field)
                .field("message", &redact(message))
                .finish(),
            Self::RateLimited { retry_after_ms } => f
                .debug_struct("RateLimited")
                .field("retry_after_ms", retry_after_ms)
                .finish(),
            Self::Internal { message } => f
                .debug_struct("Internal")
                .field("message", &redact(message))
                .finish(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SearchFailed { message, .. } => {
                write!(f, "Search failed: {}", redact(message))
            }
            Self::NetworkError { message, .. } => {
                write!(f, "Network error: {}", redact(message))
            }
            Self::ExtractionFailed { message } => {
                write!(f, "Extraction failed: {}", redact(message))
            }
            Self::DownloadFailed {
                job_id, message, ..
            } => {
                write!(f, "Download {job_id} failed: {}", redact(message))
            }
            Self::InvalidInput { field, message } => {
                write!(f, "Invalid {field}: {}", redact(message))
            }
            Self::RateLimited { retry_after_ms } => match retry_after_ms {
                Some(ms) => write!(f, "Rate limited, retry after {ms} ms"),
                None => write!(f, "Rate limited"),
            },
            Self::Internal { message } => {
                write!(f, "Internal error: {}", redact(message))
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

    #[test]
    fn serialized_messages_are_redacted() {
        // The load-bearing one for the UI. `AppError` derives `Serialize`,
        // which reads the field directly — so the Display/Debug redaction that
        // covers RdlpError and RdlpApiError does nothing here, and a test on
        // those two would pass while the frontend still received the password.
        let e = AppError::NetworkError {
            message: "failed for uri (https://admin:hunter2@cdn.example.com/v.mp4)".to_string(),
            retryable: true,
        };
        let json = serde_json::to_string(&e).expect("AppError must serialize");
        assert!(
            !json.contains("hunter2"),
            "password reached the frontend: {json}"
        );
        assert!(
            !json.contains("admin"),
            "username reached the frontend: {json}"
        );
        assert!(
            json.contains("cdn.example.com"),
            "over-redacted, message no longer says where: {json}"
        );
    }

    #[test]
    fn app_error_display_and_debug_redact_credentials() {
        let e = AppError::NetworkError {
            message: "failed for uri (https://admin:hunter2@cdn.example.com/v.mp4)".to_string(),
            retryable: true,
        };
        let shown = e.to_string();
        let dbg = format!("{e:?}");
        assert!(!shown.contains("hunter2"), "Display leaked: {shown}");
        assert!(!dbg.contains("hunter2"), "Debug leaked: {dbg}");
        // Display omits `retryable`; Debug must not, or `{e:?}` stops
        // reporting whether the frontend was told it could retry.
        assert!(
            dbg.contains("retryable"),
            "Debug dropped a field Display omits: {dbg}"
        );
    }
}
