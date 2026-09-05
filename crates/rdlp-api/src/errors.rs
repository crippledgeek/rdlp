//! Stable error types for the rdlp public API.
//!
//! [`RdlpApiError`] maps internal error types to a stable, frontend-friendly
//! enum with human-readable messages and retryability information.

use crate::orchestrator::OrchestratorError;
use rdlp_core::RdlpError;
use rdlp_redact::RedactedUrlBuf;
use std::borrow::Cow;
use thiserror::Error;

/// Stable error enum for the public API.
///
/// Internal implementation details (regex errors, JSON parse errors) are wrapped
/// into the appropriate high-level variant with a user-friendly message.
#[derive(Clone, Error)]
pub enum RdlpApiError {
    /// Invalid URL or request parameters.
    #[error("Invalid input: {}", redact(message))]
    InvalidInput {
        /// Description of what's invalid.
        message: String,
    },

    /// No extractor found for the given URL.
    ///
    /// `url` is stored as [`RedactedUrlBuf`] so that `#[error("…{url}")]`
    /// Display and `user_message()` automatically strip credentials — the type
    /// system enforces this at every construction site.
    #[error("Unsupported URL: {url}")]
    UnsupportedUrl {
        /// The URL that no extractor was found for. Credentials are redacted
        /// in all Display / Debug output.
        url: RedactedUrlBuf,
    },

    /// Extraction failed (metadata retrieval).
    ///
    /// `source_url` is stored as [`RedactedUrlBuf`] so that
    /// `#[error("…{source_url}")]` Display automatically strips credentials —
    /// the type system enforces this at every construction site.
    #[error("Extraction failed for {source_url}: {}", redact(message))]
    ExtractError {
        /// What went wrong.
        message: String,
        /// The URL being extracted. Credentials are redacted in all
        /// Display / Debug output. Use `RedactedUrlBuf::from("")`
        /// when no URL is available.
        source_url: RedactedUrlBuf,
    },

    /// Network or HTTP failure.
    #[error("Network error: {}", redact(message))]
    NetworkError {
        /// Description of the failure.
        message: String,
        /// HTTP status code, if applicable.
        status: Option<u16>,
    },

    /// Filesystem I/O error.
    #[error("I/O error: {}", redact(message))]
    IoError {
        /// Description of the I/O failure.
        message: String,
    },

    /// `FFmpeg` processing failed.
    #[error("FFmpeg error: {}", redact(message))]
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
    #[error("Soft error: {}", redact(message))]
    Soft {
        /// Warning message.
        message: String,
    },

    /// Builder misconfiguration.
    #[error("Builder error: {}", redact(message))]
    BuilderError {
        /// What's wrong with the builder configuration.
        message: String,
    },
}

/// Redact free text on its way to an operator. See `rdlp_core::error`'s
/// counterpart — these messages are carried over from `RdlpError` or built the
/// same way, so they can hold a URL that arrived inside a stringified error.
fn redact(text: &str) -> String {
    rdlp_redact::redact_str(text)
}

/// Debug redacts the free text while keeping the structure.
///
/// `{e:?}` must not leak what `{e}` strips. Delegating to Display is not the
/// fix — it would drop fields Display omits, `NetworkError::status` among
/// them — so this mirrors the derive with `message` passed through `redact`.
/// The `RedactedUrlBuf` fields redact themselves. See
/// `rdlp_core::error::RdlpError`'s Debug, which does the same for the same
/// reason.
impl std::fmt::Debug for RdlpApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// One arm's worth: a struct variant whose only free text is `message`.
        macro_rules! msg_only {
            ($name:literal, $message:expr) => {
                f.debug_struct($name)
                    .field("message", &redact($message))
                    .finish()
            };
        }

        match self {
            Self::InvalidInput { message } => msg_only!("InvalidInput", message),
            Self::IoError { message } => msg_only!("IoError", message),
            Self::FfmpegError { message } => msg_only!("FfmpegError", message),
            Self::Soft { message } => msg_only!("Soft", message),
            Self::BuilderError { message } => msg_only!("BuilderError", message),
            Self::UnsupportedUrl { url } => {
                f.debug_struct("UnsupportedUrl").field("url", url).finish()
            }
            Self::ExtractError {
                message,
                source_url,
            } => f
                .debug_struct("ExtractError")
                .field("message", &redact(message))
                .field("source_url", source_url)
                .finish(),
            Self::NetworkError { message, status } => f
                .debug_struct("NetworkError")
                .field("message", &redact(message))
                .field("status", status)
                .finish(),
            Self::UnsupportedPlatform { feature } => f
                .debug_struct("UnsupportedPlatform")
                .field("feature", feature)
                .finish(),
            Self::UserCancelled => f.write_str("UserCancelled"),
        }
    }
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
    /// **Note:** For [`RdlpError::Extraction`], the source URL IS propagated
    /// into [`RdlpApiError::ExtractError::source_url`] via the redacted
    /// [`rdlp_redact::RedactedUrlBuf`] Display (credentials stripped). For
    /// [`RdlpError::Network`] and [`RdlpError::Download`], the URL is not
    /// surfaced in the API error — call sites that need it should construct
    /// `RdlpApiError::NetworkError` directly.
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
                source_url: url.unwrap_or_else(|| RedactedUrlBuf::from("")),
            },
            RdlpError::NoExtractor(url) => Self::UnsupportedUrl {
                url: RedactedUrlBuf::from(url),
            },
            RdlpError::InvalidUrl(msg) | RdlpError::FormatSelection(msg) => {
                Self::InvalidInput { message: msg }
            }
            RdlpError::PostProcess(msg) | RdlpError::FFmpeg(msg) => {
                Self::FfmpegError { message: msg }
            }
            RdlpError::JavaScript(msg) => Self::ExtractError {
                message: format!("JavaScript error: {msg}"),
                source_url: RedactedUrlBuf::from(""),
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
                source_url: RedactedUrlBuf::from(""),
            },
            RdlpError::Regex(err) => Self::ExtractError {
                message: format!("Regex error: {err}"),
                source_url: RedactedUrlBuf::from(""),
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
            OrchestratorError::PostProcessingFailed(msg) => Self::FfmpegError { message: msg },
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
                source_url: RedactedUrlBuf::from(""),
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
                source_url: RedactedUrlBuf::from(""),
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
                // `url` is now `RedactedUrlBuf`; use `.expose()` to check the raw value.
                assert_eq!(url.expose(), "http://unknown.com");
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
                // Use `.expose()` to check the raw (unredacted) propagated value.
                assert_eq!(source_url.expose(), "https://example.com/video");
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
                assert!(source_url.expose().is_empty());
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
    fn extraction_error_source_url_is_redacted() {
        // Guards the From<RdlpError> construction path: the `RedactedUrlBuf`
        // moved through from `RdlpError::Extraction.url` redacts at Display.
        let err = RdlpError::extraction("nope", "https://x/v?token=SECRET");
        let api: RdlpApiError = err.into();
        if let RdlpApiError::ExtractError { source_url, .. } = api {
            let displayed = source_url.to_string();
            assert!(
                !displayed.contains("SECRET"),
                "raw token must not leak: {displayed}"
            );
            assert!(
                displayed.contains("token=***"),
                "redacted form: {displayed}"
            );
        } else {
            panic!("expected ExtractError");
        }
    }

    #[test]
    fn extracterror_display_redacts_when_constructed_directly() {
        // Failing-first evidence: against the old `source_url: String` field,
        // `RedactedUrlBuf::from(...)` is not assignable — type-mismatch compile
        // error.  With the new `RedactedUrlBuf` field the type accepts it and
        // Display redacts automatically, so a caller that bypasses the
        // `From<RdlpError>` path cannot accidentally store a raw credential.
        let err = RdlpApiError::ExtractError {
            message: "direct construction".into(),
            source_url: RedactedUrlBuf::from("https://x.com/v?token=SECRET"),
        };
        let displayed = err.to_string();
        assert!(
            !displayed.contains("SECRET"),
            "raw token must not leak via direct construction: {displayed}"
        );
        assert!(
            displayed.contains("token=***"),
            "redacted form expected in direct construction: {displayed}"
        );
    }

    // ── Phase 2 redaction tests ────────────────────────────────────────────────
    // Failing-first: these tests reference `RedactedUrlBuf` in the `UnsupportedUrl`
    // field, which is still `String` until the Phase 2 type change below. The first
    // two tests (unsupported_url_*) fail with a type-mismatch compile error before
    // the change; `from_rdlp_no_extractor_redacts` fails at runtime because the raw
    // `String` still contains "SECRET"; `no_downloader_invalid_input_redacts` already
    // passes (Phase 1 made NoDownloader.url `RedactedUrlBuf`).

    #[test]
    fn unsupported_url_display_redacts() {
        use rdlp_redact::RedactedUrlBuf;
        let err = RdlpApiError::UnsupportedUrl {
            url: RedactedUrlBuf::from("https://x.com/v?token=SECRET"),
        };
        let s = err.to_string();
        assert!(!s.contains("SECRET"), "raw token must not leak: {s}");
        assert!(s.contains("token=***"), "redacted form expected: {s}");
    }

    #[test]
    fn unsupported_url_user_message_redacts() {
        use rdlp_redact::RedactedUrlBuf;
        let err = RdlpApiError::UnsupportedUrl {
            url: RedactedUrlBuf::from("https://x.com/v?token=SECRET"),
        };
        let msg = err.user_message();
        assert!(!msg.contains("SECRET"), "raw token must not leak: {msg}");
        assert!(msg.contains("token=***"), "redacted form expected: {msg}");
    }

    #[test]
    fn from_rdlp_no_extractor_redacts() {
        let api: RdlpApiError =
            RdlpError::NoExtractor("https://x.com/v?token=SECRET".to_owned()).into();
        match api {
            RdlpApiError::UnsupportedUrl { url } => {
                let s = url.to_string();
                assert!(!s.contains("SECRET"), "raw token must not leak: {s}");
                assert!(s.contains("token=***"), "redacted form expected: {s}");
            }
            other => panic!("expected UnsupportedUrl, got: {other:?}"),
        }
    }

    #[test]
    fn no_downloader_invalid_input_redacts() {
        use crate::orchestrator::OrchestratorError;
        use rdlp_redact::RedactedUrlBuf;
        let api: RdlpApiError = OrchestratorError::NoDownloader {
            url: RedactedUrlBuf::from("https://cdn.example.com/seg.ts?token=SECRET"),
        }
        .into();
        match api {
            RdlpApiError::InvalidInput { message } => {
                assert!(
                    !message.contains("SECRET"),
                    "raw token must not leak: {message}"
                );
                assert!(
                    message.contains("token=***"),
                    "redacted form expected: {message}"
                );
            }
            other => panic!("expected InvalidInput, got: {other:?}"),
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

    const LEAKY_API: &str = "Failed for uri (https://admin:hunter2@cdn.example.com/v.mp4)";

    #[test]
    fn api_error_display_and_debug_redact_credentials() {
        let e = RdlpApiError::NetworkError {
            message: LEAKY_API.to_string(),
            status: None,
        };
        let shown = e.to_string();
        let dbg = format!("{e:?}");
        assert!(!shown.contains("hunter2"), "Display leaked: {shown}");
        assert!(!dbg.contains("hunter2"), "Debug leaked: {dbg}");
        assert!(shown.contains("cdn.example.com"), "over-redacted: {shown}");
    }

    #[test]
    fn credentials_do_not_survive_the_conversion_from_rdlp_error() {
        // The path a real leak takes: built in rdlp-core, carried across.
        let converted: RdlpApiError = RdlpError::Network {
            message: LEAKY_API.to_string(),
            url: None,
        }
        .into();
        let shown = converted.to_string();
        assert!(
            !shown.contains("hunter2"),
            "leaked across the boundary: {shown}"
        );
    }
}
