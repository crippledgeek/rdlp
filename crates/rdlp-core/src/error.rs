use rdlp_redact::RedactedUrlBuf;
use thiserror::Error;

/// Core error types for rdlp
#[derive(Error)]
pub enum RdlpError {
    /// HTTP/Network related errors
    #[error("Network error: {}", redact(message))]
    Network {
        /// Human-readable description of the error
        message: String,
        /// The URL that was being accessed, if applicable
        url: Option<RedactedUrlBuf>,
    },

    /// HTTP response error with status code
    #[error("HTTP error {status}: {}", redact(reason))]
    Http {
        /// HTTP status code
        status: u16,
        /// Human-readable reason
        reason: String,
    },

    /// Extraction errors
    #[error("Extraction failed: {}", redact(message))]
    Extraction {
        /// Human-readable description of the error
        message: String,
        /// The URL that was being extracted, if applicable
        url: Option<RedactedUrlBuf>,
    },

    /// No suitable extractor found for URL
    #[error("No extractor found for URL: {}", redact(_0))]
    NoExtractor(String),

    /// Invalid URL format
    #[error("Invalid URL: {}", redact(_0))]
    InvalidUrl(String),

    /// Download errors
    #[error("Download failed: {}", redact(message))]
    Download {
        /// Human-readable description of the error
        message: String,
        /// The URL that was being downloaded, if applicable
        url: Option<RedactedUrlBuf>,
    },

    /// Post-processing errors
    #[error("Post-processing failed: {}", redact(_0))]
    PostProcess(String),

    /// `FFmpeg` related errors
    #[error("FFmpeg error: {}", redact(_0))]
    FFmpeg(String),

    /// JavaScript execution errors
    #[error("JavaScript execution failed: {}", redact(_0))]
    JavaScript(String),

    /// Cookie extraction errors
    #[error("Cookie extraction failed: {}", redact(_0))]
    Cookie(String),

    /// Plugin loading errors
    #[error("Plugin error: {}", redact(_0))]
    Plugin(String),

    /// Format selection errors
    #[error("Format selection error: {}", redact(_0))]
    FormatSelection(String),

    /// Configuration errors
    #[error("Configuration error: {}", redact(_0))]
    Config(String),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// URL parsing errors
    #[error("URL parsing error: {0}")]
    UrlParse(#[from] url::ParseError),

    /// JSON parsing errors
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    /// Regex errors
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),

    /// Operation not supported by this component
    #[error("Unsupported: {}", redact(_0))]
    Unsupported(String),

    /// Generic errors
    #[error("{}", redact(_0))]
    Other(String),

    /// User cancelled the operation. Cooperative-cancellation typed signal.
    #[error("operation cancelled")]
    Cancelled,
}

/// Redact free text on its way to an operator.
///
/// These messages are assembled by `format!("…: {e}")` at hundreds of call
/// sites, and an error stringified that way can carry a URL: `wreq::Error`'s
/// Display prints the request URI verbatim, credentials included. The URL then
/// sits inside an opaque string that no URL-shaped gate can see — which is why
/// `scripts/check-url-redaction.sh` passes over call sites that leak, even
/// inside the crates it scans.
///
/// Redacting where the text is rendered rather than where it is built covers
/// every variant and every call site at once, including ones added later. The
/// cost is 22 regex scans and an allocation or two, on an error path only.
///
/// It filters free text, not URLs specifically, so a message that merely
/// starts with `key=` or `code=` is redacted too. That is a deliberate trade:
/// over-redacting a diagnostic beats leaking a credential.
use rdlp_redact::redact_str as redact;

/// Debug redacts the free text while keeping the structure.
///
/// The derived Debug printed each field verbatim, so `{e:?}` — what a panic
/// from `unwrap`/`expect` prints, and a good deal of logging — leaked
/// precisely what Display redacts. Delegating Debug to Display is not the fix:
/// it would also drop the `url` field, which is separately useful and already
/// redacted by `RedactedUrlBuf`'s own Debug.
///
/// So this mirrors the derive, with the free-text fields passed through
/// `redact`. Adding a variant means adding an arm — the compiler's
/// exhaustiveness check enforces that, which is why this is a `match` rather
/// than a catch-all.
impl std::fmt::Debug for RdlpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// One arm's worth: a struct variant carrying `message` and `url`.
        macro_rules! with_url {
            ($name:literal, $message:expr, $url:expr) => {
                f.debug_struct($name)
                    .field("message", &redact($message))
                    .field("url", $url)
                    .finish()
            };
        }
        /// One arm's worth: a newtype variant carrying free text.
        macro_rules! text {
            ($name:literal, $text:expr) => {
                f.debug_tuple($name).field(&redact($text)).finish()
            };
        }

        match self {
            Self::Network { message, url } => with_url!("Network", message, url),
            Self::Extraction { message, url } => with_url!("Extraction", message, url),
            Self::Download { message, url } => with_url!("Download", message, url),
            Self::Http { status, reason } => f
                .debug_struct("Http")
                .field("status", status)
                .field("reason", &redact(reason))
                .finish(),
            Self::NoExtractor(t) => text!("NoExtractor", t),
            Self::InvalidUrl(t) => text!("InvalidUrl", t),
            Self::PostProcess(t) => text!("PostProcess", t),
            Self::FFmpeg(t) => text!("FFmpeg", t),
            Self::JavaScript(t) => text!("JavaScript", t),
            Self::Cookie(t) => text!("Cookie", t),
            Self::Plugin(t) => text!("Plugin", t),
            Self::FormatSelection(t) => text!("FormatSelection", t),
            Self::Config(t) => text!("Config", t),
            Self::Unsupported(t) => text!("Unsupported", t),
            Self::Other(t) => text!("Other", t),
            // Typed sources: their Display is the crate's own and carries no
            // URL we assembled, so they keep the derive's shape.
            Self::Io(e) => f.debug_tuple("Io").field(e).finish(),
            Self::UrlParse(e) => f.debug_tuple("UrlParse").field(e).finish(),
            Self::Json(e) => f.debug_tuple("Json").field(e).finish(),
            Self::Regex(e) => f.debug_tuple("Regex").field(e).finish(),
            Self::Cancelled => f.write_str("Cancelled"),
        }
    }
}

impl RdlpError {
    /// Create an `Extraction` error with a URL.
    pub fn extraction(message: impl Into<String>, url: &str) -> Self {
        Self::Extraction {
            message: message.into(),
            url: Some(RedactedUrlBuf::from(url)),
        }
    }

    /// Create a `Network` error with a URL.
    pub fn network(message: impl Into<String>, url: &str) -> Self {
        Self::Network {
            message: message.into(),
            url: Some(RedactedUrlBuf::from(url)),
        }
    }

    /// Create a `Download` error with a URL — symmetry with `extraction`
    /// and `network`. Avoids `RdlpError::Download { … }` struct-literal
    /// noise at the dozen-or-so call sites that hit this variant.
    pub fn download(message: impl Into<String>, url: &str) -> Self {
        Self::Download {
            message: message.into(),
            url: Some(RedactedUrlBuf::from(url)),
        }
    }
}

/// Result type alias for rdlp operations
pub type Result<T> = std::result::Result<T, RdlpError>;

/// Helper function to check if an HTTP response is successful
///
/// Returns `Ok(())` if the response has a 2xx status code, otherwise returns
/// an `Err(RdlpError::Network)` with details about the HTTP error.
///
/// # Errors
///
/// Returns [`RdlpError::Http`] if the response status code is not 2xx.
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_core::check_http_response;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let client = wreq::Client::new();
///     let url = "https://example.com";
///     let response = client.get(url).send().await?;
///     check_http_response(&response)?;
///     Ok(())
/// }
/// ```
pub fn check_http_response(response: &wreq::Response) -> Result<()> {
    let status = response.status();
    if !status.is_success() {
        return Err(RdlpError::Http {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or("Unknown").to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_error_with_url() {
        let err = RdlpError::Network {
            message: "timeout".into(),
            url: Some(RedactedUrlBuf::from("https://example.com")),
        };
        assert!(err.to_string().contains("timeout"));
        if let RdlpError::Network { url, .. } = &err {
            assert_eq!(
                url.as_ref().map(rdlp_redact::RedactedUrlBuf::expose),
                Some("https://example.com")
            );
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_network_error_without_url() {
        let err = RdlpError::Network {
            message: "dns failed".into(),
            url: None,
        };
        assert!(err.to_string().contains("dns failed"));
        if let RdlpError::Network { url, .. } = &err {
            assert!(url.is_none());
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_extraction_error_with_url() {
        let err = RdlpError::Extraction {
            message: "no formats".into(),
            url: Some(RedactedUrlBuf::from("https://example.com/video")),
        };
        assert!(err.to_string().contains("no formats"));
        if let RdlpError::Extraction { url, .. } = &err {
            assert_eq!(
                url.as_ref().map(rdlp_redact::RedactedUrlBuf::expose),
                Some("https://example.com/video")
            );
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn test_download_error_with_url() {
        let err = RdlpError::Download {
            message: "chunk failed".into(),
            url: Some(RedactedUrlBuf::from("https://cdn.example.com/seg1.ts")),
        };
        assert!(err.to_string().contains("chunk failed"));
        if let RdlpError::Download { url, .. } = &err {
            assert_eq!(
                url.as_ref().map(rdlp_redact::RedactedUrlBuf::expose),
                Some("https://cdn.example.com/seg1.ts")
            );
        } else {
            panic!("wrong variant");
        }
    }
}

#[cfg(test)]
mod redact_tests {
    use super::*;

    #[test]
    fn network_error_debug_redacts_url() {
        let err = RdlpError::network("boom", "https://cdn/s.m4s?X-Amz-Signature=DEADBEEF");
        let dbg = format!("{err:?}");
        assert!(
            !dbg.contains("DEADBEEF"),
            "raw signature must not appear in Debug: {dbg}"
        );
        assert!(
            dbg.contains("X-Amz-Signature=***"),
            "redacted form expected: {dbg}"
        );
        // Display renders message only (unchanged) — must NOT leak the url at all.
        let disp = format!("{err}");
        assert_eq!(disp, "Network error: boom", "Display is message-only");
        assert!(
            !disp.contains("DEADBEEF") && !disp.contains("cdn"),
            "url absent from Display: {disp}"
        );
    }

    #[test]
    fn network_error_message_should_be_built_with_redaction() {
        // Guards the #328 seam: a message built from a presigned URL must redact it.
        // Construct the error the way the downloader does (message via RedactedUrl inline).
        let raw = "https://cdn/s.m4s?X-Amz-Signature=DEADBEEF";
        let safe = rdlp_redact::redact_str(raw);
        let err = RdlpError::Network {
            message: format!("Failed to read chunk body from {safe}: timeout"),
            url: Some(rdlp_redact::RedactedUrlBuf::from(raw)),
        };
        let rendered = format!("{err}"); // Display = "Network error: {message}"
        assert!(
            !rendered.contains("DEADBEEF"),
            "raw signature must not appear in message: {rendered}"
        );
        assert!(
            rendered.contains("X-Amz-Signature=***"),
            "redacted form expected in message: {rendered}"
        );
    }
    /// A URL with credentials, arriving the way it really does: inside a
    /// message some call site built with `format!("…: {e}")` over an error
    /// whose Display printed the request URI verbatim.
    const LEAKY: &str = "Failed to read response body: \
error following redirect for uri (https://admin:hunter2@cdn.example.com/v.mp4)";

    #[test]
    fn display_redacts_credentials_carried_in_a_message() {
        let e = RdlpError::Network {
            message: LEAKY.to_string(),
            url: None,
        };
        let shown = e.to_string();
        assert!(!shown.contains("hunter2"), "password leaked: {shown}");
        assert!(!shown.contains("admin"), "username leaked: {shown}");
        // Still useful: the host survives, so the message still says where.
        assert!(shown.contains("cdn.example.com"), "over-redacted: {shown}");
    }

    #[test]
    fn debug_redacts_what_display_redacts() {
        // The derived Debug printed fields verbatim, so `{e:?}` — what a panic
        // from unwrap/expect prints, and much logging — leaked exactly what
        // Display was stripping.
        let e = RdlpError::Extraction {
            message: LEAKY.to_string(),
            url: None,
        };
        let shown = format!("{e:?}");
        assert!(
            !shown.contains("hunter2"),
            "password leaked via Debug: {shown}"
        );
    }

    #[test]
    fn every_free_text_variant_is_redacted() {
        // Redaction is per-variant in the `#[error]` attributes, so one
        // covered variant says nothing about the rest.
        let variants: Vec<RdlpError> = vec![
            RdlpError::Network {
                message: LEAKY.into(),
                url: None,
            },
            RdlpError::Extraction {
                message: LEAKY.into(),
                url: None,
            },
            RdlpError::Download {
                message: LEAKY.into(),
                url: None,
            },
            RdlpError::Http {
                status: 500,
                reason: LEAKY.into(),
            },
            RdlpError::NoExtractor(LEAKY.into()),
            RdlpError::InvalidUrl(LEAKY.into()),
            RdlpError::PostProcess(LEAKY.into()),
            RdlpError::FFmpeg(LEAKY.into()),
            RdlpError::JavaScript(LEAKY.into()),
            RdlpError::Cookie(LEAKY.into()),
            RdlpError::Plugin(LEAKY.into()),
            RdlpError::FormatSelection(LEAKY.into()),
            RdlpError::Config(LEAKY.into()),
            RdlpError::Unsupported(LEAKY.into()),
            RdlpError::Other(LEAKY.into()),
        ];
        for e in variants {
            let d = e.to_string();
            assert!(!d.contains("hunter2"), "Display leaked: {d}");
            let dbg = format!("{e:?}");
            assert!(!dbg.contains("hunter2"), "Debug leaked: {dbg}");
        }
    }
}

#[cfg(test)]
mod cancelled_tests {
    use super::*;

    #[test]
    fn cancelled_variant_displays_lowercase() {
        let e = RdlpError::Cancelled;
        let s = format!("{e}");
        assert!(s.to_lowercase().contains("cancelled"), "got: {s}");
    }
}
