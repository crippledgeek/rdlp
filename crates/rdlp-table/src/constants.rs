//! Legacy column layout constants for backward compatibility.
//!
//! These constants were the original `rdlp-table` API. They are preserved
//! for any external code that references them directly. The active table
//! renderer uses `column::ColumnDef` definitions instead.

/// Width of the Quality column (e.g. `"1080p60     "`).
pub const QUALITY_WIDTH: usize = 12;

/// Width of the Resolution column (e.g. `"1920x1080 "`).
pub const RESOLUTION_WIDTH: usize = 10;

/// Width of the Type column (e.g. `"HLS   "`).
pub const TYPE_WIDTH: usize = 6;

/// Width of the column separator `" | "`.
pub const SEP_WIDTH: usize = 3;

/// Approximate width of the trailing Codecs label used in the separator line.
const CODECS_LABEL_WIDTH: usize = 6;

/// Compute the total separator line width given a dynamic size column width.
#[must_use]
pub const fn separator_width(size_width: usize) -> usize {
    QUALITY_WIDTH
        + SEP_WIDTH
        + RESOLUTION_WIDTH
        + SEP_WIDTH
        + size_width
        + SEP_WIDTH
        + TYPE_WIDTH
        + SEP_WIDTH
        + CODECS_LABEL_WIDTH
}
