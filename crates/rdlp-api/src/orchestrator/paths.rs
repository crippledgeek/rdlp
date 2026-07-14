//! Path handling and filename sanitization for the orchestrator
//!
//! Provides secure filename generation and path manipulation utilities.

use super::template::{OutputTemplate, RenderContext};
use super::{Orchestrator, Result};
use anyhow::Context;
use std::path::{Path, PathBuf};

impl Orchestrator {
    /// Generate output file path from the configured output template.
    ///
    /// Uses `%(field)s` template syntax to generate filenames from metadata.
    /// Templates containing `/` create subdirectories. Each path component
    /// is individually sanitized for filesystem safety.
    ///
    /// This function is pure — it does NOT create any directories. Callers in
    /// async contexts must call [`Orchestrator::ensure_parent_dir`] on the
    /// returned path before writing, so that the blocking `create_dir_all`
    /// syscall is dispatched via `tokio::fs` rather than stalling the runtime.
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

        let path = Self::sanitize_template_path(&rendered);

        let full_path = if path.is_absolute() {
            path
        } else {
            self.config.output_directory.join(path)
        };

        Ok(full_path)
    }

    /// Ensure the parent directory of `path` exists, creating it if necessary.
    ///
    /// Uses `tokio::fs::create_dir_all`, which internally dispatches to a
    /// blocking worker — this keeps the caller's runtime thread responsive
    /// even on slow or network filesystems.
    pub(super) async fn ensure_parent_dir(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create output directory {}", parent.display()))
                .map_err(super::OrchestratorError::Other)?;
        }
        Ok(())
    }

    /// Sanitize a rendered template path by sanitizing each component individually.
    ///
    /// Splits on `/` (the template directory separator), sanitizes each component
    /// with `sanitize_filename()`, and reassembles as a `PathBuf`.
    fn sanitize_template_path(rendered: &str) -> PathBuf {
        rendered.split('/').map(Self::sanitize_filename).collect()
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
    pub(super) fn sanitize_filename(name: &str) -> String {
        // Step 1: Replace invalid filesystem characters and filter control characters
        let sanitized: String = name
            .chars()
            .filter_map(|c| {
                // Filter out null bytes and control characters (except space)
                if c == '\0' || (c.is_control() && c != ' ') {
                    return None;
                }
                // Filter out bidi-control characters (Trojan Source, CVE-2021-42574):
                // a U+202E RIGHT-TO-LEFT OVERRIDE in a title would otherwise persist
                // to a real filename and reorder how it renders (`...gpj.exe` shown as
                // `...exe.jpg`). These are category `Cf`, not `Cc`, so the control
                // filter above does not catch them (#487). The zero-width joiner
                // U+200D — also `Cf`, load-bearing for emoji — is intentionally kept.
                if rdlp_security::text::is_bidi_control(c) {
                    return None;
                }
                // Normalize non-ASCII whitespace (e.g. U+00A0 NO-BREAK SPACE from
                // a decoded `&nbsp;`, or other Unicode Zs/Zl/Zp separators) to a
                // plain ASCII space, so it renders and trims like ordinary
                // whitespace instead of persisting as an invisible non-space.
                if c.is_whitespace() {
                    return Some(' ');
                }
                // Replace invalid filesystem characters with underscore
                Some(match c {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    _ => c,
                })
            })
            .collect();

        // Step 1.5 (#406): neutralize the reserved temp-file markers so a user
        // title containing them cannot collide with the download/pipeline naming
        // scheme. Only the leading dot of each marker is changed to an
        // underscore, so the visible title text is preserved while the strip
        // logic (marker-search on the dotted form) can never mistake it for a
        // real marker.
        let sanitized = sanitized
            .replace(super::naming::PART_MARKER, "_rdlp-part")
            .replace(super::naming::TMP_MARKER, "_rdlp-tmp-");

        // Step 2: Trim leading/trailing dots and spaces (problematic on Windows)
        let trimmed = sanitized.trim_matches(|c| c == '.' || c == ' ');

        // Step 3: Check for Windows reserved names
        let base_name = trimmed
            .rfind('.')
            .map_or(trimmed, |dot_pos| &trimmed[..dot_pos]);

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

#[cfg(test)]
mod sanitize_marker_tests {
    use crate::orchestrator::Orchestrator;

    #[test]
    fn neutralizes_part_marker_in_title() {
        let out = Orchestrator::sanitize_filename("foo.rdlp-part");
        assert!(!out.contains(".rdlp-part"), "got: {out}");
        assert_eq!(out, "foo_rdlp-part");
    }

    #[test]
    fn neutralizes_tmp_marker_in_title() {
        let out = Orchestrator::sanitize_filename("clip.rdlp-tmp-abc");
        assert!(!out.contains(".rdlp-tmp-"), "got: {out}");
        assert_eq!(out, "clip_rdlp-tmp-abc");
    }

    #[test]
    fn leaves_ordinary_titles_unchanged() {
        assert_eq!(
            Orchestrator::sanitize_filename("Normal Title"),
            "Normal Title"
        );
    }

    /// A non-breaking space (U+00A0) — e.g. from a decoded `&nbsp;` title — is
    /// normalized to a plain ASCII space so it renders and trims like ordinary
    /// whitespace. Fails against the old code, which preserved U+00A0 verbatim
    /// (it is category Zs, not a control char, and the trim only matched 0x20).
    #[test]
    fn normalizes_non_breaking_space_to_ascii_space() {
        assert_eq!(Orchestrator::sanitize_filename("a\u{00A0}b"), "a b");
    }

    /// Leading/trailing Unicode whitespace is normalized first, so the existing
    /// dot/space trim then strips it. Fails against the old code, which left the
    /// U+00A0 padding in place.
    #[test]
    fn trims_leading_trailing_unicode_whitespace() {
        assert_eq!(
            Orchestrator::sanitize_filename("\u{00A0}Title\u{00A0}"),
            "Title"
        );
    }

    #[test]
    fn neutralizes_marker_after_leading_dots() {
        // Guards the neutralize-BEFORE-trim ordering: trim must not re-expose a marker.
        let out = Orchestrator::sanitize_filename("..rdlp-part");
        assert!(!out.contains(".rdlp-part"), "got: {out}");
    }

    #[test]
    fn neutralizes_all_marker_occurrences() {
        let out = Orchestrator::sanitize_filename("x.rdlp-part.rdlp-part");
        assert!(!out.contains(".rdlp-part"), "got: {out}");
    }

    #[test]
    fn neutralized_title_round_trips_to_intended_clean_name() {
        // The cross-module invariant: a title containing the marker, once
        // sanitized, survives a full part_path -> finalize_to_clean cycle and
        // yields the SANITIZED clean name, never a truncation at title text.
        use crate::orchestrator::naming::{finalize_to_clean, part_path};
        use std::path::PathBuf;

        let stem = Orchestrator::sanitize_filename("foo.rdlp-part"); // "foo_rdlp-part"
        let clean = PathBuf::from(format!("/v/{stem}.mp4"));
        let part = part_path(&clean);
        assert_eq!(part, PathBuf::from("/v/foo_rdlp-part.rdlp-part.mp4"));
        assert_eq!(finalize_to_clean(&part), Some(clean));
    }
}

/// Filesystem "Trojan Source" (CVE-2021-42574) coverage: `sanitize_filename`
/// must strip bidi-control characters so an extractor-sourced title cannot
/// persist a right-to-left-override to a real filename on disk. The original
/// Trojan Source attack embeds `U+202E` so a name whose bytes end `...gpj.exe`
/// renders as `...exe.jpg`. #485 fixed only the terminal boundary; this pins
/// the higher-value persistent filesystem boundary (#487).
#[cfg(test)]
mod sanitize_bidi_tests {
    use crate::orchestrator::Orchestrator;

    #[test]
    fn strips_rlo_override_from_filename() {
        // The canonical Trojan Source vector: U+202E RIGHT-TO-LEFT OVERRIDE
        // reorders how the filename renders. It must never reach the filesystem.
        // Fails against the pre-#487 code, which only filtered `Cc` controls
        // and left this `Cf` bidi-control intact.
        let out = Orchestrator::sanitize_filename("invoice\u{202e}gpj.exe");
        assert!(!out.contains('\u{202e}'), "RLO must be stripped: {out}");
        assert_eq!(out, "invoicegpj.exe");
    }

    #[test]
    fn strips_full_bidi_block_and_preserves_boundaries() {
        // First range U+202A..=U+202E (LRE,RLE,PDF,LRO,RLO): pin both edges.
        // U+2029 (below) is a Zp separator normalized to ASCII space, and
        // U+202F NARROW NO-BREAK SPACE (above) is a Zs also normalized to a
        // space; both survive as (interior) spaces while the whole override
        // block between them is stripped — so a slide of either edge into the
        // strip range would drop a space and fail this assertion.
        let out = Orchestrator::sanitize_filename("a\u{2029}\u{202a}\u{202e}\u{202f}b");
        assert_eq!(out, "a  b");

        // Second range U+2066..=U+2069 (LRI,RLI,FSI,PDI): U+2065 (below,
        // unassigned) and U+206A (above, a distinct `Cf` char we keep) survive
        // while the isolate block is removed.
        let out = Orchestrator::sanitize_filename("\u{2065}\u{2066}\u{2069}\u{206a}");
        assert_eq!(out, "\u{2065}\u{206a}");
    }

    #[test]
    fn preserves_zwj_emoji_sequence() {
        // U+200D ZERO WIDTH JOINER is category `Cf` like the bidi controls, but
        // it is load-bearing for legitimate emoji (a family emoji is
        // person-ZWJ-person-ZWJ-child). Only the bidi-control block may be
        // stripped — the whole `Cf` category must NOT be, or real titles break.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(Orchestrator::sanitize_filename(family), family);
    }
}
