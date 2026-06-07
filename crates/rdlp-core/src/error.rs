use rdlp_redact::RedactedUrlBuf;
use thiserror::Error;

/// Core error types for rdlp
#[derive(Error, Debug)]
pub enum RdlpError {
    /// HTTP/Network related errors
    #[error("Network error: {message}")]
    Network {
        /// Human-readable description of the error
        message: String,
        /// The URL that was being accessed, if applicable
        url: Option<RedactedUrlBuf>,
    },

    /// HTTP response error with status code
    #[error("HTTP error {status}: {reason}")]
    Http {
        /// HTTP status code
        status: u16,
        /// Human-readable reason
        reason: String,
    },

    /// Extraction errors
    #[error("Extraction failed: {message}")]
    Extraction {
        /// Human-readable description of the error
        message: String,
        /// The URL that was being extracted, if applicable
        url: Option<RedactedUrlBuf>,
    },

    /// No suitable extractor found for URL
    #[error("No extractor found for URL: {0}")]
    NoExtractor(String),

    /// Invalid URL format
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Download errors
    #[error("Download failed: {message}")]
    Download {
        /// Human-readable description of the error
        message: String,
        /// The URL that was being downloaded, if applicable
        url: Option<RedactedUrlBuf>,
    },

    /// Post-processing errors
    #[error("Post-processing failed: {0}")]
    PostProcess(String),

    /// `FFmpeg` related errors
    #[error("FFmpeg error: {0}")]
    FFmpeg(String),

    /// JavaScript execution errors
    #[error("JavaScript execution failed: {0}")]
    JavaScript(String),

    /// Cookie extraction errors
    #[error("Cookie extraction failed: {0}")]
    Cookie(String),

    /// Plugin loading errors
    #[error("Plugin error: {0}")]
    Plugin(String),

    /// Format selection errors
    #[error("Format selection error: {0}")]
    FormatSelection(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
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
    #[error("Unsupported: {0}")]
    Unsupported(String),

    /// Generic errors
    #[error("{0}")]
    Other(String),

    /// User cancelled the operation. Cooperative-cancellation typed signal.
    #[error("operation cancelled")]
    Cancelled,
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
