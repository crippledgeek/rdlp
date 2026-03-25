//! Format selection DSL -- yt-dlp-compatible expression parser and selector.
//!
//! Grammar:
//!   expression   = format_spec ( "/" format_spec )*        -- fallback chain
//!   format_spec  = selector ( "+" selector )?              -- video+audio merge
//!   selector     = base_name filter*                       -- base with filters
//!   filter       = "[" field op value "]"
//!   base_name    = "best" | "worst" | "b" | "w"
//!                | "bestvideo" | "bv" | "bv*"
//!                | "bestaudio" | "ba" | "ba*"
//!                | "worstvideo" | "wv"
//!                | "worstaudio" | "wa"
//!                | <format_id>
//!   field        = "height" | "width" | "ext" | "vcodec" | "acodec"
//!                | "fps" | "tbr" | "vbr" | "abr" | "asr"
//!                | "filesize" | "protocol" | "format_id"
//!   op           = "<=" | ">=" | "!=" | "<" | ">" | "="
//!   value        = number | string

mod error;
mod eval;
mod parser;

#[cfg(test)]
mod tests;

pub use error::FormatSelectError;

use super::Format;
use winnow::prelude::*;

/// Parsed format selection expression supporting yt-dlp-compatible syntax.
///
/// # Examples
///
/// ```
/// use rdlp_types::FormatSelector;
///
/// // Basic selectors
/// let sel = FormatSelector::parse("best").unwrap();
/// let sel = FormatSelector::parse("bv+ba").unwrap();
///
/// // Filters
/// let sel = FormatSelector::parse("bv[height<=720]+ba").unwrap();
///
/// // Fallback chains
/// let sel = FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b").unwrap();
/// ```
pub struct FormatSelector {
    expression: String,
    fallbacks: Vec<FormatSpec>,
}

/// A single format specification: either a single selector or a video+audio merge.
#[derive(Debug, Clone, PartialEq)]
enum FormatSpec {
    Single(Selector),
    Merge { video: Selector, audio: Selector },
}

/// A base selector with optional filters.
#[derive(Debug, Clone, PartialEq)]
struct Selector {
    base: BaseSelector,
    filters: Vec<Filter>,
}

/// The base selector type determining which formats are candidates.
#[derive(Debug, Clone, PartialEq)]
enum BaseSelector {
    /// Best combined (video+audio) format
    Best,
    /// Worst combined (video+audio) format
    Worst,
    /// Best video-only format (excludes combined)
    BestVideo,
    /// Best video format (may include combined)
    BestVideoStar,
    /// Worst video-only format
    WorstVideo,
    /// Best audio-only format (excludes combined)
    BestAudio,
    /// Best audio format (may include combined)
    BestAudioStar,
    /// Worst audio-only format
    WorstAudio,
    /// Match a specific format ID
    FormatId(String),
}

/// A filter condition applied to a format field.
#[derive(Debug, Clone, PartialEq)]
struct Filter {
    field: FilterField,
    op: FilterOp,
    value: FilterValue,
}

/// Format fields that can be filtered on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterField {
    Height,
    Width,
    Ext,
    Vcodec,
    Acodec,
    Fps,
    Tbr,
    Vbr,
    Abr,
    Asr,
    Filesize,
    Protocol,
    FormatId,
}

/// Comparison operators for filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A filter value: either a number or a string.
#[derive(Debug, Clone, PartialEq)]
enum FilterValue {
    Number(f64),
    Text(String),
}

impl FormatSelector {
    /// Parse a format selection expression.
    ///
    /// Returns an error if the expression is empty or contains invalid syntax.
    pub fn parse(expression: &str) -> Result<Self, FormatSelectError> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err(FormatSelectError::Parse {
                message: "Empty format expression".to_string(),
                position: 0,
                input: String::new(),
            });
        }

        // Strip trailing '/' (yt-dlp tolerates it)
        let to_parse = expression.strip_suffix('/').unwrap_or(expression);

        let fallbacks = parser::parse_expression
            .parse(to_parse)
            .map_err(|e| FormatSelectError::Parse {
                // winnow's ParseError::offset() gives the byte offset of the failure.
                position: e.offset(),
                message: e.inner().to_string(),
                input: expression.to_string(),
            })?;

        Ok(Self {
            expression: expression.to_string(),
            fallbacks,
        })
    }

    /// Get the original expression string.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Select formats from the available list.
    ///
    /// Tries each fallback in order, returning the first non-empty result.
    /// Returns 1 format for single selectors, 2 for merge (`video+audio`),
    /// or 0 if nothing matches.
    pub fn select<'a>(&self, formats: &'a [Format]) -> Vec<&'a Format> {
        for spec in &self.fallbacks {
            let result = eval::select_spec(spec, formats);
            if !result.is_empty() {
                return result;
            }
        }
        Vec::new()
    }
}
