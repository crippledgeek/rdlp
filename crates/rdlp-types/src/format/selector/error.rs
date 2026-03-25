//! Typed error enum for format selection parsing and evaluation.

use std::fmt;

/// Errors that can occur during format selection parsing.
#[derive(Debug, Clone)]
pub enum FormatSelectError {
    /// A syntax error encountered while parsing the expression.
    Parse {
        /// Human-readable description of what went wrong.
        message: String,
        /// Byte offset in `input` where the error was detected.
        position: usize,
        /// The full input expression that was being parsed.
        input: String,
    },
    /// The expression parsed successfully but matched no formats.
    NoMatch {
        /// The expression that produced no matches.
        expression: String,
    },
    /// A filter referenced a field name that is not recognised.
    UnknownField {
        /// The unrecognised field name.
        field: String,
        /// All field names that are valid in this context.
        available: Vec<&'static str>,
    },
    /// A regex pattern supplied in a filter was syntactically invalid.
    InvalidRegex {
        /// The pattern that failed to compile.
        pattern: String,
        /// The error message returned by the regex engine.
        error: String,
    },
}

impl fmt::Display for FormatSelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse {
                message,
                position,
                input,
            } => {
                writeln!(f, "Parse error at position {position}: {message}")?;
                writeln!(f, "  {input}")?;
                // Render a caret under the error position.
                let caret_offset = (*position).min(input.len());
                write!(f, "  {: <width$}^", "", width = caret_offset)
            }
            Self::NoMatch { expression } => {
                write!(f, "No formats matched expression '{expression}'")
            }
            Self::UnknownField { field, available } => {
                write!(
                    f,
                    "Unknown field '{field}'. Available fields: {}",
                    available.join(", ")
                )
            }
            Self::InvalidRegex { pattern, error } => {
                write!(f, "Invalid regex pattern '{pattern}': {error}")
            }
        }
    }
}

impl std::error::Error for FormatSelectError {}
