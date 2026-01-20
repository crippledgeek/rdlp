//! Utility functions for extractors
//!
//! This module provides common helper functions used across multiple extractors
//! to reduce code duplication and maintain consistency.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
