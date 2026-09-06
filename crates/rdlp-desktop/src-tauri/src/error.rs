//! Desktop application error types.
//!
//! [`AppError`] wraps [`RdlpApiError`] into a frontend-friendly enum that
//! serializes as externally-tagged JSON (`{ "kind": "...", "data": { ... } }`)
//! for consistent IPC error handling in the Tauri frontend.

use std::fmt;

use log::{error, warn};
use rdlp_api::RdlpApiError;
use rdlp_redact::redact_str as redact;
use serde::Serialize;

use rdlp_types::boundary::Action;

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

/// Map an API error to its frontend shape, without recording anything.
///
/// Private: the only caller is [`AppError::from_api`], which pairs this
/// mapping with the terminal record. There is deliberately no `From`
/// impl here — a `?`-based conversion cannot log (it has no `Action`),
/// so `commands/formats/mod.rs:68`'s `.map_err(AppError::from)?` no
/// longer compiles once this impl is gone. That call site is fixed in a
/// later task, not here.
fn map_api(err: &RdlpApiError) -> AppError {
    match err {
        RdlpApiError::InvalidInput { message } => AppError::InvalidInput {
            field: "url".to_owned(),
            message: message.clone(),
        },
        // `url` is a `RedactedUrlBuf`, so Display already strips
        // credentials; bound as `safe_url` to say so at the use site.
        RdlpApiError::UnsupportedUrl { url: safe_url } => AppError::InvalidInput {
            field: "url".to_owned(),
            message: format!("Unsupported URL: {safe_url}"),
        },
        RdlpApiError::ExtractError { .. }
        | RdlpApiError::NetworkError {
            status: Some(404), ..
        } => AppError::ExtractionFailed {
            message: err.user_message().into_owned(),
        },
        RdlpApiError::NetworkError {
            status: Some(429), ..
        } => AppError::RateLimited {
            retry_after_ms: Some(5000),
        },
        RdlpApiError::NetworkError { .. } => AppError::NetworkError {
            message: err.user_message().into_owned(),
            retryable: err.is_retryable(),
        },
        _ => AppError::Internal {
            message: err.user_message().into_owned(),
        },
    }
}

/// Terminal-record constructors.
///
/// Every one of these logs the outcome as it builds the error. That is the
/// point: the boundary record is not something a call site can forget,
/// because there is no path to an `AppError` that does not write one. A
/// later task in this change set adds `scripts/check-boundary-log.sh` to
/// gate any bypass of these constructors.
///
/// The record interpolates `{self}` — the constructed error — rather than the
/// incoming `reason`, because `AppError`'s `Display` redacts every `message`
/// (see the impl below) and the incoming value is the unwrapped one.
impl AppError {
    /// Record and build an invalid-input failure. WARN: user-correctable.
    #[must_use]
    pub fn invalid_input(action: Action<'_>, field: &str, message: impl fmt::Display) -> Self {
        let e = Self::InvalidInput {
            field: field.to_owned(),
            message: message.to_string(),
        };
        warn!("{action} outcome=failed reason={e}");
        e
    }

    /// Record and build an internal failure. ERROR: unexpected state.
    #[must_use]
    pub fn internal(action: Action<'_>, reason: impl fmt::Display) -> Self {
        let e = Self::Internal {
            message: reason.to_string(),
        };
        error!("{action} outcome=failed reason={e}");
        e
    }

    /// Record and build a search failure. WARN: expected, user-facing.
    #[must_use]
    pub fn search_failed(action: Action<'_>, reason: impl fmt::Display, retryable: bool) -> Self {
        let e = Self::SearchFailed {
            message: reason.to_string(),
            retryable,
        };
        warn!("{action} outcome=failed reason={e}");
        e
    }

    /// Record and build a network failure. WARN: expected, user-facing.
    #[must_use]
    pub fn network(action: Action<'_>, reason: impl fmt::Display, retryable: bool) -> Self {
        let e = Self::NetworkError {
            message: reason.to_string(),
            retryable,
        };
        warn!("{action} outcome=failed reason={e}");
        e
    }

    /// Record and build an extraction failure. WARN: expected, user-facing.
    #[must_use]
    pub fn extraction_failed(action: Action<'_>, reason: impl fmt::Display) -> Self {
        let e = Self::ExtractionFailed {
            message: reason.to_string(),
        };
        warn!("{action} outcome=failed reason={e}");
        e
    }

    /// Record and build a download failure. The job id travels in `action`'s
    /// `Subject` (`rdlp_types::boundary::Subject`), not as a fourth parameter.
    #[must_use]
    pub fn download_failed(action: Action<'_>, reason: impl fmt::Display, retryable: bool) -> Self {
        let e = Self::DownloadFailed {
            job_id: action.job_id().unwrap_or_default().to_owned(),
            message: reason.to_string(),
            retryable,
        };
        warn!("{action} outcome=failed reason={e}");
        e
    }

    /// Record and build a rate-limit outcome. WARN: expected, user-facing.
    #[must_use]
    pub fn rate_limited(action: Action<'_>, retry_after_ms: Option<u64>) -> Self {
        let e = Self::RateLimited { retry_after_ms };
        warn!("{action} outcome=failed reason={e}");
        e
    }

    /// Map an API error to its frontend shape AND record it.
    ///
    /// This is the recording replacement for `From<RdlpApiError>`: the `From`
    /// impl cannot log, because it has no way to learn the action, and the
    /// module target names `error.rs` rather than the command.
    #[must_use]
    pub fn from_api(action: Action<'_>, err: &RdlpApiError) -> Self {
        let e = map_api(err);
        if matches!(e, Self::Internal { .. }) {
            error!("{action} outcome=failed reason={e}");
        } else {
            warn!("{action} outcome=failed reason={e}");
        }
        e
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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
        let app_err = AppError::from_api(Action::new("test"), &api_err);
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod boundary_record_tests {
    use super::AppError;
    use rdlp_types::boundary::{Action, Subject};

    /// Exactly one record, at WARN, carrying the full field vocabulary.
    #[test]
    fn a_constructed_boundary_error_records_once_at_warn() {
        testing_logger::setup();
        let err = AppError::search_failed(Action::new("search"), "upstream 503", true);

        testing_logger::validate(|captured| {
            let warns: Vec<_> = captured
                .iter()
                .filter(|l| l.level == log::Level::Warn)
                .collect();
            assert_eq!(warns.len(), 1, "exactly one terminal record");
            let body = warns.first().map_or("", |l| l.body.as_str());
            assert!(body.contains("action=search"), "names the action: {body}");
            assert!(
                body.contains("outcome=failed"),
                "states the outcome: {body}"
            );
            assert!(body.contains("upstream 503"), "names the reason: {body}");
        });

        assert!(matches!(
            err,
            AppError::SearchFailed {
                retryable: true,
                ..
            }
        ));
    }

    /// `Internal` is the unexpected-state variant, so it is the one that
    /// reaches ERROR. Everything else stays at WARN.
    #[test]
    fn an_internal_error_records_at_error_not_warn() {
        testing_logger::setup();
        let _ = AppError::internal(Action::new("save_settings"), "poisoned lock");

        testing_logger::validate(|captured| {
            assert_eq!(
                captured
                    .iter()
                    .filter(|l| l.level == log::Level::Error)
                    .count(),
                1,
                "internal state failure is ERROR"
            );
            assert_eq!(
                captured
                    .iter()
                    .filter(|l| l.level == log::Level::Warn)
                    .count(),
                0,
                "and is not ALSO recorded at WARN"
            );
        });
    }

    /// The record is redacted because `AppError`'s Display is, not because
    /// the constructor remembered to redact. Mutating the constructor to log
    /// the raw `reason` instead of `{self}` must fail this test.
    #[test]
    fn a_credential_in_the_reason_is_redacted_in_the_record() {
        testing_logger::setup();
        let _ = AppError::network(
            Action::new("analyze"),
            "connect failed for https://user:hunter2@example.com/v",
            true,
        );

        testing_logger::validate(|captured| {
            let body = captured.first().map_or("", |l| l.body.as_str());
            assert!(
                !body.contains("hunter2"),
                "credential reached the log: {body}"
            );
        });
    }

    /// A job id travels as a Subject, not as a fourth parameter.
    #[test]
    fn a_download_failure_names_its_job() {
        testing_logger::setup();
        let _ = AppError::download_failed(
            Action::with_subject("download", Subject::Job("job-7")),
            "disk full",
            false,
        );

        testing_logger::validate(|captured| {
            let body = captured.first().map_or("", |l| l.body.as_str());
            assert!(body.contains("job_id=job-7"), "names the job: {body}");
            // Pins the whole record, not just a substring: `contains` alone
            // cannot tell `action=X subject outcome=Y` from `action=X
            // outcome=Y subject`, which is exactly how a field-order defect
            // stayed green through a passing gate.
            assert_eq!(
                body,
                "action=download job_id=job-7 outcome=failed reason=Download job-7 failed: disk full"
            );
        });
    }

    /// A subject that is NOT `Subject::Job` still produces a valid record —
    /// `unwrap_or_default()` falls back to an empty job id rather than
    /// panicking or leaving the message malformed.
    #[test]
    fn a_download_failure_without_a_job_subject_has_an_empty_job_id() {
        testing_logger::setup();
        let err = AppError::download_failed(Action::new("download"), "disk full", false);

        testing_logger::validate(|captured| {
            let body = captured.first().map_or("", |l| l.body.as_str());
            assert_eq!(
                body,
                "action=download outcome=failed reason=Download  failed: disk full"
            );
        });
        assert!(matches!(
            err,
            AppError::DownloadFailed { ref job_id, .. } if job_id.is_empty()
        ));
    }

    /// `from_api` maps AND records, so a `?`-converted API error cannot reach
    /// the frontend unrecorded.
    #[test]
    fn from_api_records_the_mapped_outcome() {
        testing_logger::setup();
        let api = rdlp_api::RdlpApiError::InvalidInput {
            message: "bad url".to_owned(),
        };
        let err = AppError::from_api(Action::new("analyze"), &api);

        testing_logger::validate(|captured| {
            assert_eq!(captured.len(), 1, "exactly one record");
            let body = captured.first().map_or("", |l| l.body.as_str());
            assert!(body.contains("action=analyze"), "got: {body}");
            assert!(body.contains("outcome=failed"), "got: {body}");
        });
        assert!(matches!(err, AppError::InvalidInput { .. }));
    }
}
