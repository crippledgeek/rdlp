//! Winnow-based parser for format selection expressions.
//!
//! Converts text expressions like `"bv[height<=720]+ba"` into
//! structured [`FormatSpec`] trees.

use winnow::combinator::{alt, delimited, opt, repeat, separated};
use winnow::prelude::*;
use winnow::token::take_while;

use super::{BaseSelector, Filter, FilterField, FilterOp, FilterValue, FormatSpec, Selector};

/// Parse a complete format expression: `format_spec ( "/" format_spec )*`
pub(super) fn parse_expression(input: &mut &str) -> ModalResult<Vec<FormatSpec>> {
    separated(1.., parse_format_spec, '/').parse_next(input)
}

/// Parse a format spec: `selector` or `selector "+" selector`
fn parse_format_spec(input: &mut &str) -> ModalResult<FormatSpec> {
    let first = parse_selector(input)?;
    if opt('+').parse_next(input)?.is_some() {
        let second = parse_selector(input)?;
        Ok(FormatSpec::Merge {
            video: first,
            audio: second,
        })
    } else {
        Ok(FormatSpec::Single(first))
    }
}

/// Parse a selector: `base_name filter*`
fn parse_selector(input: &mut &str) -> ModalResult<Selector> {
    let base = parse_base_selector(input)?;
    let filters: Vec<Filter> = repeat(0.., parse_filter).parse_next(input)?;
    Ok(Selector { base, filters })
}

/// Parse a base selector keyword or format ID.
fn parse_base_selector(input: &mut &str) -> ModalResult<BaseSelector> {
    alt((
        "bestvideo*".value(BaseSelector::BestVideoStar),
        "bestaudio*".value(BaseSelector::BestAudioStar),
        "bestvideo".value(BaseSelector::BestVideo),
        "bestaudio".value(BaseSelector::BestAudio),
        "worstvideo".value(BaseSelector::WorstVideo),
        "worstaudio".value(BaseSelector::WorstAudio),
        "best".value(BaseSelector::Best),
        "worst".value(BaseSelector::Worst),
        "bv*".value(BaseSelector::BestVideoStar),
        "ba*".value(BaseSelector::BestAudioStar),
        "bv".value(BaseSelector::BestVideo),
        "ba".value(BaseSelector::BestAudio),
        "wv".value(BaseSelector::WorstVideo),
        "wa".value(BaseSelector::WorstAudio),
        "b".value(BaseSelector::Best),
        "w".value(BaseSelector::Worst),
        parse_format_id,
    ))
    .parse_next(input)
}

/// Parse a literal format ID (anything except whitespace and special chars).
fn parse_format_id(input: &mut &str) -> ModalResult<BaseSelector> {
    take_while(1.., |c: char| {
        !c.is_whitespace() && !matches!(c, '+' | '/' | '[' | ']')
    })
    .map(|id: &str| BaseSelector::FormatId(id.to_string()))
    .parse_next(input)
}

/// Parse a single `[field op value]` filter.
fn parse_filter(input: &mut &str) -> ModalResult<Filter> {
    delimited('[', parse_filter_inner, ']').parse_next(input)
}

/// Parse the inside of a filter: `field op value`.
fn parse_filter_inner(input: &mut &str) -> ModalResult<Filter> {
    let field = parse_filter_field(input)?;
    let op = parse_filter_op(input)?;
    let value = parse_filter_value(input)?;
    Ok(Filter { field, op, value })
}

/// Parse a filter field name.
fn parse_filter_field(input: &mut &str) -> ModalResult<FilterField> {
    alt((
        "height".value(FilterField::Height),
        "width".value(FilterField::Width),
        "filesize".value(FilterField::Filesize),
        "format_id".value(FilterField::FormatId),
        "protocol".value(FilterField::Protocol),
        "vcodec".value(FilterField::Vcodec),
        "acodec".value(FilterField::Acodec),
        "ext".value(FilterField::Ext),
        "fps".value(FilterField::Fps),
        "tbr".value(FilterField::Tbr),
        "vbr".value(FilterField::Vbr),
        "abr".value(FilterField::Abr),
        "asr".value(FilterField::Asr),
    ))
    .parse_next(input)
}

/// Parse a comparison operator (two-char operators first to avoid prefix ambiguity).
fn parse_filter_op(input: &mut &str) -> ModalResult<FilterOp> {
    alt((
        "<=".value(FilterOp::Le),
        ">=".value(FilterOp::Ge),
        "!=".value(FilterOp::Ne),
        "<".value(FilterOp::Lt),
        ">".value(FilterOp::Gt),
        "=".value(FilterOp::Eq),
    ))
    .parse_next(input)
}

/// Parse a filter value -- try as number first, fall back to text.
fn parse_filter_value(input: &mut &str) -> ModalResult<FilterValue> {
    take_while(1.., |c: char| c != ']')
        .map(|raw: &str| {
            let raw = raw.trim();
            if let Ok(n) = raw.parse::<f64>() {
                FilterValue::Number(n)
            } else {
                FilterValue::Text(raw.to_string())
            }
        })
        .parse_next(input)
}
