//! Path handling and filename sanitization for the orchestrator
//!
//! Provides secure filename generation and path manipulation utilities.

use super::template::{OutputTemplate, RenderContext};
use super::{Orchestrator, Result};
use anyhow::Context;
use std::path::PathBuf;

impl Orchestrator {
    /// Generate output file path from the configured output template.
    ///
    /// Uses `%(field)s` template syntax to generate filenames from metadata.
    /// Templates containing `/` create subdirectories. Each path component
    /// is individually sanitized for filesystem safety.
    pub(super) fn generate_output_path(
        &self,
        info: &rdlp_types::InfoDict,
        format: &rdlp_types::Format,
    ) -> Result<PathBuf> {
        let file_ext = self.determine_file_extension(format);
        let template = OutputTemplate::parse(&self.config.output_template).map_err(|e| {
            super::OrchestratorError::Configuration(format!(
                "Invalid output template '{}': {e}",
                self.config.output_template
            ))
        })?;
        let ctx = RenderContext {
            epoch: chrono::Utc::now().timestamp(),
            autonumber: 0,
        };
        let rendered = template
            .render(info, format, &file_ext, &ctx)
            .map_err(|e| {
                super::OrchestratorError::Configuration(format!(
                    "Template rendering failed for '{}' (format {}): {e}",
                    info.title, format.format_id
                ))
            })?;

        let path = self.sanitize_template_path(&rendered);

        let full_path = if path.is_absolute() {
            path
        } else {
            self.config.output_directory.join(path)
        };

        // Create parent directories if the template produced subdirectories
        if let Some(parent) = full_path.parent()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create output directory {}", parent.display()))
                .map_err(super::OrchestratorError::Other)?;
        }

        Ok(full_path)
    }

    /// Sanitize a rendered template path by sanitizing each component individually.
    ///
    /// Splits on `/` (the template directory separator), sanitizes each component
    /// with `sanitize_filename()`, and reassembles as a `PathBuf`.
    fn sanitize_template_path(&self, rendered: &str) -> PathBuf {
        rendered
            .split('/')
            .map(|component| self.sanitize_filename(component))
            .collect()
    }

    /// Determine the actual file extension for a format
    ///
    /// For streaming protocols (HLS, DASH), uses the detected container format
    /// from segment metadata. Falls back to URL-based detection if not available.
    pub(super) fn determine_file_extension(&self, format: &rdlp_types::Format) -> String {
        // Priority 1: Use container field from HLS/DASH metadata
        if let Some(ref container) = format.container {
            return match container.as_str() {
                "m4s" | "m4v" | "m4a" => "mp4".to_owned(), // fMP4 variants → mp4
                other => other.split('_').next().unwrap_or(other).to_owned(),
            };
        }

        // Priority 2: Fallback for formats without detected container
        match format.ext.as_str() {
            "hls" | "m3u8" | "dash" | "mpd" => {
                super::container_resolver::ResolvedContainer::resolve(&self.config, None)
                    .format
                    .as_ext()
                    .to_owned()
            }
            ext => ext.to_owned(),
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
