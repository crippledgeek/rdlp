//! Column definitions and responsive column selection.
//!
//! Each column has a priority, minimum width, alignment, and a cell
//! extraction function. The budget algorithm selects which columns fit
//! within the available terminal width, dropping lowest-priority columns
//! first.

use console::measure_text_width;

/// Horizontal alignment for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Left-align cell content (pad right).
    Left,
    /// Right-align cell content (pad left).
    Right,
}

/// Pre-extracted row data for table rendering.
///
/// Callers convert their format type into this struct before passing
/// to the renderer. This keeps `rdlp-table` free of `rdlp-types` as
/// a dependency (which would create a circular crate dependency).
#[derive(Debug, Clone, Default)]
pub struct FormatRow {
    /// Quality label (e.g. "1080p", "720p60").
    pub quality: String,
    /// Resolution string (e.g. "1920x1080") or "N/A".
    pub resolution: String,
    /// Format type (e.g. "HLS", "MP4").
    pub format_type: String,
    /// Human-readable size text (e.g. "837.9 MB", "50:16 (754 seg)").
    pub size: String,
    /// Codec description (e.g. "h264/aac", "vp9 (video only)").
    pub codecs: String,
    /// Frames per second as a string (e.g. "60"), or empty string.
    pub fps: String,
    /// Audio bitrate in kbps as a string (e.g. "128"), or empty string.
    pub abr: String,
    /// Video bitrate in kbps as a string (e.g. "5000"), or empty string.
    pub vbr: String,
    /// Extra notes (e.g. "HDR, en", "Dolby Vision"), or empty string.
    pub note: String,
}

/// Definition of a single table column.
#[derive(Clone)]
pub struct ColumnDef {
    /// Column header text.
    pub header: &'static str,
    /// Minimum display width (excluding padding/borders).
    pub min_width: usize,
    /// Priority for column selection (higher = more important, kept longer).
    /// Core columns (Quality, Resolution, Type) have priority >= 85.
    pub priority: u8,
    /// Text alignment within the column.
    pub align: Align,
    /// Extract cell text from a `FormatRow`.
    pub extract: fn(&FormatRow) -> &str,
}

fn extract_quality(r: &FormatRow) -> &str {
    &r.quality
}

fn extract_resolution(r: &FormatRow) -> &str {
    &r.resolution
}

fn extract_type(r: &FormatRow) -> &str {
    &r.format_type
}

fn extract_size(r: &FormatRow) -> &str {
    &r.size
}

fn extract_codecs(r: &FormatRow) -> &str {
    &r.codecs
}

fn extract_fps(r: &FormatRow) -> &str {
    &r.fps
}

fn extract_abr(r: &FormatRow) -> &str {
    &r.abr
}

fn extract_vbr(r: &FormatRow) -> &str {
    &r.vbr
}

fn extract_note(r: &FormatRow) -> &str {
    &r.note
}

/// All available columns in display order.
///
/// The order here determines the left-to-right column order in the table.
/// Priority determines which columns survive when the terminal is narrow.
#[must_use]
pub fn all_columns() -> Vec<ColumnDef> {
    vec![
        ColumnDef {
            header: "Quality",
            min_width: 7,
            priority: 100,
            align: Align::Left,
            extract: extract_quality,
        },
        ColumnDef {
            header: "Resolution",
            min_width: 9,
            priority: 90,
            align: Align::Left,
            extract: extract_resolution,
        },
        ColumnDef {
            header: "Type",
            min_width: 4,
            priority: 85,
            align: Align::Left,
            extract: extract_type,
        },
        ColumnDef {
            header: "Size",
            min_width: 8,
            priority: 80,
            align: Align::Right,
            extract: extract_size,
        },
        ColumnDef {
            header: "Codecs",
            min_width: 10,
            priority: 70,
            align: Align::Left,
            extract: extract_codecs,
        },
        ColumnDef {
            header: "FPS",
            min_width: 3,
            priority: 40,
            align: Align::Right,
            extract: extract_fps,
        },
        ColumnDef {
            header: "ABR",
            min_width: 5,
            priority: 35,
            align: Align::Right,
            extract: extract_abr,
        },
        ColumnDef {
            header: "VBR",
            min_width: 5,
            priority: 30,
            align: Align::Right,
            extract: extract_vbr,
        },
        ColumnDef {
            header: "Note",
            min_width: 6,
            priority: 20,
            align: Align::Left,
            extract: extract_note,
        },
    ]
}

/// Overhead per column from table borders/separators.
///
/// For `Style::rounded()`: outer borders (2 chars: `│` left + `│` right)
/// plus separators between columns (3 chars each: ` │ `).
/// Total overhead = 4 + (n-1)*3 for n columns.
///
/// For `Style::blank()`: no borders/separators, just 2 spaces between columns.
/// Overhead = (n-1) * 2.
#[must_use]
pub fn border_overhead(n_cols: usize, compact: bool) -> usize {
    if n_cols == 0 {
        return 0;
    }
    if compact {
        // Blank style: 1 space padding on each outer edge + 3 spaces between cells.
        // = 2 + (n-1)*3
        2 + (n_cols - 1) * 3
    } else {
        // Rounded style: │ + space (2) outer borders + space + │ + space (3) between each pair.
        // = 4 + (n-1)*3
        4 + (n_cols - 1) * 3
    }
}

/// Result of the column budget algorithm.
pub struct ColumnBudget {
    /// Indices into `all_columns()` that fit within the width.
    pub selected_indices: Vec<usize>,
    /// Allocated width for each selected column (in display columns).
    pub widths: Vec<usize>,
}

/// Select which columns fit within `available_width` and allocate widths.
///
/// Drops lowest-priority columns first until the remaining columns fit.
/// Core columns (priority >= 85: Quality, Resolution, Type) are never dropped.
/// Remaining width after minimums is distributed proportionally.
///
/// # Arguments
/// * `columns` - All available column definitions
/// * `available_width` - Terminal width in display columns
/// * `compact` - Whether compact (blank) style is used
/// * `rows` - The actual row data (used to compute natural column widths)
#[must_use]
pub fn compute_budget(
    columns: &[ColumnDef],
    available_width: usize,
    compact: bool,
    rows: &[FormatRow],
) -> ColumnBudget {
    // Start with all columns, sorted by display order (preserving index).
    let mut candidates: Vec<(usize, &ColumnDef)> = columns.iter().enumerate().collect();

    loop {
        let overhead = border_overhead(candidates.len(), compact);
        let min_total: usize =
            candidates.iter().map(|(_, c)| c.min_width).sum::<usize>() + overhead;

        if min_total <= available_width || candidates.len() <= 3 {
            // Fits, or we're at minimum columns — stop dropping.
            break;
        }

        // Drop the lowest-priority non-core column.
        // Core = priority >= 85 (Quality 100, Resolution 90, Type 85).
        if let Some(drop_pos) = candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, c))| c.priority < 85)
            .min_by_key(|(_, (_, c))| c.priority)
            .map(|(pos, _)| pos)
        {
            candidates.remove(drop_pos);
        } else {
            // All remaining are core — can't drop more.
            break;
        }
    }

    let overhead = border_overhead(candidates.len(), compact);
    let min_total: usize = candidates.iter().map(|(_, c)| c.min_width).sum::<usize>() + overhead;
    let extra = available_width.saturating_sub(min_total);

    // Compute natural widths (max of header width and all cell values).
    let natural_widths: Vec<usize> = candidates
        .iter()
        .map(|(_, col)| {
            let header_w = measure_text_width(col.header);
            let max_cell = rows
                .iter()
                .map(|r| measure_text_width((col.extract)(r)))
                .max()
                .unwrap_or(0);
            header_w.max(max_cell)
        })
        .collect();

    // Distribute extra width proportionally to columns that want more.
    let wants: Vec<usize> = natural_widths
        .iter()
        .zip(candidates.iter())
        .map(|(nat, (_, col))| nat.saturating_sub(col.min_width))
        .collect();
    let total_want: usize = wants.iter().sum();

    let widths: Vec<usize> = if total_want == 0 || extra == 0 {
        candidates.iter().map(|(_, c)| c.min_width).collect()
    } else {
        candidates
            .iter()
            .enumerate()
            .map(|(i, (_, col))| {
                let bonus = if total_want > 0 {
                    (wants[i] as f64 / total_want as f64 * extra as f64).floor() as usize
                } else {
                    0
                };
                col.min_width + bonus
            })
            .collect()
    };

    let selected_indices = candidates.iter().map(|(idx, _)| *idx).collect();

    ColumnBudget {
        selected_indices,
        widths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_columns_count() {
        assert_eq!(all_columns().len(), 9);
    }

    #[test]
    fn test_core_columns_never_dropped() {
        let columns = all_columns();
        // Width so small only core columns fit.
        let budget = compute_budget(&columns, 30, true, &[]);
        let selected: Vec<&str> = budget
            .selected_indices
            .iter()
            .map(|&i| columns[i].header)
            .collect();
        assert!(selected.contains(&"Quality"));
        assert!(selected.contains(&"Resolution"));
        assert!(selected.contains(&"Type"));
    }

    #[test]
    fn test_all_columns_at_wide_width() {
        let columns = all_columns();
        let budget = compute_budget(&columns, 200, false, &[]);
        assert_eq!(budget.selected_indices.len(), 9);
    }

    #[test]
    fn test_low_priority_dropped_first() {
        let columns = all_columns();
        // Moderately narrow — should drop Note (20) first, then VBR (30), etc.
        let budget = compute_budget(&columns, 60, false, &[]);
        let selected: Vec<&str> = budget
            .selected_indices
            .iter()
            .map(|&i| columns[i].header)
            .collect();
        // Note, VBR, ABR, FPS should be dropped before Codecs and Size.
        assert!(!selected.contains(&"Note"));
    }

    #[test]
    fn test_border_overhead_rounded() {
        // │ col1 │ col2 │ col3 │ = 2 + 2*3 + 2 = 10
        assert_eq!(border_overhead(3, false), 10);
    }

    #[test]
    fn test_border_overhead_compact() {
        // 3 cols blank style: 2 + (3-1)*3 = 2 + 6 = 8
        assert_eq!(border_overhead(3, true), 8);
    }

    #[test]
    fn test_border_overhead_single_column() {
        assert_eq!(border_overhead(1, false), 4);
        assert_eq!(border_overhead(1, true), 2);
    }

    #[test]
    fn test_budget_widths_at_least_minimum() {
        let columns = all_columns();
        let budget = compute_budget(&columns, 120, false, &[]);
        for (i, &idx) in budget.selected_indices.iter().enumerate() {
            assert!(
                budget.widths[i] >= columns[idx].min_width,
                "Column '{}' width {} < min {}",
                columns[idx].header,
                budget.widths[i],
                columns[idx].min_width
            );
        }
    }
}
