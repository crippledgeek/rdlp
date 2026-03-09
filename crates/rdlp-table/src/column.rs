//! Column definitions and responsive column selection.
//!
//! Each column has a priority, minimum width, alignment, and a cell
//! extraction function. The budget algorithm selects which columns fit
//! within the available terminal width, dropping lowest-priority columns
//! first.

use console::measure_text_width;
use rdlp_types::Format;

/// Horizontal alignment for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Left-align cell content (pad right).
    Left,
    /// Right-align cell content (pad left).
    Right,
}

/// Definition of a single table column.
#[derive(Debug, Clone)]
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
    /// Extract cell text from a [`Format`].
    pub extract: fn(&Format) -> String,
}

fn extract_quality(f: &Format) -> String {
    let base = f.format_note.as_deref().unwrap_or("unknown");
    match f.fps {
        Some(fps) if fps > 0.0 && (fps - 30.0).abs() > 1.0 => format!("{base}{fps:.0}"),
        _ => base.to_string(),
    }
}

fn extract_resolution(f: &Format) -> String {
    match (f.width, f.height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => "N/A".to_string(),
    }
}

fn extract_type(f: &Format) -> String {
    if f.is_hls() {
        "HLS".to_string()
    } else {
        f.ext.to_ascii_uppercase()
    }
}

fn extract_size(f: &Format) -> String {
    f.size_text()
}

fn extract_codecs(f: &Format) -> String {
    let codecs = match (&f.vcodec, &f.acodec) {
        (Some(v), Some(a)) if v != "none" && a != "none" => format!("{v}/{a}"),
        (Some(v), _) if v != "none" => format!("{v} (video only)"),
        (_, Some(a)) if a != "none" => format!("{a} (audio only)"),
        _ => "Unknown".to_string(),
    };
    if f.has_drm.unwrap_or(false) {
        format!("{codecs} [DRM]")
    } else {
        codecs
    }
}

fn extract_fps(f: &Format) -> String {
    match f.fps {
        Some(fps) if fps > 0.0 => format!("{fps:.0}"),
        _ => String::new(),
    }
}

fn extract_abr(f: &Format) -> String {
    match f.abr {
        Some(abr) if abr > 0.0 => format!("{abr:.0}"),
        _ => String::new(),
    }
}

fn extract_vbr(f: &Format) -> String {
    match f.vbr {
        Some(vbr) if vbr > 0.0 => format!("{vbr:.0}"),
        _ => String::new(),
    }
}

fn extract_note(f: &Format) -> String {
    let mut parts = Vec::new();
    if let Some(dr) = &f.dynamic_range {
        parts.push(dr.clone());
    }
    if let Some(lang) = &f.language {
        parts.push(lang.clone());
    }
    parts.join(", ")
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// * `formats` - The actual format data (used to compute natural column widths)
#[must_use]
pub fn compute_budget(
    columns: &[ColumnDef],
    available_width: usize,
    compact: bool,
    formats: &[&Format],
) -> ColumnBudget {
    // Start with all columns, sorted by display order (preserving index).
    let mut candidates: Vec<(usize, &ColumnDef)> = columns.iter().enumerate().collect();

    // Drop columns where ALL cells are empty (no data to show).
    if !formats.is_empty() {
        candidates.retain(|(_, col)| formats.iter().any(|f| !((col.extract)(f)).is_empty()));
    }

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
            let max_cell = formats
                .iter()
                .map(|f| measure_text_width(&(col.extract)(f)))
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
                // Cap at natural width so extra space isn't wasted as padding.
                (col.min_width + bonus)
                    .min(natural_widths[i])
                    .max(col.min_width)
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
        f
    }

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
        let f1 = make_format("1080p", 1920, 1080, "mp4");
        let f2 = make_format("720p", 1280, 720, "mp4");
        let columns = all_columns();
        let budget = compute_budget(&columns, 120, false, &[&f1, &f2]);
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

    #[test]
    fn test_empty_columns_hidden() {
        // Format with no fps, abr, vbr, or note — those columns should be dropped.
        let mut f = Format::new(
            "id",
            "https://example.com/video",
            "mp4",
            DownloadProtocol::Https,
        );
        f.format_note = Some("720p".into());
        f.width = Some(1280);
        f.height = Some(720);
        f.vcodec = Some("h264".into());
        f.acodec = Some("aac".into());
        // fps, abr, vbr not set → extract returns empty string
        // dynamic_range, language not set → extract_note returns empty string

        let columns = all_columns();
        let budget = compute_budget(&columns, 120, false, &[&f]);
        let selected: Vec<&str> = budget
            .selected_indices
            .iter()
            .map(|&i| columns[i].header)
            .collect();

        // Core + Size + Codecs should be present.
        assert!(selected.contains(&"Quality"));
        assert!(selected.contains(&"Resolution"));
        assert!(selected.contains(&"Type"));
        assert!(selected.contains(&"Codecs"));

        // All-empty columns should be hidden.
        assert!(!selected.contains(&"FPS"));
        assert!(!selected.contains(&"ABR"));
        assert!(!selected.contains(&"VBR"));
        assert!(!selected.contains(&"Note"));
    }
}
