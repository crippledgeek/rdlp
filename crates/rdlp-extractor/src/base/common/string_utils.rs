//! String and logging utility methods
//!
//! Helper methods for string truncation, HTML cleaning, URL sanitization,
//! and verbose logging output.

use log::{debug, trace};
use rdlp_core::ExtractionContext;

use super::BaseExtractor;
use super::selectors::HTML_TAG_PATTERN;

impl BaseExtractor {
    // ========================================================================
    // Logging Utilities
    // ========================================================================

    /// Log a message if verbose mode is enabled
    ///
    /// **Note**: Now uses the `log` crate. Set `RUST_LOG=debug` to see these messages.
    ///
    /// # Arguments
    /// * `ctx` - Extraction context
    /// * `prefix` - Log prefix (e.g., extractor name)
    /// * `message` - Message to log
    #[inline]
    pub(crate) fn log_if_verbose(_ctx: &ExtractionContext, prefix: &str, message: &str) {
        debug!("[{prefix}] {message}");
    }

    /// Log content with truncation if verbose mode is enabled
    ///
    /// **Note**: Now uses the `log` crate with `trace` level. Set `RUST_LOG=trace` to see these messages.
    ///
    /// # Arguments
    /// * `ctx` - Extraction context
    /// * `prefix` - Log prefix
    /// * `label` - Label for the content
    /// * `content` - Content to log (will be truncated)
    /// * `max_length` - Maximum characters to show
    pub(crate) fn log_content_if_verbose(
        _ctx: &ExtractionContext,
        prefix: &str,
        label: &str,
        content: &str,
        max_length: usize,
    ) {
        trace!("\n=== [{prefix}] {label} ===");
        trace!("{}", &content.chars().take(max_length).collect::<String>());
        if content.len() > max_length {
            trace!("... (truncated, {} total chars)", content.len());
        }
        trace!("=== END ===\n");
    }

    // ========================================================================
    // String Utilities
    // ========================================================================

    /// Truncate a string to a maximum length
    ///
    /// Truncates at character boundaries to avoid breaking Unicode.
    ///
    /// # Arguments
    /// * `s` - String to truncate
    /// * `max_len` - Maximum length in characters
    ///
    /// # Returns
    /// Truncated string (or original if already shorter)
    #[must_use]
    pub(crate) fn truncate_string(s: String, max_len: usize) -> String {
        if s.chars().count() <= max_len {
            s
        } else {
            s.chars().take(max_len).collect()
        }
    }

    /// Strip HTML tags from text and truncate to a maximum length.
    ///
    /// Useful for cleaning error messages and other text extracted from HTML.
    ///
    /// # Arguments
    /// * `text` - Text potentially containing HTML tags
    /// * `max_len` - Maximum length of the result (default 200 if None)
    ///
    /// # Returns
    /// Cleaned text with HTML tags removed and whitespace trimmed
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let clean = BaseExtractor::clean_html_tags("<p>Video <b>removed</b></p>", Some(200));
    /// assert_eq!(clean, "Video removed");
    /// ```
    #[must_use]
    pub(crate) fn clean_html_tags(text: &str, max_len: Option<usize>) -> String {
        let max = max_len.unwrap_or(200);
        HTML_TAG_PATTERN
            .replace_all(text.trim(), "")
            .trim()
            .chars()
            .take(max)
            .collect()
    }

    /// Sanitize a string for safe logging (redact sensitive data)
    ///
    /// **Note**: This function delegates to [`rdlp_security::sanitize_for_logging`].
    ///
    /// Redacts common sensitive patterns like tokens, keys, passwords.
    ///
    /// # Arguments
    /// * `s` - String to sanitize
    ///
    /// # Returns
    /// Sanitized string with sensitive data redacted
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn sanitize_for_logging(s: &str) -> String {
        rdlp_security::sanitize_for_logging(s)
    }
}
