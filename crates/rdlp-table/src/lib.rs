//! Format table rendering for CLI display.
//!
//! Provides a responsive, terminal-width-aware table renderer for
//! displaying video/audio format lists. Supports column priority-based
//! hiding, Unicode-aware truncation, and configurable styles.

#![warn(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

pub mod column;
pub mod truncate;

pub use column::{Align, ColumnBudget, ColumnDef, all_columns, border_overhead, compute_budget};

use std::borrow::Cow;

use rdlp_types::Format;
use tabled::{
    builder::Builder,
    settings::{Alignment, Style, object::Columns},
};

/// Options controlling table rendering.
#[derive(Debug, Clone, Default)]
pub struct TableOpts {
    /// Override terminal width. `None` = auto-detect, fallback 120.
    pub width_override: Option<usize>,
    /// Force compact mode. `None` = auto (compact when width < 80).
    pub compact: Option<bool>,
}

/// Detect terminal width, falling back to 120 if unavailable.
fn detect_width() -> usize {
    terminal_size::terminal_size().map_or(120, |(w, _)| w.0 as usize)
}

/// Per-call rendering layout shared by every entry point: compact mode, the full
/// column set, and the fitted column budget for the current width and data.
struct Layout {
    compact: bool,
    columns: Vec<ColumnDef>,
    budget: ColumnBudget,
}

/// Resolve the layout (width → compact, columns, fitted budget) shared by all
/// render entry points. Callers apply their own empty-input guards around this.
fn resolve_layout(formats: &[&Format], opts: &TableOpts) -> Layout {
    let width = opts.width_override.unwrap_or_else(detect_width);
    let compact = opts.compact.unwrap_or(width < 80);
    let columns = all_columns();
    let budget = compute_budget(&columns, width, compact, formats);
    Layout {
        compact,
        columns,
        budget,
    }
}

/// Build one table record: for each selected column, take its raw text from
/// `cell`, then truncate and pad it to that column's budgeted width and alignment.
///
/// `cell` yields borrowed text for headers (`col.header`) or owned text for data
/// rows (`(col.extract)(fmt)`), so both paths share this single implementation.
// Indexing `columns[col_idx]` / `budget.widths[i]`: `selected_indices` and `widths`
// are built together with consistent lengths, and `i` comes from `enumerate()` on
// the same `selected_indices` vec, so both indices are always in bounds.
#[allow(clippy::indexing_slicing)]
fn padded_record(
    columns: &[ColumnDef],
    budget: &ColumnBudget,
    cell: impl Fn(&ColumnDef) -> Cow<'_, str>,
) -> Vec<String> {
    budget
        .selected_indices
        .iter()
        .enumerate()
        .map(|(i, &col_idx)| {
            let col = &columns[col_idx];
            truncate::pad_to_width(&cell(col), budget.widths[i], col.align == Align::Right)
        })
        .collect()
}

/// Render a responsive format table for terminal display.
///
/// Auto-detects terminal width (or uses `opts.width_override`), selects
/// columns that fit via priority-based budget, truncates cells, and
/// returns a styled table string.
///
/// # Arguments
/// * `formats` - Format references to display
/// * `opts` - Rendering options (width, compact)
///
/// # Returns
/// A `String` containing the complete table, ready for printing.
// Indexing budget.widths[i] / columns[col_idx]: both vecs are built together
// and have consistent lengths; i is from enumerate() on the same vec.
#[allow(clippy::indexing_slicing)]
pub fn render_formats_table(formats: &[&Format], opts: &TableOpts) -> String {
    if formats.is_empty() {
        return String::new();
    }

    let Layout {
        compact,
        columns,
        budget,
    } = resolve_layout(formats, opts);

    if budget.selected_indices.is_empty() {
        return String::new();
    }

    // Build the tabled grid.
    let mut builder = Builder::new();

    // Header row.
    builder.push_record(padded_record(&columns, &budget, |col| {
        Cow::Borrowed(col.header)
    }));

    // Data rows.
    for fmt in formats {
        builder.push_record(padded_record(&columns, &budget, |col| {
            Cow::Owned((col.extract)(fmt))
        }));
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

    let Layout {
        compact,
        columns,
        budget,
    } = resolve_layout(formats, opts);

    formats
        .iter()
        .map(|fmt| {
            let cells = padded_record(&columns, &budget, |col| Cow::Owned((col.extract)(fmt)));
            if compact {
                cells.join("  ")
            } else {
                cells.join(" │ ")
            }
        })
        .collect()
}

/// Render a header line and per-row strings in a single pass.
///
/// Computes the column budget once and returns a header line (column names
/// with a separator underline) plus per-row strings for `inquire::Select`.
/// The header and rows use identical column widths and separators so they
/// align visually.
///
/// # Arguments
/// * `formats` - Format references to display
/// * `opts` - Rendering options (width, compact)
///
/// # Returns
/// A tuple of (header string, row strings). Both are empty if `formats` is empty.
#[must_use]
pub fn render_table_and_rows(formats: &[&Format], opts: &TableOpts) -> (String, Vec<String>) {
    if formats.is_empty() {
        return (String::new(), Vec::new());
    }

    let Layout {
        compact,
        columns,
        budget,
    } = resolve_layout(formats, opts);

    if budget.selected_indices.is_empty() {
        return (String::new(), Vec::new());
    }

    let sep = if compact { "  " } else { " │ " };
    let dash_sep = if compact { "  " } else { "─┼─" };

    // Header line: column names padded to budget widths.
    let header_line = padded_record(&columns, &budget, |col| Cow::Borrowed(col.header)).join(sep);

    // Separator line: dashes matching each column width.
    let dash_cells: Vec<String> = budget.widths.iter().map(|&w| "─".repeat(w)).collect();
    let dash_line = dash_cells.join(dash_sep);

    let header_str = format!("{header_line}\n{dash_line}");

    // Build row strings using the same budget.
    let rows = formats
        .iter()
        .map(|fmt| padded_record(&columns, &budget, |col| Cow::Owned((col.extract)(fmt))).join(sep))
        .collect();

    (header_str, rows)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    clippy::missing_docs_in_private_items,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use console::measure_text_width;
    use rdlp_types::Codec;
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
        f.vcodec = Codec::from("h264".to_string());
        f.acodec = Codec::from("aac".to_string());
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
                f.vcodec = Codec::from("vp9".to_string());
                f.acodec = Codec::Absent;
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
            compact: None,
        };
        let output = render_formats_table(&refs, &opts);
        assert!(!output.is_empty());
    }

    #[test]
    fn test_empty_formats() {
        let opts = TableOpts {
            width_override: Some(80),
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
            compact: Some(false),
        };
        let output = render_formats_table(&refs, &opts);
        assert!(output.contains('│') || output.contains('─'));
    }

    #[test]
    fn test_format_rows_one_per_format_and_aligned() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(160),
            compact: None,
        };
        let rows = render_format_rows(&refs, &opts);
        assert_eq!(rows.len(), formats.len());
        assert!(rows[0].contains("1080p"));
        assert!(rows[1].contains("720p"));
        assert!(rows[2].contains("480p"));
        // Every row is padded to the same display width (column alignment preserved).
        let w0 = measure_text_width(&rows[0]);
        for row in &rows {
            assert_eq!(measure_text_width(row), w0, "row width mismatch: '{row}'");
        }
    }

    #[test]
    fn test_format_rows_separator_normal_vs_compact() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let normal = render_format_rows(
            &refs,
            &TableOpts {
                width_override: Some(160),
                compact: Some(false),
            },
        );
        assert!(normal[0].contains(" │ "), "normal rows use ' │ ' separator");
        let compact = render_format_rows(
            &refs,
            &TableOpts {
                width_override: Some(60),
                compact: Some(true),
            },
        );
        assert!(!compact[0].contains('│'), "compact rows have no '│'");
    }

    #[test]
    fn test_format_rows_empty() {
        let opts = TableOpts {
            width_override: Some(120),
            compact: None,
        };
        assert!(render_format_rows(&[], &opts).is_empty());
    }

    #[test]
    fn test_table_and_rows_structure() {
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        let opts = TableOpts {
            width_override: Some(160),
            compact: Some(false),
        };
        let (header, rows) = render_table_and_rows(&refs, &opts);
        // Header is a name line plus a dash underline.
        let header_lines: Vec<&str> = header.lines().collect();
        assert_eq!(header_lines.len(), 2);
        assert!(header_lines[0].contains("Quality"));
        assert!(header_lines[1].contains('─'));
        assert_eq!(rows.len(), formats.len());
        // Header name line and each row share the same display width.
        let hw = measure_text_width(header_lines[0]);
        for row in &rows {
            assert_eq!(
                measure_text_width(row),
                hw,
                "row/header width mismatch: '{row}'"
            );
        }
    }

    #[test]
    fn test_table_and_rows_empty() {
        let opts = TableOpts {
            width_override: Some(120),
            compact: None,
        };
        let (header, rows) = render_table_and_rows(&[], &opts);
        assert!(header.is_empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn test_format_rows_and_table_rows_agree() {
        // Both entry points render identical data rows for the same input/width,
        // since they share the column budget and cell formatting.
        let formats = sample_formats();
        let refs: Vec<&Format> = formats.iter().collect();
        for &(width, compact) in &[(160, false), (60, true)] {
            let opts = TableOpts {
                width_override: Some(width),
                compact: Some(compact),
            };
            let rows = render_format_rows(&refs, &opts);
            let (_, table_rows) = render_table_and_rows(&refs, &opts);
            assert_eq!(rows, table_rows, "row rendering diverged at width {width}");
        }
    }
}
