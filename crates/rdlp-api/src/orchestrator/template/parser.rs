//! Template parser: turns a `%(field)spec` template string into an `OutputTemplate` AST.

use super::helpers::{
    parse_float_str, parse_number_str, split_fallback_fields, split_last_unescaped,
};
use super::{
    Accessor, ArithOp, FieldContentParts, FieldRef, FieldSpec, FormatSpec, FormatType,
    OutputTemplate, Segment, TemplateError,
};

impl OutputTemplate {
    /// Parse a template string into a ready-to-render `OutputTemplate`.
    #[allow(clippy::indexing_slicing)] // all accesses guarded by bounds checks within the parsing loop
    pub(in crate::orchestrator) fn parse(template: &str) -> Result<Self, TemplateError> {
        let mut segments = Vec::new();
        let chars: Vec<char> = template.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut literal = String::new();

        while i < len {
            if chars[i] == '%' {
                if i + 1 < len && chars[i + 1] == '%' {
                    // `%%` → literal `%`
                    literal.push('%');
                    i += 2;
                    continue;
                }
                if i + 1 < len && chars[i + 1] == '(' {
                    // Flush accumulated literal
                    if !literal.is_empty() {
                        segments.push(Segment::Literal(std::mem::take(&mut literal)));
                    }

                    let start = i;
                    i += 2; // skip `%(`

                    // Find closing `)`
                    let field_start = i;
                    let mut depth = 1;
                    while i < len && depth > 0 {
                        if chars[i] == '(' {
                            depth += 1;
                        } else if chars[i] == ')' {
                            depth -= 1;
                        }
                        if depth > 0 {
                            i += 1;
                        }
                    }
                    if depth > 0 {
                        return Err(TemplateError::UnclosedField { position: start });
                    }

                    let field_content: String = chars[field_start..i].iter().collect();
                    i += 1; // skip `)`

                    if field_content.is_empty() {
                        return Err(TemplateError::EmptyFieldName { position: start });
                    }

                    // Parse inside-parens content
                    let (names, replacement, default) =
                        Self::parse_field_content(&field_content, start)?;

                    // Parse format specifier after `)`
                    let format = Self::parse_format_specifier(&chars, &mut i)?;

                    segments.push(Segment::Field(FieldSpec {
                        names,
                        replacement,
                        default,
                        format,
                    }));
                } else {
                    literal.push(chars[i]);
                    i += 1;
                }
            } else {
                literal.push(chars[i]);
                i += 1;
            }
        }

        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }

        Ok(Self { segments })
    }

    /// Parse the content inside `%( ... )` into field refs, replacement, and default.
    ///
    /// Order: `name[.keys][+/-arith][>strf][,alternate][&replacement][|default]`
    fn parse_field_content(
        content: &str,
        position: usize,
    ) -> Result<FieldContentParts, TemplateError> {
        // Split off `|default` from the end (last unescaped `|`)
        let (before_default, default) = split_last_unescaped(content, '|');

        // Split off `&replacement`
        let (before_replacement, replacement) = split_last_unescaped(before_default, '&');

        // Split on `,` for fallback fields (but not inside strftime like `>%Y,%m`)
        let field_strs = split_fallback_fields(before_replacement);

        let mut names = Vec::new();
        for field_str in &field_strs {
            let field_str = field_str.trim();
            if field_str.is_empty() {
                return Err(TemplateError::EmptyFieldName { position });
            }
            names.push(Self::parse_field_ref(field_str)?);
        }

        if names.is_empty() {
            return Err(TemplateError::EmptyFieldName { position });
        }

        Ok((names, replacement, default))
    }

    /// Parse a single field reference: `name[.keys][+/-arith][>strf]`
    #[allow(clippy::too_many_lines, clippy::indexing_slicing)]
    // parser: all indexing bounded by `i < len` guard; `unnecessarily_wraps` future-proofed
    #[allow(clippy::unnecessary_wraps)]
    fn parse_field_ref(s: &str) -> Result<FieldRef, TemplateError> {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let mut i = 0;

        // Parse base name (alphanumeric, `_`)
        // Note: `-` followed by a digit is arithmetic, not part of the name
        let name_start = i;
        while i < len {
            if chars[i].is_alphanumeric() || chars[i] == '_' {
                i += 1;
            } else if chars[i] == '-' && (i + 1 >= len || !chars[i + 1].is_ascii_digit()) {
                // `-` not followed by digit is part of the name (e.g., `field-name`)
                i += 1;
            } else {
                break;
            }
        }
        let name: String = chars[name_start..i].iter().collect();

        // Parse accessors (`.key`, `.0`, `.:50`)
        let mut accessors = Vec::new();
        while i < len && chars[i] == '.' {
            i += 1; // skip `.`
            // Check for slice syntax: `:` or negative index followed by `:`
            if i < len && chars[i] == ':' {
                // `.:end` — slice from start
                i += 1;
                let end_str = parse_number_str(&chars, &mut i);
                let end = if end_str.is_empty() {
                    None
                } else {
                    Some(end_str.parse::<i64>().unwrap_or(0))
                };
                accessors.push(Accessor::Slice { start: None, end });
                continue;
            }

            // Parse the accessor value (could be key, index, or start of slice)
            let mut acc_content = String::new();
            let has_leading_minus = i < len && chars[i] == '-';

            // Allow leading `-` for negative indices (e.g., `.-1`, `.-5:`)
            if has_leading_minus {
                acc_content.push('-');
                i += 1;
            }

            while i < len
                && chars[i] != '.'
                && chars[i] != '+'
                && chars[i] != '-'
                && chars[i] != '>'
                && chars[i] != '*'
                && chars[i] != '/'
            {
                if chars[i] == ':' {
                    break;
                }
                acc_content.push(chars[i]);
                i += 1;
            }

            // Check if this is a slice: `start:end`
            if i < len && chars[i] == ':' {
                i += 1; // skip `:`
                let end_str = parse_number_str(&chars, &mut i);
                let start = if acc_content.is_empty() {
                    None
                } else {
                    Some(acc_content.parse::<i64>().unwrap_or(0))
                };
                let end = if end_str.is_empty() {
                    None
                } else {
                    Some(end_str.parse::<i64>().unwrap_or(0))
                };
                accessors.push(Accessor::Slice { start, end });
            } else if let Ok(idx) = acc_content.parse::<i64>() {
                // Numeric — array index
                if has_leading_minus || acc_content.chars().all(|c| c.is_ascii_digit()) {
                    accessors.push(Accessor::Index(idx));
                } else {
                    accessors.push(Accessor::Key(acc_content));
                }
            } else {
                accessors.push(Accessor::Key(acc_content));
            }
        }

        // Parse arithmetic: `+10`, `-5`, `*2`, `/60`
        let mut arithmetic = Vec::new();
        while i < len && matches!(chars[i], '+' | '-' | '*' | '/') {
            // Check if this looks like arithmetic (operator followed by digit)
            if i + 1 < len && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '.') {
                let op = match chars[i] {
                    '+' => ArithOp::Add,
                    '-' => ArithOp::Sub,
                    '*' => ArithOp::Mul,
                    '/' => ArithOp::Div,
                    _ => unreachable!(),
                };
                i += 1;
                let num_str = parse_float_str(&chars, &mut i);
                let val: f64 = num_str.parse().unwrap_or(0.0);
                arithmetic.push((op, val));
            } else {
                break;
            }
        }

        // Parse date format: `>%Y-%m-%d`
        let date_format = if i < len && chars[i] == '>' {
            i += 1;
            let fmt: String = chars[i..].iter().collect();
            Some(fmt)
        } else {
            None
        };

        Ok(FieldRef {
            name,
            accessors,
            arithmetic,
            date_format,
        })
    }

    /// Parse the format specifier after `)` — e.g. `s`, `#05d`, `.2f`, `#j`
    #[allow(clippy::indexing_slicing)] // all accesses guarded by `*i < chars.len()` checks
    fn parse_format_specifier(chars: &[char], i: &mut usize) -> Result<FormatSpec, TemplateError> {
        let spec_start = *i;

        // Defaults
        let mut hash = false;
        let mut zero_pad = false;
        let mut width = None;
        let mut precision = None;

        // No more characters — error
        if *i >= chars.len() {
            return Err(TemplateError::InvalidFormatSpecifier {
                specifier: String::new(),
            });
        }

        // Parse `#` flag
        if *i < chars.len() && chars[*i] == '#' {
            hash = true;
            *i += 1;
        }

        // Parse `0` flag (zero-pad)
        if *i < chars.len()
            && chars[*i] == '0'
            && *i + 1 < chars.len()
            && chars[*i + 1].is_ascii_digit()
        {
            zero_pad = true;
            *i += 1;
        }

        // Parse width digits
        let width_start = *i;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
        if *i > width_start {
            let w: String = chars[width_start..*i].iter().collect();
            width = Some(w.parse::<usize>().unwrap_or(0));
        }

        // Parse `.precision`
        if *i < chars.len() && chars[*i] == '.' {
            *i += 1;
            let prec_start = *i;
            while *i < chars.len() && chars[*i].is_ascii_digit() {
                *i += 1;
            }
            if *i > prec_start {
                let p: String = chars[prec_start..*i].iter().collect();
                precision = Some(p.parse::<usize>().unwrap_or(0));
            } else {
                precision = Some(0);
            }
        }

        // Parse type char
        if *i >= chars.len() {
            let spec: String = chars[spec_start..*i].iter().collect();
            return Err(TemplateError::InvalidFormatSpecifier { specifier: spec });
        }

        let type_char = match chars[*i] {
            's' => FormatType::String,
            'd' => FormatType::Integer,
            'f' | 'F' => FormatType::Float,
            'j' => FormatType::Json,
            'l' => FormatType::List,
            'B' => FormatType::Bytes,
            'D' => FormatType::Decimal,
            'S' => FormatType::Sanitize,
            'q' => FormatType::Quote,
            'h' => FormatType::Html,
            'c' => FormatType::Char,
            'U' => FormatType::Unicode,
            'R' => FormatType::Replace,
            _ => {
                let spec: String = chars[spec_start..=*i].iter().collect();
                return Err(TemplateError::InvalidFormatSpecifier { specifier: spec });
            }
        };
        *i += 1;

        Ok(FormatSpec {
            hash,
            zero_pad,
            width,
            precision,
            type_char,
        })
    }
}
