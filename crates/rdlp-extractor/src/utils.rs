//! Utility functions for extractors
//!
//! This module provides common helper functions used across multiple extractors
//! to reduce code duplication and maintain consistency.
//!
//! # Overview
//!
//! These utilities complement the `base::common::BaseExtractor` by providing
//! standalone helper functions that don't require the extraction context.
//!
//! ## Functions
//!
//! - **Debug Output**: `debug_print_webpage_sample`, `debug_print_json`
//! - **String Processing**: `clean_html_text`, `decode_html_entities`
//! - **URL Handling**: `extract_extension_from_url`, `make_absolute_url`
//! - **Format Helpers**: `format_filesize`, `format_duration`

use once_cell::sync::Lazy;
use regex::Regex;

// ============================================================================
// Static Patterns for Utility Functions
// ============================================================================

/// Pattern for HTML entity references (e.g., &amp; &#39; &#x27;)
static HTML_ENTITY_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"&(#?[a-zA-Z0-9]+);").expect("Valid HTML entity pattern")
});

/// Pattern for whitespace normalization
static WHITESPACE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\s+").expect("Valid whitespace pattern")
});

// ============================================================================
// Debug Output Functions
// ============================================================================

/// Print a sample of webpage content for debugging purposes
///
/// This function prints the first N characters of a webpage to stderr
/// when verbose mode is enabled. Useful for debugging extraction issues.
///
/// # Arguments
/// * `webpage` - The full webpage content as a string
/// * `sample_size` - Number of characters to include in the sample (default: 5000)
///
/// # Example
///
/// ```rust,ignore
/// use rdlp_extractor::utils::debug_print_webpage_sample;
///
/// if verbose {
///     debug_print_webpage_sample(&webpage, 5000);
/// }
/// ```
///
/// # Output Format
///
/// ```text
/// === WEBPAGE SAMPLE (first 5000 chars) ===
/// <!DOCTYPE html>
/// <html>
/// ...
/// === END SAMPLE ===
/// ```
pub fn debug_print_webpage_sample(webpage: &str, sample_size: usize) {
    eprintln!("\n=== WEBPAGE SAMPLE (first {sample_size} chars) ===");
    eprintln!("{}", &webpage.chars().take(sample_size).collect::<String>());
    eprintln!("=== END SAMPLE ===\n");
}

/// Print formatted JSON for debugging purposes
///
/// # Arguments
/// * `label` - Label for the debug output
/// * `json` - JSON value to print
/// * `max_length` - Maximum characters to print (0 for unlimited)
pub fn debug_print_json(label: &str, json: &serde_json::Value, max_length: usize) {
    eprintln!("\n=== {label} ===");
    if let Ok(formatted) = serde_json::to_string_pretty(json) {
        if max_length > 0 && formatted.len() > max_length {
            eprintln!("{}", &formatted.chars().take(max_length).collect::<String>());
            eprintln!("... (truncated, {} total chars)", formatted.len());
        } else {
            eprintln!("{formatted}");
        }
    } else {
        eprintln!("{json:?}");
    }
    eprintln!("=== END ===\n");
}

// ============================================================================
// String Processing Functions
// ============================================================================

/// Clean HTML text by removing tags and normalizing whitespace
///
/// # Arguments
/// * `text` - Text that may contain HTML tags
///
/// # Returns
/// Cleaned text with HTML tags removed and whitespace normalized
///
/// # Example
///
/// ```rust
/// use rdlp_extractor::utils::clean_html_text;
///
/// let cleaned = clean_html_text("<p>Hello  <b>World</b>!</p>");
/// assert_eq!(cleaned, "Hello World!");
/// ```
pub fn clean_html_text(text: &str) -> String {
    // Simple HTML tag removal (for more complex cases, use scraper)
    let without_tags = text
        .split('<')
        .map(|s| {
            if let Some(idx) = s.find('>') {
                &s[idx + 1..]
            } else {
                s
            }
        })
        .collect::<String>();

    // Normalize whitespace
    WHITESPACE_PATTERN
        .replace_all(&without_tags, " ")
        .trim()
        .to_string()
}

/// Decode common HTML entities
///
/// Handles:
/// - Named entities: `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&nbsp;`
/// - Numeric entities: `&#39;`, `&#34;`
/// - Hex entities: `&#x27;`, `&#x22;`
///
/// # Arguments
/// * `text` - Text containing HTML entities
///
/// # Returns
/// Text with entities decoded
///
/// # Example
///
/// ```rust
/// use rdlp_extractor::utils::decode_html_entities;
///
/// let decoded = decode_html_entities("Tom &amp; Jerry");
/// assert_eq!(decoded, "Tom & Jerry");
/// ```
pub fn decode_html_entities(text: &str) -> String {
    let mut result = text.to_string();

    // Common named entities
    result = result.replace("&amp;", "&");
    result = result.replace("&lt;", "<");
    result = result.replace("&gt;", ">");
    result = result.replace("&quot;", "\"");
    result = result.replace("&apos;", "'");
    result = result.replace("&#39;", "'");
    result = result.replace("&#34;", "\"");
    result = result.replace("&nbsp;", " ");

    // Handle numeric/hex entities
    HTML_ENTITY_PATTERN
        .replace_all(&result, |caps: &regex::Captures| {
            let entity = &caps[1];
            if let Some(stripped) = entity.strip_prefix('#') {
                // Numeric entity
                let code = if let Some(hex) = stripped.strip_prefix('x').or_else(|| stripped.strip_prefix('X')) {
                    // Hex entity
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    // Decimal entity
                    stripped.parse::<u32>().ok()
                };

                if let Some(code) = code {
                    if let Some(ch) = char::from_u32(code) {
                        return ch.to_string();
                    }
                }
            }
            // Return original if can't decode
            caps[0].to_string()
        })
        .to_string()
}

// ============================================================================
// URL Handling Functions
// ============================================================================

/// Extract file extension from a URL
///
/// # Arguments
/// * `url` - URL to extract extension from
///
/// # Returns
/// File extension (without dot) if found, `None` otherwise
///
/// # Example
///
/// ```rust
/// use rdlp_extractor::utils::extract_extension_from_url;
///
/// assert_eq!(extract_extension_from_url("https://example.com/video.mp4"), Some("mp4".to_string()));
/// assert_eq!(extract_extension_from_url("https://example.com/video.mp4?token=abc"), Some("mp4".to_string()));
/// assert_eq!(extract_extension_from_url("https://example.com/video"), None);
/// ```
pub fn extract_extension_from_url(url: &str) -> Option<String> {
    // Parse URL to get path
    let parsed = url::Url::parse(url).ok()?;
    let path = parsed.path();

    // Get last segment
    let last_segment = path.split('/').next_back()?;

    // Find extension (after last dot)
    let dot_pos = last_segment.rfind('.')?;
    let ext = &last_segment[dot_pos + 1..];

    // Filter out query strings that might be attached
    let ext_clean = ext.split('?').next().unwrap_or(ext);

    if ext_clean.is_empty() || ext_clean.len() > 10 {
        None
    } else {
        Some(ext_clean.to_lowercase())
    }
}

/// Make a relative URL absolute using a base URL
///
/// # Arguments
/// * `base_url` - Base URL to resolve against
/// * `relative_url` - Relative URL to make absolute
///
/// # Returns
/// Absolute URL if resolution succeeds, original URL otherwise
///
/// # Example
///
/// ```rust
/// use rdlp_extractor::utils::make_absolute_url;
///
/// let abs = make_absolute_url(
///     "https://example.com/video/",
///     "segment001.ts"
/// );
/// assert_eq!(abs, "https://example.com/video/segment001.ts");
/// ```
pub fn make_absolute_url(base_url: &str, relative_url: &str) -> String {
    // If already absolute, return as-is
    if relative_url.starts_with("http://") || relative_url.starts_with("https://") {
        return relative_url.to_string();
    }

    // Try to join with base
    if let Ok(base) = url::Url::parse(base_url) {
        if let Ok(absolute) = base.join(relative_url) {
            return absolute.to_string();
        }
    }

    // Fallback: return original
    relative_url.to_string()
}

// ============================================================================
// Format Helper Functions
// ============================================================================

/// Format file size in human-readable format
///
/// Uses binary units (KiB, MiB, GiB) with 1024 base.
///
/// # Arguments
/// * `bytes` - Size in bytes
///
/// # Returns
/// Formatted string (e.g., "1.5 GiB", "256 MiB", "128 KiB")
///
/// # Example
///
/// ```rust
/// use rdlp_extractor::utils::format_filesize;
///
/// assert_eq!(format_filesize(1024), "1.0 KiB");
/// assert_eq!(format_filesize(1024 * 1024), "1.0 MiB");
/// assert_eq!(format_filesize(1536 * 1024 * 1024), "1.5 GiB");
/// ```
pub fn format_filesize(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format file size using decimal units (KB, MB, GB)
///
/// Uses decimal units (KB, MB, GB) with 1000 base.
///
/// # Arguments
/// * `bytes` - Size in bytes
///
/// # Returns
/// Formatted string (e.g., "1.5 GB", "256 MB", "128 KB")
pub fn format_filesize_decimal(bytes: u64) -> String {
    const KB: u64 = 1000;
    const MB: u64 = KB * 1000;
    const GB: u64 = MB * 1000;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format duration in human-readable format
///
/// # Arguments
/// * `seconds` - Duration in seconds
///
/// # Returns
/// Formatted string (e.g., "1:30:45", "5:30", "0:45")
///
/// # Example
///
/// ```rust
/// use rdlp_extractor::utils::format_duration;
///
/// assert_eq!(format_duration(45.0), "0:45");
/// assert_eq!(format_duration(330.0), "5:30");
/// assert_eq!(format_duration(5445.0), "1:30:45");
/// ```
pub fn format_duration(seconds: f64) -> String {
    let total_secs = seconds as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Debug Output Tests
    // ========================================================================

    #[test]
    fn test_debug_print_webpage_sample() {
        let webpage = "<!DOCTYPE html><html><body>Test content</body></html>";

        // Test with sample size smaller than content
        debug_print_webpage_sample(webpage, 20);

        // Test with sample size larger than content
        debug_print_webpage_sample(webpage, 1000);

        // Should not panic with empty string
        debug_print_webpage_sample("", 100);
    }

    #[test]
    fn test_unicode_handling() {
        let webpage = "Hello 世界 🎉 Test";

        // Should handle Unicode characters correctly
        debug_print_webpage_sample(webpage, 10);
    }

    // ========================================================================
    // String Processing Tests
    // ========================================================================

    #[test]
    fn test_clean_html_text() {
        assert_eq!(
            clean_html_text("<p>Hello  <b>World</b>!</p>"),
            "Hello World!"
        );
        assert_eq!(
            clean_html_text("  Multiple   spaces  here  "),
            "Multiple spaces here"
        );
        assert_eq!(clean_html_text("No tags"), "No tags");
    }

    #[test]
    fn test_decode_html_entities() {
        assert_eq!(decode_html_entities("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(decode_html_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_html_entities("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(decode_html_entities("it&#39;s"), "it's");
        assert_eq!(decode_html_entities("&#60;"), "<");
        assert_eq!(decode_html_entities("&#x3C;"), "<");
    }

    // ========================================================================
    // URL Handling Tests
    // ========================================================================

    #[test]
    fn test_extract_extension_from_url() {
        assert_eq!(
            extract_extension_from_url("https://example.com/video.mp4"),
            Some("mp4".to_string())
        );
        assert_eq!(
            extract_extension_from_url("https://example.com/video.mp4?token=abc"),
            Some("mp4".to_string())
        );
        assert_eq!(
            extract_extension_from_url("https://example.com/video.MP4"),
            Some("mp4".to_string())
        );
        assert_eq!(
            extract_extension_from_url("https://example.com/video"),
            None
        );
        assert_eq!(
            extract_extension_from_url("https://example.com/"),
            None
        );
    }

    #[test]
    fn test_make_absolute_url() {
        assert_eq!(
            make_absolute_url("https://example.com/video/", "segment001.ts"),
            "https://example.com/video/segment001.ts"
        );
        assert_eq!(
            make_absolute_url("https://example.com/video/playlist.m3u8", "../segment.ts"),
            "https://example.com/segment.ts"
        );
        assert_eq!(
            make_absolute_url("https://example.com/", "https://cdn.example.com/video.mp4"),
            "https://cdn.example.com/video.mp4"
        );
    }

    // ========================================================================
    // Format Helper Tests
    // ========================================================================

    #[test]
    fn test_format_filesize() {
        assert_eq!(format_filesize(512), "512 B");
        assert_eq!(format_filesize(1024), "1.0 KiB");
        assert_eq!(format_filesize(1024 * 1024), "1.0 MiB");
        assert_eq!(format_filesize(1536 * 1024 * 1024), "1.5 GiB");
    }

    #[test]
    fn test_format_filesize_decimal() {
        assert_eq!(format_filesize_decimal(500), "500 B");
        assert_eq!(format_filesize_decimal(1000), "1.0 KB");
        assert_eq!(format_filesize_decimal(1000 * 1000), "1.0 MB");
        assert_eq!(format_filesize_decimal(1500 * 1000 * 1000), "1.5 GB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(45.0), "0:45");
        assert_eq!(format_duration(330.0), "5:30");
        assert_eq!(format_duration(5445.0), "1:30:45");
        assert_eq!(format_duration(0.0), "0:00");
        assert_eq!(format_duration(3661.0), "1:01:01");
    }
}
