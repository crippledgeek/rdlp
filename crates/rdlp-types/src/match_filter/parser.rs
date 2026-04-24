//! winnow parser for match filter expressions.
//!
//! Grammar:
//! ```text
//! filter      = condition ("&" condition)*
//! condition   = comparison | unary
//! comparison  = key ws negation? op none_inclusive? ws value
//! unary       = "!"? key
//! key         = [a-z_][a-z0-9_]*
//! op          = "<=" | ">=" | "!=" | "*=" | "^=" | "$=" | "~=" | "<" | ">" | "="
//! value       = quoted_string | bare_value
//! ```

use winnow::ascii::multispace0;
use winnow::combinator::{alt, opt};
use winnow::error::ContextError;
use winnow::prelude::*;
use winnow::token::{take_till, take_while};

use super::{ComparisonOp, Condition, FilterValue};

/// Parse a complete filter expression (multiple `&`-separated conditions).
pub(super) fn parse_filter(input: &str) -> Result<Vec<Condition>, String> {
    // Split on unescaped `&` first (like yt-dlp), then parse each part.
    let parts: Vec<&str> = split_on_unescaped_ampersand(input);

    let mut conditions = Vec::new();
    for part in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let condition =
            parse_condition(trimmed).map_err(|e| format!("in condition '{}': {}", trimmed, e))?;
        conditions.push(condition);
    }

    if conditions.is_empty() {
        return Err("empty filter expression".to_string());
    }

    Ok(conditions)
}

/// Split input on `&` that is not preceded by `\`.
fn split_on_unescaped_ampersand(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = input.as_bytes();

    for i in 0..bytes.len() {
        if bytes[i] == b'&' && (i == 0 || bytes[i - 1] != b'\\') {
            parts.push(&input[start..i]);
            start = i + 1;
        }
    }
    parts.push(&input[start..]);
    parts
}

/// Parse a single condition (comparison or unary).
fn parse_condition(input: &str) -> Result<Condition, String> {
    let mut s = input;

    // Try comparison first (has an operator)
    if let Ok(cond) = try_parse_comparison(&mut s) {
        return Ok(cond);
    }

    // Fall back to unary (field presence/absence)
    parse_unary(input)
}

/// Try to parse a comparison condition.
fn try_parse_comparison(input: &mut &str) -> Result<Condition, String> {
    let original = *input;

    // key
    let key = parse_key
        .parse_next(input)
        .map_err(|_: winnow::error::ErrMode<ContextError>| "expected field name".to_string())?;

    // optional whitespace
    let _ = multispace0::<_, ContextError>.parse_next(input);

    // optional negation prefix on operator
    let negated = opt('!')
        .parse_next(input)
        .map_err(|_: winnow::error::ErrMode<ContextError>| "parse error".to_string())?
        .is_some();

    // optional whitespace after negation
    let _ = multispace0::<_, ContextError>.parse_next(input);

    // operator (must match)
    let op = parse_op
        .parse_next(input)
        .map_err(|_: winnow::error::ErrMode<ContextError>| {
            *input = original;
            "no operator found".to_string()
        })?;

    // optional none-inclusive `?`
    let none_inclusive = opt('?')
        .parse_next(input)
        .map_err(|_: winnow::error::ErrMode<ContextError>| "parse error".to_string())?
        .is_some();

    // optional whitespace before value
    let _ = multispace0::<_, ContextError>.parse_next(input);

    // value
    let raw_value =
        parse_value
            .parse_next(input)
            .map_err(|_: winnow::error::ErrMode<ContextError>| {
                "expected value after operator".to_string()
            })?;

    // Unescape literal `\&` in the value
    let raw_value = raw_value.replace("\\&", "&");

    // Determine if value is numeric
    let value = if let Some(num) = try_parse_numeric(&raw_value) {
        FilterValue::Number(num)
    } else {
        FilterValue::String(raw_value)
    };

    // String operators can't be used with numeric values
    if matches!(
        op,
        ComparisonOp::Contains
            | ComparisonOp::StartsWith
            | ComparisonOp::EndsWith
            | ComparisonOp::Regex
    ) && matches!(value, FilterValue::Number(_))
    {
        // Treat as string anyway for these operators
        let string_val = match &value {
            FilterValue::Number(n) => FilterValue::String(n.to_string()),
            FilterValue::String(s) => FilterValue::String(s.clone()),
        };
        return Ok(Condition::Comparison {
            key: key.to_string(),
            op,
            negated,
            none_inclusive,
            value: string_val,
        });
    }

    Ok(Condition::Comparison {
        key: key.to_string(),
        op,
        negated,
        none_inclusive,
        value,
    })
}

/// Parse a unary condition: `field` or `!field`.
fn parse_unary(input: &str) -> Result<Condition, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty condition".to_string());
    }

    let (negated, key) = if let Some(rest) = trimmed.strip_prefix('!') {
        (true, rest.trim())
    } else {
        (false, trimmed)
    };

    // Validate key format
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!("invalid field name '{key}'"));
    }

    Ok(Condition::Unary {
        key: key.to_string(),
        negated,
    })
}

/// Parse a field key: `[a-z_][a-z0-9_]*`.
fn parse_key<'s>(input: &mut &'s str) -> ModalResult<&'s str> {
    take_while(1.., |c: char| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
    })
    .parse_next(input)
}

/// Parse an operator token (longest match first).
fn parse_op(input: &mut &str) -> ModalResult<ComparisonOp> {
    alt((
        "<=".value(ComparisonOp::Le),
        ">=".value(ComparisonOp::Ge),
        "!=".value(ComparisonOp::Ne),
        "*=".value(ComparisonOp::Contains),
        "^=".value(ComparisonOp::StartsWith),
        "$=".value(ComparisonOp::EndsWith),
        "~=".value(ComparisonOp::Regex),
        "<".value(ComparisonOp::Lt),
        ">".value(ComparisonOp::Gt),
        "=".value(ComparisonOp::Eq),
    ))
    .parse_next(input)
}

/// Parse a value: quoted string or bare value.
fn parse_value(input: &mut &str) -> ModalResult<String> {
    alt((parse_quoted_string, parse_bare_value)).parse_next(input)
}

/// Parse a double-quoted or single-quoted string.
fn parse_quoted_string(input: &mut &str) -> ModalResult<String> {
    let quote = alt(('\"', '\'')).parse_next(input)?;
    let content: String = take_till(0.., move |c: char| c == quote)
        .parse_next(input)?
        .to_string();
    let _ = winnow::token::one_of(move |c: char| c == quote).parse_next(input)?;
    Ok(content)
}

/// Parse a bare (unquoted) value — everything until end of input.
fn parse_bare_value(input: &mut &str) -> ModalResult<String> {
    let val = take_while(1.., |c: char| !c.is_ascii_whitespace() || c == ' ')
        .parse_next(input)?
        .trim()
        .to_string();
    Ok(val)
}

/// Try to parse a string as a numeric value.
///
/// Supports: integers, filesize suffixes (K/M/G), duration (H:M:S).
fn try_parse_numeric(s: &str) -> Option<f64> {
    // Plain number
    if let Ok(n) = s.parse::<f64>() {
        return Some(n);
    }

    // Filesize suffixes: 1K = 1000, 1M = 1_000_000, 1G = 1_000_000_000
    let s_upper = s.to_uppercase();
    if let Some(num_str) = s_upper.strip_suffix('K')
        && let Ok(n) = num_str.parse::<f64>()
    {
        return Some(n * 1000.0);
    }
    if let Some(num_str) = s_upper.strip_suffix('M')
        && let Ok(n) = num_str.parse::<f64>()
    {
        return Some(n * 1_000_000.0);
    }
    if let Some(num_str) = s_upper.strip_suffix('G')
        && let Ok(n) = num_str.parse::<f64>()
    {
        return Some(n * 1_000_000_000.0);
    }
    if let Some(num_str) = s_upper.strip_suffix('B')
        && let Ok(n) = num_str.parse::<f64>()
    {
        return Some(n);
    }

    // Duration: H:M:S or M:S
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        3 => {
            let h = parts[0].parse::<f64>().ok()?;
            let m = parts[1].parse::<f64>().ok()?;
            let sec = parts[2].parse::<f64>().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        2 => {
            let m = parts[0].parse::<f64>().ok()?;
            let sec = parts[1].parse::<f64>().ok()?;
            Some(m * 60.0 + sec)
        }
        _ => None,
    }
}
