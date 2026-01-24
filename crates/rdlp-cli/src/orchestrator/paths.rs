//! Path handling and filename sanitization for the orchestrator
//!
//! Provides secure filename generation and path manipulation utilities.

use super::{Orchestrator, Result};
use std::path::PathBuf;

impl Orchestrator {
    /// Generate output file path
    pub(super) fn generate_output_path(
        &self,
        info: &rdlp_core::InfoDict,
        format: &rdlp_core::Format,
    ) -> Result<PathBuf> {
        // Determine the actual file extension
        let file_ext = self.determine_file_extension(format);

        let filename = format!("{}.{}", self.sanitize_filename(&info.title), file_ext);

        let mut path = self.config.output_directory.clone();
        path.push(filename);

        Ok(path)
    }

    /// Determine the actual file extension for a format
    ///
    /// For streaming protocols (HLS, DASH), detects the actual container format
    /// from the format metadata or segment URLs.
    pub(super) fn determine_file_extension(&self, format: &rdlp_core::Format) -> String {
        // Priority 1: Use container field if explicitly set
        if let Some(ref container) = format.container {
            // Clean up container names (e.g., "mp4_dash" -> "mp4")
            return container
                .split('_')
                .next()
                .unwrap_or(container)
                .to_string();
        }

        // Priority 2: For HLS/DASH, detect from URL or default to mp4
        match format.ext.as_str() {
            "hls" | "m3u8" => {
                // Try to detect from URL (e.g., .../segment.ts)
                if format.url.contains(".ts") {
                    "ts".to_string() // MPEG-TS segments
                } else {
                    "mp4".to_string() // fMP4 segments (.m4s/.mp4) or default
                }
            }
            "dash" | "mpd" => {
                // DASH typically uses fMP4
                if format.url.contains(".webm") {
                    "webm".to_string()
                } else {
                    "mp4".to_string() // Default to MP4 for DASH
                }
            }
            ext => ext.to_string(), // Use extension as-is for direct formats
        }
    }

    /// Maximum filename length (conservative limit for cross-platform compatibility)
    pub(super) const MAX_FILENAME_LENGTH: usize = 200;

    /// Windows reserved filenames (case-insensitive)
    pub(super) const WINDOWS_RESERVED_NAMES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    /// Sanitize filename for safe filesystem usage
    ///
    /// This function provides comprehensive protection against:
    /// - Path traversal attacks (removes `/`, `\`, `:`)
    /// - Invalid filesystem characters (`*`, `?`, `"`, `<`, `>`, `|`)
    /// - Null bytes and control characters
    /// - Windows reserved filenames (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
    /// - Leading/trailing dots and spaces
    /// - Excessive filename length (truncated to 200 chars)
    ///
    /// # Security
    ///
    /// This function is critical for security. Never use unsanitized filenames
    /// directly from external sources (video titles, URLs, etc.).
    pub(super) fn sanitize_filename(&self, name: &str) -> String {
        // Step 1: Replace invalid filesystem characters and filter control characters
        let sanitized: String = name
            .chars()
            .filter_map(|c| {
                // Filter out null bytes and control characters (except space)
                if c == '\0' || (c.is_control() && c != ' ') {
                    return None;
                }
                // Replace invalid filesystem characters with underscore
                Some(match c {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    _ => c,
                })
            })
            .collect();

        // Step 2: Trim leading/trailing dots and spaces (problematic on Windows)
        let trimmed = sanitized.trim_matches(|c| c == '.' || c == ' ');

        // Step 3: Check for Windows reserved names
        let base_name = if let Some(dot_pos) = trimmed.rfind('.') {
            &trimmed[..dot_pos]
        } else {
            trimmed
        };

        let result = if Self::WINDOWS_RESERVED_NAMES
            .iter()
            .any(|&reserved| base_name.eq_ignore_ascii_case(reserved))
        {
            // Prefix with underscore to avoid reserved name collision
            format!("_{trimmed}")
        } else {
            trimmed.to_string()
        };

        // Step 4: Handle empty result
        let result = if result.is_empty() {
            "unnamed".to_string()
        } else {
            result
        };

        // Step 5: Truncate to maximum length (preserving extension if possible)
        if result.len() > Self::MAX_FILENAME_LENGTH {
            Self::truncate_filename(&result, Self::MAX_FILENAME_LENGTH)
        } else {
            result
        }
    }

    /// Truncate filename while preserving extension
    pub(super) fn truncate_filename(name: &str, max_len: usize) -> String {
        if let Some(dot_pos) = name.rfind('.') {
            let ext = &name[dot_pos..];
            // Only preserve extension if it's reasonable length (< 10 chars)
            if ext.len() < 10 && dot_pos > 0 {
                let base_max = max_len.saturating_sub(ext.len());
                if base_max > 0 {
                    let base = &name[..dot_pos];
                    // Truncate at char boundary
                    let truncated_base: String = base.chars().take(base_max).collect();
                    return format!("{truncated_base}{ext}");
                }
            }
        }
        // Fallback: simple truncation at char boundary
        name.chars().take(max_len).collect()
    }
}
