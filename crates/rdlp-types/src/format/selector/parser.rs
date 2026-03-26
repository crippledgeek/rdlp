//! Winnow-based parser for format selection expressions.
//!
//! Converts text expressions like `"bv[height<=720]+ba"` into
//! structured [`FormatSpec`] trees.
//!
//! Grammar (operator precedence: `+` > `/` > `,`):
//!   selector_list = selector ("," selector)*         -- loosest: multiple download targets
//!   selector      = stream ("/" stream)*             -- fallback chain
//!   stream        = atom ("+" atom)*                 -- merge (video+audio)
//!   atom          = (token | "(" selector_list ")") filter*
//!                 | filter+                           -- implicit "best"
//!   token         = keyword [".N"] | format_id
//!   keyword       = "bestvideo*" | "bestaudio*" | "bestvideo" | "bestaudio"
//!                 | "worstvideo" | "worstaudio" | "best" | "worst"
//!                 | "bv*" | "ba*" | "bv" | "ba" | "wv" | "wa" | "b" | "w"
//!                 | "all" | "mergeall"
//!                 | <extension_shorthand>
//!   filter        = "[" field op value "]"

use winnow::ascii::digit1;
use winnow::combinator::{alt, cut_err, delimited, opt, peek, repeat, separated};
use winnow::error::{ContextError, ErrMode};
use winnow::prelude::*;
use winnow::token::take_while;

use super::{
    Filter, FilterField, FilterOp, FilterValue, FormatSpec, FormatToken, Quality, Selector,
    SelectorNode, StreamType,
};

/// Known bare-extension shorthands (checked after keyword matching).
const KNOWN_EXTENSIONS: &[&str] = &[
    "mp4", "webm", "flv", "3gp", "m4a", "mp3", "ogg", "wav", "aac",
];

// ---------------------------------------------------------------------------
// Top-level public entry points
// ---------------------------------------------------------------------------

/// Parse a complete format expression: comma-separated `SelectorNode`s.
///
/// Returns a `Vec<SelectorNode>` representing independent download targets.
pub(super) fn parse_expression(input: &mut &str) -> ModalResult<Vec<SelectorNode>> {
    separated(1.., parse_selector_node, ',').parse_next(input)
}

// ---------------------------------------------------------------------------
// Grammar levels (loosest to tightest)
// ---------------------------------------------------------------------------

/// Parse a selector node: a fallback chain of streams separated by `/`.
fn parse_selector_node(input: &mut &str) -> ModalResult<SelectorNode> {
    let fallbacks: Vec<FormatSpec> = separated(1.., parse_stream, '/').parse_next(input)?;
    Ok(SelectorNode { fallbacks })
}

/// Parse a stream: one or two atoms joined by `+` (merge).
///
/// Only exactly two atoms are supported as a merge (video + audio), matching
/// the existing `FormatSpec::Merge { video, audio }` structure.  Additional
/// `+` tokens after the second atom are left unconsumed and will cause the
/// surrounding parser to fail gracefully, as per yt-dlp semantics.
fn parse_stream(input: &mut &str) -> ModalResult<FormatSpec> {
    let first = parse_atom(input)?;
    if opt('+').parse_next(input)?.is_some() {
        let second = parse_atom(input)?;
        Ok(FormatSpec::Merge {
            video: first,
            audio: second,
        })
    } else {
        Ok(FormatSpec::Single(first))
    }
}

/// Parse a single atom: either a parenthesised group or a token with filters,
/// or a bare filter list (implicit `best`).
///
/// ```text
/// atom = (token | "(" selector_list ")") filter*
///      | filter+                                    -- implicit best
/// ```
fn parse_atom(input: &mut &str) -> ModalResult<Selector> {
    // Case 1: parenthesised group, optionally followed by outer filters.
    if opt(peek('(')).parse_next(input)?.is_some() {
        return parse_group_atom(input);
    }

    // Case 2: bare `[` without a preceding token → implicit `best`.
    if opt(peek('[')).parse_next(input)?.is_some() {
        let implicit_best = FormatToken::Keyword {
            quality: Quality::Best,
            stream_type: StreamType::Any,
            modified: false,
            nth: None,
        };
        let filters: Vec<Filter> = repeat(1.., parse_filter).parse_next(input)?;
        return Ok(Selector {
            base: implicit_best,
            filters,
        });
    }

    // Case 3: normal token followed by optional filters.
    let base = parse_format_token(input)?;
    let filters: Vec<Filter> = repeat(0.., parse_filter).parse_next(input)?;
    Ok(Selector { base, filters })
}

/// Parse a parenthesised group atom: `"(" selector_list ")" filter*`.
fn parse_group_atom(input: &mut &str) -> ModalResult<Selector> {
    // Consume the opening paren (we already peeked it).
    let inner_nodes: Vec<SelectorNode> =
        delimited('(', parse_expression, cut_err(')')).parse_next(input)?;
    let filters: Vec<Filter> = repeat(0.., parse_filter).parse_next(input)?;
    Ok(Selector {
        base: FormatToken::Group(inner_nodes),
        filters,
    })
}

// ---------------------------------------------------------------------------
// Token parsing
// ---------------------------------------------------------------------------

/// Parse a base format token.
///
/// Matching order:
///   1. Long-form keywords (before short aliases to avoid prefix collision).
///   2. Short aliases (`bv*` before `bv`, etc.).
///   3. Special keywords (`mergeall` before `all`).
///   4. Known extension shorthands (`mp4`, `webm`, …).
///   5. Fallback bare format ID.
fn parse_format_token(input: &mut &str) -> ModalResult<FormatToken> {
    alt((
        parse_keyword_or_special,
        parse_extension_shorthand,
        parse_format_id,
    ))
    .parse_next(input)
}

/// Check that a keyword is not immediately followed by an alphanumeric character
/// or `_` that would make it part of a longer token.
///
/// Returns `true` if the keyword boundary is clean (end of input or next char
/// is a non-word character), `false` if it is a prefix of a longer identifier.
#[inline]
fn keyword_boundary(input: &str) -> bool {
    input
        .chars()
        .next()
        .map(|c| !c.is_alphanumeric() && c != '_')
        .unwrap_or(true)
}

/// Try to match a keyword literal and assert a clean word boundary after it.
///
/// Returns `Ok((literal_consumed, value))` if the literal matches AND the
/// next character is not alphanumeric / underscore.  Returns a backtrack error
/// otherwise (without consuming any input).
fn keyword_literal<T: Clone>(literal: &'static str, value: T, input: &mut &str) -> ModalResult<T> {
    let snapshot = *input;
    if input.starts_with(literal) {
        let after = &input[literal.len()..];
        // Allow `*` immediately after the word (e.g. `bv*`) — that's part of
        // the keyword itself and is already included in the literal string.
        if keyword_boundary(after) {
            *input = after;
            return Ok(value);
        }
    }
    *input = snapshot;
    Err(ErrMode::Backtrack(ContextError::new()))
}

/// Parse all keyword and special tokens, including the optional `.N` suffix.
fn parse_keyword_or_special(input: &mut &str) -> ModalResult<FormatToken> {
    // Try each keyword in order, longest first to avoid prefix collisions.
    // Each call to `keyword_literal` checks a clean word boundary.
    type KeywordTuple = (Quality, StreamType, bool);

    // Helper macro to avoid repetition.
    macro_rules! kw {
        ($lit:literal, $q:expr, $s:expr, $m:expr) => {
            keyword_literal::<KeywordTuple>($lit, ($q, $s, $m), input)
        };
    }

    let base: Result<KeywordTuple, ErrMode<ContextError>> =
        // Long forms with * first (longest match)
        kw!("bestvideo*", Quality::Best, StreamType::Video, true)
            .or_else(|_| kw!("bestaudio*", Quality::Best, StreamType::Audio, true))
            .or_else(|_| kw!("worstvideo*", Quality::Worst, StreamType::Video, true))
            .or_else(|_| kw!("worstaudio*", Quality::Worst, StreamType::Audio, true))
            .or_else(|_| kw!("bestvideo", Quality::Best, StreamType::Video, false))
            .or_else(|_| kw!("bestaudio", Quality::Best, StreamType::Audio, false))
            .or_else(|_| kw!("worstvideo", Quality::Worst, StreamType::Video, false))
            .or_else(|_| kw!("worstaudio", Quality::Worst, StreamType::Audio, false))
            .or_else(|_| kw!("best*", Quality::Best, StreamType::Any, true))
            .or_else(|_| kw!("worst*", Quality::Worst, StreamType::Any, true))
            .or_else(|_| kw!("best", Quality::Best, StreamType::Any, false))
            .or_else(|_| kw!("worst", Quality::Worst, StreamType::Any, false))
            // Short forms with * first
            .or_else(|_| kw!("bv*", Quality::Best, StreamType::Video, true))
            .or_else(|_| kw!("ba*", Quality::Best, StreamType::Audio, true))
            .or_else(|_| kw!("wv*", Quality::Worst, StreamType::Video, true))
            .or_else(|_| kw!("wa*", Quality::Worst, StreamType::Audio, true))
            .or_else(|_| kw!("b*", Quality::Best, StreamType::Any, true))
            .or_else(|_| kw!("w*", Quality::Worst, StreamType::Any, true))
            .or_else(|_| kw!("bv", Quality::Best, StreamType::Video, false))
            .or_else(|_| kw!("ba", Quality::Best, StreamType::Audio, false))
            .or_else(|_| kw!("wv", Quality::Worst, StreamType::Video, false))
            .or_else(|_| kw!("wa", Quality::Worst, StreamType::Audio, false))
            .or_else(|_| kw!("b", Quality::Best, StreamType::Any, false))
            .or_else(|_| kw!("w", Quality::Worst, StreamType::Any, false));

    if let Ok((quality, stream_type, modified)) = base {
        // Optional `.N` suffix after a keyword.
        let nth = parse_nth_suffix(input)?;
        return Ok(FormatToken::Keyword {
            quality,
            stream_type,
            modified,
            nth,
        });
    }

    // Special non-quality keywords — no `.N` suffix, also need boundary check.
    keyword_literal("mergeall", FormatToken::MergeAll, input)
        .or_else(|_| keyword_literal("all", FormatToken::All, input))
}

/// Parse an optional `.N` suffix (e.g. `.3`), returning `Some(N)` or `None`.
///
/// Only consumes input if the dot is immediately followed by digits.
/// Leaves the input unchanged when no `.N` suffix is present.
fn parse_nth_suffix(input: &mut &str) -> ModalResult<Option<u32>> {
    // Peek ahead: check whether the next two characters are '.' followed by a digit.
    // If yes, consume both; otherwise leave input untouched.
    if !input.starts_with('.') {
        return Ok(None);
    }
    // Check that what follows the dot is at least one ASCII digit.
    let after_dot = &input[1..];
    if after_dot.starts_with(|c: char| c.is_ascii_digit()) {
        // Consume the dot.
        '.'.parse_next(input)?;
        // Consume the digit sequence.
        let digits: &str = digit1.parse_next(input)?;
        let n: u32 = digits
            .parse()
            .map_err(|_| ErrMode::Backtrack(ContextError::new()))?;
        Ok(Some(n))
    } else {
        // Dot is present but not followed by digits — leave it for other parsers.
        Ok(None)
    }
}

/// Try to match a known bare-extension shorthand.
///
/// Extensions are matched only when the word is not followed by characters
/// that would extend it into an arbitrary format ID (alpha-numeric or `_`).
fn parse_extension_shorthand(input: &mut &str) -> ModalResult<FormatToken> {
    for &ext in KNOWN_EXTENSIONS {
        // Match `ext` as a prefix, then verify the next char terminates the word.
        let matched = input.starts_with(ext)
            && input[ext.len()..]
                .chars()
                .next()
                .map(|c| !c.is_alphanumeric() && c != '_' && c != '.')
                .unwrap_or(true);
        if matched {
            // Advance the input past the extension.
            *input = &input[ext.len()..];
            return Ok(FormatToken::Extension(ext.to_string()));
        }
    }
    Err(ErrMode::Backtrack(ContextError::new()))
}

/// Parse a literal format ID (anything except whitespace and special chars).
fn parse_format_id(input: &mut &str) -> ModalResult<FormatToken> {
    take_while(1.., |c: char| {
        !c.is_whitespace() && !matches!(c, '+' | '/' | '[' | ']' | ',' | '(' | ')')
    })
    .map(|id: &str| FormatToken::FormatId(id.to_string()))
    .parse_next(input)
}

// ---------------------------------------------------------------------------
// Filter parsing (unchanged from previous implementation)
// ---------------------------------------------------------------------------

/// Parse a single `[field op value]` filter.
fn parse_filter(input: &mut &str) -> ModalResult<Filter> {
    // yt-dlp allows spaces before `[`: `best [filesize = 1000]`
    let _ = take_while(0.., ' ').parse_next(input)?;
    delimited('[', parse_filter_inner, cut_err(']')).parse_next(input)
}

/// Parse the inside of a filter: `field op value`.
/// yt-dlp allows spaces around all elements: `filesize <= ? 3000`
fn parse_filter_inner(input: &mut &str) -> ModalResult<Filter> {
    let _ = take_while(0.., ' ').parse_next(input)?;
    let field = parse_filter_field(input)?;
    let _ = take_while(0.., ' ').parse_next(input)?;
    let (op, negated, non_fatal) = parse_filter_op(input)?;
    let _ = take_while(0.., ' ').parse_next(input)?;
    let value = parse_filter_value(input)?;
    let _ = take_while(0.., ' ').parse_next(input)?;
    Ok(Filter {
        field,
        op,
        value,
        negated,
        non_fatal,
    })
}

/// Parse a filter field name.
///
/// Accepts any identifier (letters, digits, underscores). Known names map
/// to typed `FilterField` variants; unknown names use `FilterField::Other`.
/// This matches yt-dlp which allows arbitrary fields like `aspect_ratio`,
/// `language_preference`, `source_preference`, etc.
fn parse_filter_field(input: &mut &str) -> ModalResult<FilterField> {
    // Field names must start with a letter or underscore (not a digit).
    // This rejects `[720<height]` where the value is on the left.
    if input.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(ErrMode::Backtrack(ContextError::new()));
    }
    let name: &str = take_while(1.., |c: char| c.is_alphanumeric() || c == '_')
        .parse_next(input)?;
    let field = match name {
        "height" => FilterField::Height,
        "width" => FilterField::Width,
        "filesize" | "filesize_approx" => FilterField::Filesize,
        "format_id" => FilterField::FormatId,
        "format_note" => FilterField::Other(name.to_owned()),
        "protocol" => FilterField::Protocol,
        "vcodec" => FilterField::Vcodec,
        "acodec" => FilterField::Acodec,
        "ext" => FilterField::Ext,
        "fps" => FilterField::Fps,
        "tbr" => FilterField::Tbr,
        "vbr" => FilterField::Vbr,
        "abr" => FilterField::Abr,
        "asr" => FilterField::Asr,
        _ => FilterField::Other(name.to_owned()),
    };
    Ok(field)
}

/// Parse a comparison or string operator.
///
/// Returns `(op, negated, non_fatal)`.
///
/// Parsing order: longest tokens first to prevent prefix ambiguity.
///   3-char negated string ops: `!~=`, `!*=`, `!^=`, `!$=`
///   2-char comparison ops: `<=`, `>=`, `!=`
///   2-char string ops:     `~=`, `*=`, `^=`, `$=`
///   1-char ops:            `<`, `>`, `=`
///
/// After matching the operator token, an optional trailing `?` sets `non_fatal`.
fn parse_filter_op(input: &mut &str) -> ModalResult<(FilterOp, bool, bool)> {
    let (op, negated) = alt((
        // 3-char negated string ops — must come before 2-char ops.
        "!~=".value((FilterOp::Regex, true)),
        "!*=".value((FilterOp::Contains, true)),
        "!^=".value((FilterOp::StartsWith, true)),
        "!$=".value((FilterOp::EndsWith, true)),
        // 2-char comparison ops.
        "<=".value((FilterOp::Le, false)),
        ">=".value((FilterOp::Ge, false)),
        "!=".value((FilterOp::Ne, false)),
        // 2-char string ops.
        "~=".value((FilterOp::Regex, false)),
        "*=".value((FilterOp::Contains, false)),
        "^=".value((FilterOp::StartsWith, false)),
        "$=".value((FilterOp::EndsWith, false)),
        // 1-char ops — must come last.
        "<".value((FilterOp::Lt, false)),
        ">".value((FilterOp::Gt, false)),
        "=".value((FilterOp::Eq, false)),
    ))
    .parse_next(input)?;

    // Optional trailing `?` marks the filter as non-fatal.
    let non_fatal = opt('?').parse_next(input)?.is_some();

    Ok((op, negated, non_fatal))
}

/// Parse a filter value — try size literal first, then number, then text.
fn parse_filter_value(input: &mut &str) -> ModalResult<FilterValue> {
    take_while(1.., |c: char| c != ']')
        .map(|raw: &str| {
            let raw = raw.trim();
            // Try size literal first (e.g. `500M`, `1.5GiB`).
            if let Some(bytes) = super::size::parse_size(raw) {
                return FilterValue::Size(bytes);
            }
            // Then try bare number.
            if let Ok(n) = raw.parse::<f64>() {
                return FilterValue::Number(n);
            }
            FilterValue::Text(raw.to_string())
        })
        .parse_next(input)
}
