//! Format table rendering for CLI display.
//!
//! Provides a responsive, terminal-width-aware table renderer for
//! displaying video/audio format lists. Supports column priority-based
//! hiding, Unicode-aware truncation, and configurable styles.

#![warn(missing_docs)]

pub mod column;
mod constants;
pub mod truncate;

// Re-export legacy constants at crate root for backward compatibility.
pub use constants::*;

pub use column::{Align, ColumnBudget, ColumnDef, all_columns, border_overhead, compute_budget};

use rdlp_types::Format;
use tabled::{
    builder::Builder,
    settings::{Alignment, Style, object::Columns},
};

/// Options controlling table rendering.
#[derive(Debug, Clone)]
pub struct TableOpts {
    /// Override terminal width. `None` = auto-detect, fallback 120.
    pub width_override: Option<usize>,
    /// ANSI color output mode.
    pub color: ColorMode,
    /// Force compact mode. `None` = auto (compact when width < 80).
    pub compact: Option<bool>,
}

impl Default for TableOpts {
    fn default() -> Self {
        Self {
            width_override: None,
            color: ColorMode::Auto,
            compact: None,
        }
    }
}

/// ANSI color output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Detect from terminal capabilities and `NO_COLOR` env var.
    Auto,
    /// Always emit ANSI codes.
    Always,
    /// Never emit ANSI codes.
    Never,
}

/// Detect terminal width, falling back to 120 if unavailable.
fn detect_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(120)
}

/// Render a responsive format table for terminal display.
///
/// Auto-detects terminal width (or uses `opts.width_override`), selects
/// columns that fit via priority-based budget, truncates cells, and
/// returns a styled table string.
///
/// # Arguments
/// * `formats` - Format references to display
/// * `opts` - Rendering options (width, color, compact)
///
/// # Returns
/// A `String` containing the complete table, ready for printing.
pub fn render_formats_table(formats: &[&Format], opts: &TableOpts) -> String {
    if formats.is_empty() {
        return String::new();
    }

    let width = opts.width_override.unwrap_or_else(detect_width);
    let compact = opts.compact.unwrap_or(width < 80);
    let columns = all_columns();
    let budget = compute_budget(&columns, width, compact, formats);

    if budget.selected_indices.is_empty() {
        return String::new();
    }

    // Build the tabled grid.
    let mut builder = Builder::new();

    // Header row.
    let headers: Vec<String> = budget
        .selected_indices
        .iter()
        .enumerate()
        .map(|(i, &col_idx)| {
            let col = &columns[col_idx];
            truncate::pad_to_width(col.header, budget.widths[i], col.align == Align::Right)
        })
        .collect();
    builder.push_record(headers);

    // Data rows.
    for fmt in formats {
        let cells: Vec<String> = budget
            .selected_indices
            .iter()
            .enumerate()
            .map(|(i, &col_idx)| {
                let col = &columns[col_idx];
                let raw = (col.extract)(fmt);
                truncate::pad_to_width(&raw, budget.widths[i], col.align == Align::Right)
            })
            .collect();
        builder.push_record(cells);
    }

    let mut table = builder.build();

    // Apply style.
    if compact {
        table.with(Style::blank());
    } else {
        table.with(Style::rounded());
    }

    // Apply alignment for right-aligned columns.
    for (i, &col_idx) in budget.selected_indices.iter().enumerate() {
        if columns[col_idx].align == Align::Right {
            table.modify(Columns::one(i), Alignment::right());
        }
    }

    table.to_string()
}

/// Render per-row strings matching the table's column layout.
///
/// Returns one string per format, using the same column set and widths
/// as `render_formats_table`. These are suitable for `inquire::Select` items
/// so the selection menu aligns with the header table.
///
/// # Arguments
/// * `formats` - Format references to render
/// * `opts` - Same options used for the header table
#[must_use]
pub fn render_format_rows(formats: &[&Format], opts: &TableOpts) -> Vec<String> {
    if formats.is_empty() {
        return Vec::new();
    }

    let width = opts.width_override.unwrap_or_else(detect_width);
    let compact = opts.compact.unwrap_or(width < 80);
    let columns = all_columns();
    let budget = compute_budget(&columns, width, compact, formats);

    formats
        .iter()
        .map(|fmt| {
            let cells: Vec<String> = budget
                .selected_indices
                .iter()
                .enumerate()
                .map(|(i, &col_idx)| {
                    let col = &columns[col_idx];
                    let raw = (col.extract)(fmt);
                    truncate::pad_to_width(&raw, budget.widths[i], col.align == Align::Right)
                })
                .collect();
            if compact {
                cells.join("  ")
            } else {
                cells.join(" │ ")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::measure_text_width;
    use rdlp_types::protocol::DownloadProtocol;

    fn make_format(note: &str, width: u32, height: u32, ext: &str) -> Format {
        let mut f = Format::new(
            "id",
            "https://example.com/video",
            ext,
            DownloadProtocol::Https,
        );
        f.format_note = Some(note.to_string());
        f.width = Some(width);
        f.height = Some(height);
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("aac".to_string());
        f.fps = Some(30.0);
        f.abr = Some(128.0);
        f.vbr = Some(5000.0);
        f.filesize = Some(878_000_000);
        f
    }

    fn sample_formats() -> Vec<Format> {
        vec![
            make_format("1080p", 1920, 1080, "mp4"),
            make_format("720p", 1280, 720, "mp4"),
            {
                let mut f = make_format("480p", 854, 480, "webm");
                f.vcodec = Some("vp9".to_string());
                f.acodec = Some("none".to_string());
                f.filesize = Some(230_000_000);
                f
            },
        ]
    }

    #[test]
    fn test_render_wide_includes_all_columns() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(160),
            color: ColorMode::Never,
            compact: None,
        };
        let output = render_formats_table(&refs, &opts);
        assert!(output.contains("Quality"));
        assert!(output.contains("Resolution"));
        assert!(output.contains("Type"));
        assert!(output.contains("Size"));
        assert!(output.contains("Codecs"));
        assert!(output.contains("FPS"));
    }

    #[test]
    fn test_render_narrow_hides_low_priority() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(60),
            color: ColorMode::Never,
            compact: Some(true),
        };
        let output = render_formats_table(&refs, &opts);
        assert!(output.contains("Quality"));
        assert!(output.contains("Type"));
        assert!(!output.contains("Note"));
        assert!(!output.contains("VBR"));
    }

    #[test]
    fn test_no_line_exceeds_width() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        for width in [60, 80, 100, 120] {
            let opts = TableOpts {
                width_override: Some(width),
                color: ColorMode::Never,
                compact: None,
            };
            let output = render_formats_table(&refs, &opts);
            for line in output.lines() {
                let line_width = measure_text_width(line);
                assert!(
                    line_width <= width,
                    "Line exceeds width {width}: ({line_width}) '{line}'"
                );
            }
        }
    }

    #[test]
    fn test_right_alignment_renders_without_error() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(120),
            color: ColorMode::Never,
            compact: None,
        };
        let output = render_formats_table(&refs, &opts);
        assert!(!output.is_empty());
    }

    #[test]
    fn test_empty_formats() {
        let opts = TableOpts {
            width_override: Some(80),
            color: ColorMode::Never,
            compact: None,
        };
        let output = render_formats_table(&[], &opts);
        assert!(output.is_empty());
    }

    #[test]
    fn test_compact_uses_blank_style() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(60),
            color: ColorMode::Never,
            compact: Some(true),
        };
        let output = render_formats_table(&refs, &opts);
        assert!(!output.contains('│'));
        assert!(!output.contains('─'));
        assert!(!output.contains('╭'));
    }

    #[test]
    fn test_normal_uses_rounded_style() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(120),
            color: ColorMode::Never,
            compact: Some(false),
        };
        let output = render_formats_table(&refs, &opts);
        assert!(output.contains('│') || output.contains('─'));
    }
}
