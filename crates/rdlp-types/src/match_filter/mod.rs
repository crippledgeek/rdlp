//! Match filter for skipping videos by metadata.
//!
//! Implements yt-dlp-compatible `--match-filter` syntax:
//!
//! ```text
//! --match-filter "duration > 60 & !is_live"
//! --match-filter "title *= cats"
//! --match-filter "like_count >? 100"   # pass if field missing
//! ```
//!
//! Multiple `--match-filter` flags use OR logic.

mod eval;
mod parser;

#[cfg(test)]
mod tests;

use std::fmt;

/// A parsed match filter expression (one or more `&`-separated conditions).
#[derive(Debug, Clone)]
pub struct MatchFilter {
    /// Conditions joined by `&` (all must pass).
    pub(crate) conditions: Vec<Condition>,
    /// Original input string (for display).
    raw: String,
}

/// A single filter condition.
#[derive(Debug, Clone)]
pub(crate) enum Condition {
    /// Field presence/absence: `field` (present) or `!field` (absent).
    Unary { key: String, negated: bool },
    /// Comparison: `field [!]op[?] value`.
    Comparison {
        key: String,
        op: ComparisonOp,
        negated: bool,
        none_inclusive: bool,
        value: FilterValue,
    },
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComparisonOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `*=` — string contains
    Contains,
    /// `^=` — string starts with
    StartsWith,
    /// `$=` — string ends with
    EndsWith,
    /// `~=` — regex match
    Regex,
}

/// A filter value (right-hand side of comparison).
#[derive(Debug, Clone)]
pub(crate) enum FilterValue {
    Number(f64),
    String(String),
}

/// Error from parsing a match filter expression.
#[derive(Debug, Clone)]
pub struct MatchFilterError {
    /// The original input.
    pub input: String,
    /// Human-readable error message.
    pub message: String,
}

impl fmt::Display for MatchFilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid match filter '{}': {}", self.input, self.message)
    }
}

impl std::error::Error for MatchFilterError {}

impl MatchFilter {
    /// Parse a match filter expression.
    ///
    /// # Errors
    ///
    /// Returns [`MatchFilterError`] if the expression contains invalid syntax
    /// (unknown operator, malformed condition, or parse failure).
    ///
    /// # Examples
    ///
    /// ```
    /// use rdlp_types::match_filter::MatchFilter;
    ///
    /// let filter = MatchFilter::parse("duration > 60 & !is_live").unwrap();
    /// ```
    pub fn parse(input: &str) -> Result<Self, MatchFilterError> {
        let conditions = parser::parse_filter(input).map_err(|msg| MatchFilterError {
            input: input.to_string(),
            message: msg,
        })?;
        Ok(Self {
            conditions,
            raw: input.to_string(),
        })
    }

    /// Evaluate against a JSON value (`InfoDict` serialized as JSON).
    ///
    /// Returns `true` if the filter passes (video should be downloaded).
    #[must_use]
    pub fn evaluate(&self, value: &serde_json::Value) -> bool {
        self.conditions
            .iter()
            .all(|c| eval::evaluate_condition(c, value))
    }

    /// Evaluate against an `InfoDict` by serializing to JSON first.
    #[must_use]
    pub fn evaluate_info(&self, info: &crate::InfoDict) -> bool {
        serde_json::to_value(info).is_ok_and(|value| self.evaluate(&value))
    }
}

impl fmt::Display for MatchFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}
