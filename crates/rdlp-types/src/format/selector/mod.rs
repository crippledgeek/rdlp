//! Format selection DSL -- yt-dlp-compatible expression parser and selector.
//!
//! Grammar notation uses `"..."` for literal tokens and backticks for identifiers.
//! Double-quoted strings in the grammar below are literal tokens, not doc links.
//!
//! Grammar (operator precedence: `+` > `/` > `,`):
//!   `selector_list` = selector ("," selector)*        -- comma: multiple format sets
//!   selector      = stream ("/" stream)*            -- fallback chain
//!   stream        = atom ("+" atom)?                -- video+audio merge
//!   atom          = (token | "(" `selector_list` ")") filter*
//!                 | filter+                          -- implicit "best"
//!   token         = keyword \[`.N`\] | extension | `format_id`
//!   keyword       = "best" | "worst" | "b" | "w"
//!                 | "bestvideo" | "bv" | "bv*"
//!                 | "bestaudio" | "ba" | "ba*"
//!                 | "worstvideo" | "wv"
//!                 | "worstaudio" | "wa"
//!                 | "all" | "mergeall"
//!   extension     = "mp4" | "webm" | "flv" | "3gp" | "m4a"
//!                 | "mp3" | "ogg" | "wav" | "aac"   -- bare `mp4` = best[ext=mp4]
//!   filter        = `[` field op value `]`
//!   field         = "height" | "width" | "ext" | "vcodec" | "acodec"
//!                 | "fps" | "tbr" | "vbr" | "abr" | "asr"
//!                 | "filesize" | "protocol" | "`format_id`"
//!                 | <`any_field_name`>        -- arbitrary fields via Other(String)
//!   op            = "<=" | ">=" | "!=" | "<" | ">" | "="
//!                 | "^=" | "$=" | "*=" | "~="               -- string ops
//!                 | "!^=" | "!$=" | "!*=" | "!~="           -- negated string ops
//!                 | "<=?" | ">=?" | "!=?" | "<?" | ">?" | "=?"  -- non-fatal ops
//!   value         = `size_literal` | number | string

mod error;
mod eval;
mod parser;
mod size;
pub mod sort;

#[cfg(test)]
mod tests;

pub use error::FormatSelectError;
pub use sort::FormatSorter;

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
///
/// // Comma (multiple targets)
/// let sel = FormatSelector::parse("best,bestaudio").unwrap();
///
/// // Parenthesised groups
/// let sel = FormatSelector::parse("(bestvideo+bestaudio)/best").unwrap();
/// ```
#[derive(Debug)]
pub struct FormatSelector {
    expression: String,
    /// Top-level comma-separated selector nodes.
    ///
    /// Each node represents an independent download target (like yt-dlp's
    /// comma-separated format lists). `select()` evaluates only the first node
    /// for backward compatibility; `select_all()` evaluates every node.
    selectors: Vec<SelectorNode>,
}

/// A single selector node in the comma-separated top level.
///
/// Holds a fallback chain: the first `FormatSpec` whose `select_spec` call
/// returns a non-empty result wins.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SelectorNode {
    pub(super) fallbacks: Vec<FormatSpec>,
}

/// A single format specification within a fallback chain.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FormatSpec {
    /// A single selector: base token with optional filters.
    ///
    /// When `base` is `FormatToken::Group`, the evaluator evaluates the inner
    /// nodes and applies `filters` as outer filters on the group result.
    Single(Selector),
    /// A video+audio merge: `video + audio`.
    Merge { video: Selector, audio: Selector },
    /// A parenthesised group with optional outer filters.
    ///
    /// Currently unused by the parser (groups are encoded as
    /// `FormatSpec::Single` with `FormatToken::Group` as the base token).
    /// Kept as an explicit variant for potential future direct construction.
    #[allow(dead_code)]
    Group {
        inner: Vec<SelectorNode>,
        filters: Vec<Filter>,
    },
}

/// A base selector with optional filters.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Selector {
    pub(super) base: FormatToken,
    pub(super) filters: Vec<Filter>,
}

/// The base token determining which formats are candidates.
///
/// This replaces the old `BaseSelector` flat enum with a richer structure
/// that supports star variants, `.N` nth-best suffixes, `all`/`mergeall`,
/// bare-extension shorthands, arbitrary format IDs, and parenthesised groups.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FormatToken {
    /// A quality+stream keyword such as `bestvideo`, `bv*`, `ba.3`, etc.
    Keyword {
        quality: Quality,
        stream_type: StreamType,
        /// `*` variant — allows combined formats (e.g. `bv*` includes combined streams).
        modified: bool,
        /// `.N` suffix — select the Nth-best match (1-based). `None` = first best.
        nth: Option<u32>,
    },
    /// `all` — yield every candidate format (for use in pipelines).
    All,
    /// `mergeall` — merge all formats into a single output.
    MergeAll,
    /// A specific format ID, e.g. `c720` or `137`.
    FormatId(String),
    /// Bare extension shorthand, e.g. `mp4` → `best[ext=mp4]`.
    Extension(String),
    /// A parenthesised group of selector nodes, e.g. `(bv+ba)`.
    ///
    /// Outer filters on the group atom are stored on the enclosing `Selector`,
    /// not here.  The `inner` vec holds the parsed selector list.
    Group(Vec<SelectorNode>),
}

/// Quality direction for a `FormatToken::Keyword`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Quality {
    Best,
    Worst,
}

/// Stream type constraint for a `FormatToken::Keyword`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamType {
    /// Any / combined stream (e.g. `best`, `worst`, `b`, `w`).
    Any,
    /// Video stream (e.g. `bestvideo`, `bv`, `bv*`).
    Video,
    /// Audio stream (e.g. `bestaudio`, `ba`, `ba*`).
    Audio,
}

/// A filter condition applied to a format field.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Filter {
    pub(super) field: FilterField,
    pub(super) op: FilterOp,
    pub(super) value: FilterValue,
    /// If `true`, the filter is negated (e.g. `!^=`, `!$=`, `!*=`, `!~=`).
    pub(super) negated: bool,
    /// If `true`, skip this format instead of failing the whole expression
    /// when the field is absent (non-fatal `?` operators like `<=?`).
    pub(super) non_fatal: bool,
}

/// Format fields that can be filtered on.
///
/// Known fields are represented by named variants for efficient dispatch in
/// the evaluator. Arbitrary field names (for forward compatibility) are
/// captured by `Other(String)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Other is produced by the future parser extension (task 4)
pub(super) enum FilterField {
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
    /// Any field not in the fixed list above.
    Other(String),
}

/// Comparison operators for filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // String ops (StartsWith, EndsWith, Contains, Regex) added by task 4
pub(super) enum FilterOp {
    // Numeric / generic equality-ordering ops
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // String-specific ops
    /// `^=` — value starts with the filter string.
    StartsWith,
    /// `$=` — value ends with the filter string.
    EndsWith,
    /// `*=` — value contains the filter string.
    Contains,
    /// `~=` — value matches the filter regex.
    Regex,
}

/// A filter value: a size literal, a number, or a string.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FilterValue {
    /// A parsed size literal such as `500M` or `1.5GiB`.
    Size(u64),
    /// A bare floating-point number.
    Number(f64),
    /// A string value (extension names, codec names, format IDs, etc.).
    Text(String),
}

impl FormatSelector {
    /// Parse a format selection expression.
    ///
    /// Returns an error if the expression is empty or contains invalid syntax.
    ///
    /// # Errors
    ///
    /// Returns [`FormatSelectError::Parse`] if the expression is empty or
    /// contains unrecognised syntax.
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

        let selectors =
            parser::parse_expression
                .parse(to_parse)
                .map_err(|e| FormatSelectError::Parse {
                    // winnow's ParseError::offset() gives the byte offset of the failure.
                    position: e.offset(),
                    message: e.inner().to_string(),
                    input: expression.to_string(),
                })?;

        Ok(Self {
            expression: expression.to_string(),
            selectors,
        })
    }

    /// Get the original expression string.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Select formats from the available list.
    ///
    /// Evaluates the **first** comma-separated selector node (backward
    /// compatible with pre-comma behavior).  For each node, tries each
    /// fallback in order and returns the first non-empty result.
    ///
    /// Returns 1 format for single selectors, 2 for merge (`video+audio`),
    /// or 0 if nothing matches.
    ///
    /// Use [`Self::select_all`] to evaluate every comma-separated node.
    pub fn select<'a>(&self, formats: &'a [Format]) -> Vec<&'a Format> {
        self.selectors
            .first()
            .map_or_else(Vec::new, |node| eval_node(node, formats, None))
    }

    /// Select formats for **all** comma-separated nodes.
    ///
    /// Each entry in the returned `Vec` corresponds to one comma-separated
    /// selector node. Nodes that produce no matches return an empty inner `Vec`.
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::FormatSelector;
    ///
    /// // (formats omitted for brevity — in real usage pass a non-empty slice)
    /// let sel = FormatSelector::parse("best,bestaudio").unwrap();
    /// let results: Vec<Vec<_>> = sel.select_all(&[]);
    /// assert_eq!(results.len(), 2);
    /// ```
    pub fn select_all<'a>(&self, formats: &'a [Format]) -> Vec<Vec<&'a Format>> {
        self.selectors
            .iter()
            .map(|node| eval_node(node, formats, None))
            .collect()
    }

    /// Select formats using a custom sorter (for `-S` flag support).
    ///
    /// Sorts the format pool using `sorter` before evaluating the selector
    /// expression.  Quality-based keywords (`best`, `bestvideo`, `bestaudio`,
    /// etc.) then resolve according to the custom sort order rather than the
    /// built-in ranking heuristics.
    ///
    /// Evaluates only the **first** comma-separated node (same as [`select`]).
    ///
    /// [`select`]: FormatSelector::select
    pub fn select_with_sorter<'a>(
        &self,
        formats: &'a [Format],
        sorter: &sort::FormatSorter,
    ) -> Vec<&'a Format> {
        // The sorter is plumbed through `eval_node` → `select_spec` → `select_one`,
        // where it replaces the local `rank_formats` heuristic for `best` /
        // `worst` / nth picks. No pre-sort + clone-into-owned dance is
        // required: the sorter's `compare` directly drives `max_by` / `min_by`
        // / `sort_by` inside the eval engine, so the returned references
        // borrow the original input slice's lifetime.
        self.selectors
            .first()
            .map_or_else(Vec::new, |node| eval_node(node, formats, Some(sorter)))
    }
}

/// Evaluate a single `SelectorNode` against the format list.
///
/// Tries each fallback spec in order and returns the result of the first
/// non-empty match (or an empty `Vec` if nothing matches).
pub(super) fn eval_node<'a>(
    node: &SelectorNode,
    formats: &'a [Format],
    sorter: Option<&sort::FormatSorter>,
) -> Vec<&'a Format> {
    for spec in &node.fallbacks {
        let result = eval::select_spec(spec, formats, sorter);
        if !result.is_empty() {
            return result;
        }
    }
    Vec::new()
}

/// Parse a format expression and select matching formats using default sort order.
///
/// Convenience wrapper around `FormatSelector::parse(expr)?.select(formats)`.
///
/// # Errors
///
/// Returns [`FormatSelectError`] if `expr` is empty or contains invalid syntax.
///
/// # Examples
///
/// ```
/// use rdlp_types::{Codec, Format, format_select};
/// use rdlp_types::protocol::DownloadProtocol;
///
/// let mut f = Format::new("1", "https://example.com/v", "mp4", DownloadProtocol::Https);
/// f.vcodec = Codec::from("h264");
/// f.acodec = Codec::from("aac");
/// let formats = vec![f];
/// let selected = format_select("best", &formats).unwrap();
/// assert!(!selected.is_empty());
/// ```
pub fn format_select<'a>(
    expr: &str,
    formats: &'a [Format],
) -> Result<Vec<&'a Format>, FormatSelectError> {
    let selector = FormatSelector::parse(expr)?;
    Ok(selector.select(formats))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    clippy::missing_docs_in_private_items,
    clippy::indexing_slicing
)]
mod integration_tests {
    use super::*;
    use crate::format::Codec;
    use crate::protocol::DownloadProtocol;

    fn make_test_formats() -> Vec<Format> {
        let mut v = Format::new(
            "v1080",
            "https://example.com/v1080",
            "mp4",
            DownloadProtocol::Https,
        );
        v.vcodec = Codec::from("h264".to_string());
        v.acodec = Codec::Absent;
        v.height = Some(1080);

        let mut a = Format::new(
            "a128",
            "https://example.com/a128",
            "m4a",
            DownloadProtocol::Https,
        );
        a.vcodec = Codec::Absent;
        a.acodec = Codec::from("aac".to_string());
        a.abr = Some(128.0);

        let mut combined = Format::new(
            "c720",
            "https://example.com/c720",
            "mp4",
            DownloadProtocol::Https,
        );
        combined.vcodec = Codec::from("h264".to_string());
        combined.acodec = Codec::from("aac".to_string());
        combined.height = Some(720);
        combined.quality = Some(2);

        vec![v, a, combined]
    }

    #[test]
    fn public_api_format_select() {
        let formats = make_test_formats();
        let result = format_select("bestvideo+bestaudio/best", &formats).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn public_api_sorter() {
        let sorter = FormatSorter::parse("res:720,vcodec:h264").unwrap();
        assert!(!sorter.fields_is_empty());
    }

    #[test]
    fn public_api_error_type() {
        let err = format_select("", &[]).unwrap_err();
        assert!(matches!(err, FormatSelectError::Parse { .. }));
    }

    #[test]
    fn public_api_select_with_sorter() {
        let formats = make_test_formats();
        let selector = FormatSelector::parse("bestvideo").unwrap();
        let sorter = FormatSorter::parse("res").unwrap();
        let result = selector.select_with_sorter(&formats, &sorter);
        assert!(!result.is_empty());
        // The video-only format should be selected.
        assert_eq!(result[0].format_id, "v1080");
    }

    #[test]
    fn public_api_select_all() {
        let formats = make_test_formats();
        let selector = FormatSelector::parse("best,bestaudio").unwrap();
        let results = selector.select_all(&formats);
        assert_eq!(results.len(), 2);
    }
}
