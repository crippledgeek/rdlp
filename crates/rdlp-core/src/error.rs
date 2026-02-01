use thiserror::Error;

/// Core error types for rdlp
#[derive(Error, Debug)]
pub enum RdlpError {
    /// HTTP/Network related errors
    #[error("Network error: {0}")]
    Network(String),

    /// HTTP response error with status code
    #[error("HTTP error {status}: {reason}")]
    Http {
        /// HTTP status code
        status: u16,
        /// Human-readable reason
        reason: String,
    },

    /// Extraction errors
    #[error("Extraction failed: {0}")]
    Extraction(String),

    /// No suitable extractor found for URL
    #[error("No extractor found for URL: {0}")]
    NoExtractor(String),

    /// Invalid URL format
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Download errors
    #[error("Download failed: {0}")]
    Download(String),

    /// Post-processing errors
    #[error("Post-processing failed: {0}")]
    PostProcess(String),

    /// FFmpeg related errors
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

    /// Generic errors
    #[error("{0}")]
    Other(String),
}

/// Result type alias for rdlp operations
pub type Result<T> = std::result::Result<T, RdlpError>;

/// Helper function to check if an HTTP response is successful
///
/// Returns `Ok(())` if the response has a 2xx status code, otherwise returns
/// an `Err(RdlpError::Network)` with details about the HTTP error.
///
/// # Example
///
/// ```rust,no_run
/// use rdlp_core::check_http_response;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let client = reqwest::Client::new();
///     let url = "https://example.com";
///     let response = client.get(url).send().await?;
///     check_http_response(&response)?;
///     Ok(())
/// }
/// ```
pub fn check_http_response(response: &reqwest::Response) -> Result<()> {
    if !response.status().is_success() {
        return Err(RdlpError::Http {
            status: response.status().as_u16(),
            reason: response
                .status()
                .canonical_reason()
                .unwrap_or("Unknown")
                .to_string(),
        });
    }
    Ok(())
}
