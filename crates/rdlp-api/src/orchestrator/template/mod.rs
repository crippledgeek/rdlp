//! Output template parser and renderer — full yt-dlp compatibility
//!
//! Supports the complete yt-dlp `%(field)s` template syntax for output filename
//! generation from `InfoDict` and `Format` metadata.
//!
//! ## Syntax
//!
//! ```text
//! %(name[.keys][+/-/*÷arith][>strf][,alternate][&replacement][|default])[#][0][width][.precision]type
//! ```

mod format_types;
mod helpers;
mod parser;
mod render;

#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod tests_format_types;
#[cfg(test)]
mod tests_parser;
#[cfg(test)]
mod tests_render;

/// Parsed output template ready for rendering
#[derive(Debug)]
pub(in crate::orchestrator) struct OutputTemplate {
    segments: Vec<Segment>,
}

/// A segment in a parsed template
#[derive(Debug)]
enum Segment {
    /// Literal text (passed through unchanged)
    Literal(String),
    /// A field reference to be resolved at render time
    Field(FieldSpec),
}

/// Specification for a single template field `%( ... )spec`
#[derive(Debug)]
struct FieldSpec {
    /// Fallback chain of field references (field1,field2,field3)
    names: Vec<FieldRef>,
    /// Conditional replacement text (`&text`); `{}` in text is replaced by field value
    replacement: Option<String>,
    /// Explicit default value (`|default`); `None` means fall back to `"NA"`
    default: Option<String>,
    /// Format specifier after `)`
    format: FormatSpec,
}

/// A single field reference with accessors and transforms
#[derive(Debug)]
struct FieldRef {
    /// Base field name
    name: String,
    /// Traversal accessors: `.key`, `.0`, `.:50`
    accessors: Vec<Accessor>,
    /// Arithmetic operations: `+10`, `-5`, `*2`, `/60`
    arithmetic: Vec<(ArithOp, f64)>,
    /// Date formatting via strftime: `>%Y-%m-%d`
    date_format: Option<String>,
}

/// Object traversal / string slicing accessor
#[derive(Debug)]
enum Accessor {
    /// `.fieldname` — object key lookup
    Key(String),
    /// `.0`, `.-1` — array index
    Index(i64),
    /// `.:50`, `.5:10`, `.-20:` — string/array slice
    Slice {
        start: Option<i64>,
        end: Option<i64>,
    },
}

/// Arithmetic operator
#[derive(Debug, Clone, Copy)]
enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Format specifier after the closing `)`
#[derive(Debug)]
struct FormatSpec {
    /// `#` flag — modifier for j/l/B/D/S/U
    hash: bool,
    /// `0` flag — zero-pad
    zero_pad: bool,
    /// Minimum field width
    width: Option<usize>,
    /// Precision (`.N`)
    precision: Option<usize>,
    /// Type character
    type_char: FormatType,
}

/// All supported format type characters
#[derive(Debug, PartialEq, Eq)]
enum FormatType {
    /// `s` — string
    String,
    /// `d` — integer
    Integer,
    /// `f` / `F` — float
    Float,
    /// `j` — JSON (`#j` = pretty)
    Json,
    /// `l` — list (`, ` separator; `#l` = newline)
    List,
    /// `B` — human-readable bytes (`#B` = 1024-based)
    Bytes,
    /// `D` — decimal suffix (K, M, G; `#D` = 1024-based)
    Decimal,
    /// `S` — filename-safe (`#S` = ASCII-only)
    Sanitize,
    /// `q` — shell-quoted
    Quote,
    /// `h` — HTML-escaped
    Html,
    /// `c` — int → char
    Char,
    /// `U` — Unicode NFC (`#U` = NFD)
    Unicode,
    /// `R` — thousands separator
    Replace,
}

/// Parsed field content: (fallback names, optional replacement, optional default)
type FieldContentParts = (Vec<FieldRef>, Option<String>, Option<String>);

/// Render-time context for computed fields
pub(in crate::orchestrator) struct RenderContext {
    pub epoch: i64,
    pub autonumber: u64,
}

/// Errors that can occur during template parsing or rendering
#[derive(Debug, thiserror::Error)]
pub(in crate::orchestrator) enum TemplateError {
    #[error("Unclosed template field at position {position}")]
    UnclosedField { position: usize },

    #[error("Empty field name at position {position}")]
    EmptyFieldName { position: usize },

    #[error("Invalid format specifier '{specifier}'")]
    InvalidFormatSpecifier { specifier: String },
}
