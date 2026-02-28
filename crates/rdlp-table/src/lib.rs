//! Format table rendering for CLI display.
//!
//! Provides a responsive, terminal-width-aware table renderer for
//! displaying video/audio format lists. Supports column priority-based
//! hiding, Unicode-aware truncation, and configurable styles.
//!
//! The renderer works with [`FormatRow`] — a pre-extracted data struct
//! that callers populate from their domain types. This avoids a circular
//! crate dependency (rdlp-types already depends on rdlp-table for legacy
//! column constants).

#![warn(missing_docs)]

pub mod column;
mod constants;
pub mod truncate;

// Re-export legacy constants at crate root for backward compatibility.
pub use constants::*;

pub use column::{
    Align, ColumnBudget, ColumnDef, FormatRow, all_columns, border_overhead, compute_budget,
};

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
/// * `rows` - Pre-extracted format rows to display
/// * `opts` - Rendering options (width, color, compact)
///
/// # Returns
/// A `String` containing the complete table, ready for printing.
pub fn render_formats_table(rows: &[FormatRow], opts: &TableOpts) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let width = opts.width_override.unwrap_or_else(detect_width);
    let compact = opts.compact.unwrap_or(width < 80);
    let columns = all_columns();
    let budget = compute_budget(&columns, width, compact, rows);

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
    for row in rows {
        let cells: Vec<String> = budget
            .selected_indices
            .iter()
            .enumerate()
            .map(|(i, &col_idx)| {
                let col = &columns[col_idx];
                let raw = (col.extract)(row);
                truncate::pad_to_width(raw, budget.widths[i], col.align == Align::Right)
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
/// Returns one string per row, using the same column set and widths
/// as `render_formats_table`. These are suitable for `inquire::Select` items
/// so the selection menu aligns with the header table.
///
/// # Arguments
/// * `rows` - Pre-extracted format rows to render
/// * `opts` - Same options used for the header table
#[must_use]
pub fn render_format_rows(rows: &[FormatRow], opts: &TableOpts) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }

    let width = opts.width_override.unwrap_or_else(detect_width);
    let compact = opts.compact.unwrap_or(width < 80);
    let columns = all_columns();
    let budget = compute_budget(&columns, width, compact, rows);

    rows.iter()
        .map(|row| {
            let cells: Vec<String> = budget
                .selected_indices
                .iter()
                .enumerate()
                .map(|(i, &col_idx)| {
                    let col = &columns[col_idx];
                    let raw = (col.extract)(row);
                    truncate::pad_to_width(raw, budget.widths[i], col.align == Align::Right)
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

    fn sample_rows() -> Vec<FormatRow> {
        vec![
            FormatRow {
                quality: "1080p".to_string(),
                resolution: "1920x1080".to_string(),
                format_type: "MP4".to_string(),
                size: "837.9 MB".to_string(),
                codecs: "h264/aac".to_string(),
                fps: "30".to_string(),
                abr: "128".to_string(),
                vbr: "5000".to_string(),
                note: String::new(),
            },
            FormatRow {
                quality: "720p".to_string(),
                resolution: "1280x720".to_string(),
                format_type: "MP4".to_string(),
                size: "450.2 MB".to_string(),
                codecs: "h264/aac".to_string(),
                fps: "30".to_string(),
                abr: "128".to_string(),
                vbr: "3000".to_string(),
                note: String::new(),
            },
            FormatRow {
                quality: "480p".to_string(),
                resolution: "854x480".to_string(),
                format_type: "WebM".to_string(),
                size: "220.0 MB".to_string(),
                codecs: "vp9 (video only)".to_string(),
                fps: "30".to_string(),
                abr: String::new(),
                vbr: "1500".to_string(),
                note: String::new(),
            },
        ]
    }

    #[test]
    fn test_render_wide_includes_all_columns() {
        let rows = sample_rows();
        let opts = TableOpts {
            width_override: Some(160),
            color: ColorMode::Never,
            compact: None,
        };
        let output = render_formats_table(&rows, &opts);
        assert!(output.contains("Quality"));
        assert!(output.contains("Resolution"));
        assert!(output.contains("Type"));
        assert!(output.contains("Size"));
        assert!(output.contains("Codecs"));
        assert!(output.contains("FPS"));
    }

    #[test]
    fn test_render_narrow_hides_low_priority() {
        let rows = sample_rows();
        // 60 chars compact: Quality(7) + Resolution(10) + Type(4) + Size(8) + 4*2=8 overhead = 37.
        // Codecs(10)+2 = 49, FPS(3)+2=54, ABR(5)+2=61 won't fit — ABR and higher optional cols dropped.
        let opts = TableOpts {
            width_override: Some(60),
            color: ColorMode::Never,
            compact: Some(true),
        };
        let output = render_formats_table(&rows, &opts);
        // Core columns must exist
        assert!(output.contains("Quality"));
        assert!(output.contains("Type"));
        // Low priority should be hidden
        assert!(!output.contains("Note"));
        assert!(!output.contains("VBR"));
    }

    #[test]
    fn test_no_line_exceeds_width() {
        let rows = sample_rows();
        for width in [60, 80, 100, 120] {
            let opts = TableOpts {
                width_override: Some(width),
                color: ColorMode::Never,
                compact: None,
            };
            let output = render_formats_table(&rows, &opts);
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
        let rows = sample_rows();
        let opts = TableOpts {
            width_override: Some(120),
            color: ColorMode::Never,
            compact: None,
        };
        let output = render_formats_table(&rows, &opts);
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
        // Should produce empty string for empty input.
        assert!(output.is_empty());
    }

    #[test]
    fn test_compact_uses_blank_style() {
        let rows = sample_rows();
        let opts = TableOpts {
            width_override: Some(60),
            color: ColorMode::Never,
            compact: Some(true),
        };
        let output = render_formats_table(&rows, &opts);
        // Blank style has no box-drawing characters.
        assert!(!output.contains('│'));
        assert!(!output.contains('─'));
        assert!(!output.contains('╭'));
    }

    #[test]
    fn test_normal_uses_rounded_style() {
        let rows = sample_rows();
        let opts = TableOpts {
            width_override: Some(120),
            color: ColorMode::Never,
            compact: Some(false),
        };
        let output = render_formats_table(&rows, &opts);
        // Rounded style uses box-drawing chars.
        assert!(output.contains('│') || output.contains('─'));
    }
}
