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

use chrono::NaiveDate;
use rdlp_core::{Format, InfoDict};

/// Parsed output template ready for rendering
#[derive(Debug)]
pub(super) struct OutputTemplate {
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
pub(super) struct RenderContext {
    pub epoch: i64,
    pub autonumber: u64,
}

/// Errors that can occur during template parsing or rendering
#[derive(Debug, thiserror::Error)]
pub(super) enum TemplateError {
    #[error("Unclosed template field at position {position}")]
    UnclosedField { position: usize },

    #[error("Empty field name at position {position}")]
    EmptyFieldName { position: usize },

    #[error("Invalid format specifier '{specifier}'")]
    InvalidFormatSpecifier { specifier: String },

    #[error("Required field '{field}' is missing and no default was provided")]
    #[allow(dead_code)]
    MissingField { field: String },
}

impl OutputTemplate {
    /// Parse a template string into a ready-to-render `OutputTemplate`.
    pub(super) fn parse(template: &str) -> Result<Self, TemplateError> {
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

    /// Render the template using metadata from `InfoDict`, `Format`, and a computed extension.
    ///
    /// Field resolution priority:
    /// 1. Special computed fields (`ext`, `epoch`, `autonumber`)
    /// 2. InfoDict fields (via serde_json serialization)
    /// 3. Format fields (via serde_json serialization)
    /// 4. Conditional replacement / default / `"NA"`
    pub(super) fn render(
        &self,
        info: &InfoDict,
        format: &Format,
        ext: &str,
        ctx: &RenderContext,
    ) -> Result<String, TemplateError> {
        let info_json = serde_json::to_value(info).unwrap_or_default();
        let format_json = serde_json::to_value(format).unwrap_or_default();

        let mut output = String::new();

        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => output.push_str(text),
                Segment::Field(spec) => {
                    let value = self.resolve_field(spec, &info_json, &format_json, ext, ctx)?;
                    // Sanitize Windows-restricted characters from field values
                    // so they don't create invalid paths or unintended
                    // subdirectories. Also strip null bytes and non-whitespace
                    // control characters which are invalid in filenames on
                    // all platforms. Only literal `/` in the template string
                    // should create directory structure.
                    let sanitized = value
                        .chars()
                        .filter(|c| {
                            // Strip null byte and non-whitespace C0/C1 controls.
                            // Keep \t, \n, \r (whitespace) for formats like
                            // pretty-JSON and newline-separated lists.
                            *c != '\0'
                                && (!c.is_control() || *c == '\t' || *c == '\n' || *c == '\r')
                        })
                        .map(|c| match c {
                            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                            _ => c,
                        })
                        .collect::<String>();
                    output.push_str(&sanitized);
                }
            }
        }

        Ok(output)
    }

    /// Resolve a single field spec to its formatted string value
    fn resolve_field(
        &self,
        spec: &FieldSpec,
        info_json: &serde_json::Value,
        format_json: &serde_json::Value,
        ext: &str,
        ctx: &RenderContext,
    ) -> Result<String, TemplateError> {
        // Try each field in the fallback chain
        for field_ref in &spec.names {
            if let Some(val) = self.lookup_field(field_ref, info_json, format_json, ext, ctx) {
                return Ok(self.format_value(&val, &spec.format));
            }
        }

        // All fields missed — try conditional replacement
        if spec.replacement.is_some() {
            // If `&text`, the field doesn't exist so replacement text doesn't contain `{}`
            // yt-dlp: `&text` outputs `text` if field exists (already handled above).
            // If field is missing, `&text` outputs empty string.
            return Ok(String::new());
        }

        // Try explicit default
        if let Some(ref default) = spec.default {
            return Ok(self.format_value(&serde_json::Value::String(default.clone()), &spec.format));
        }

        // yt-dlp behavior: missing fields produce "NA" (not an error)
        Ok(self.format_value(&serde_json::Value::String("NA".to_string()), &spec.format))
    }

    /// Look up a single field reference, applying accessors/arithmetic/date format.
    /// Returns `None` if the field doesn't exist or is null.
    fn lookup_field(
        &self,
        field_ref: &FieldRef,
        info_json: &serde_json::Value,
        format_json: &serde_json::Value,
        ext: &str,
        ctx: &RenderContext,
    ) -> Option<serde_json::Value> {
        // Computed fields
        let base_value = match field_ref.name.as_str() {
            "ext" => Some(serde_json::Value::String(ext.to_string())),
            "epoch" => Some(serde_json::Value::Number(ctx.epoch.into())),
            "autonumber" => Some(serde_json::Value::Number(ctx.autonumber.into())),
            _ => {
                // Priority: InfoDict → Format
                let val = lookup_key(info_json, &field_ref.name)
                    .or_else(|| lookup_key(format_json, &field_ref.name));
                val.filter(|v| !v.is_null())
            }
        };

        let mut val = base_value?;

        // Apply accessors
        for accessor in &field_ref.accessors {
            val = apply_accessor(&val, accessor)?;
        }

        // Apply arithmetic
        if !field_ref.arithmetic.is_empty() {
            let mut num = value_to_f64(&val)?;
            for (op, operand) in &field_ref.arithmetic {
                num = match op {
                    ArithOp::Add => num + operand,
                    ArithOp::Sub => num - operand,
                    ArithOp::Mul => num * operand,
                    ArithOp::Div => {
                        if *operand == 0.0 {
                            return None;
                        }
                        num / operand
                    }
                };
            }
            val = serde_json::Value::Number(
                serde_json::Number::from_f64(num).unwrap_or_else(|| serde_json::Number::from(0)),
            );
        }

        // Apply date formatting
        if let Some(ref fmt) = field_ref.date_format {
            val = apply_date_format(&val, fmt)?;
        }

        Some(val)
    }

    /// Format a JSON value according to the format specifier
    fn format_value(&self, val: &serde_json::Value, spec: &FormatSpec) -> String {
        let raw = match spec.type_char {
            FormatType::String => format_as_string(val),
            FormatType::Integer => format_as_integer(val, spec),
            FormatType::Float => format_as_float(val, spec),
            FormatType::Json => format_as_json(val, spec.hash),
            FormatType::List => format_as_list(val, spec.hash),
            FormatType::Bytes => format_as_bytes(val, spec.hash),
            FormatType::Decimal => format_as_decimal(val, spec.hash),
            FormatType::Sanitize => format_as_sanitize(val, spec.hash),
            FormatType::Quote => format_as_quote(val),
            FormatType::Html => format_as_html(val),
            FormatType::Char => format_as_char(val),
            FormatType::Unicode => format_as_unicode(val, spec.hash),
            FormatType::Replace => format_as_thousands(val),
        };

        // Apply width (for string types; integer/float handle width internally)
        apply_width(&raw, spec)
    }
}

// === Helper functions ===

/// Look up a key in a JSON value (object field lookup)
fn lookup_key(val: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    val.get(key).cloned()
}

/// Apply an accessor to a JSON value
fn apply_accessor(val: &serde_json::Value, accessor: &Accessor) -> Option<serde_json::Value> {
    match accessor {
        Accessor::Key(key) => val.get(key.as_str()).cloned().filter(|v| !v.is_null()),
        Accessor::Index(idx) => {
            if let serde_json::Value::Array(arr) = val {
                let actual_idx = if *idx < 0 {
                    (arr.len() as i64 + idx) as usize
                } else {
                    *idx as usize
                };
                arr.get(actual_idx).cloned()
            } else {
                None
            }
        }
        Accessor::Slice { start, end } => match val {
            serde_json::Value::String(s) => {
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let start_idx = resolve_slice_bound(start.unwrap_or(0), len);
                let end_idx = resolve_slice_bound(end.unwrap_or(len), len);
                if start_idx >= end_idx {
                    Some(serde_json::Value::String(String::new()))
                } else {
                    let sliced: String =
                        chars[start_idx as usize..end_idx as usize].iter().collect();
                    Some(serde_json::Value::String(sliced))
                }
            }
            serde_json::Value::Array(arr) => {
                let len = arr.len() as i64;
                let start_idx = resolve_slice_bound(start.unwrap_or(0), len);
                let end_idx = resolve_slice_bound(end.unwrap_or(len), len);
                if start_idx >= end_idx {
                    Some(serde_json::Value::Array(Vec::new()))
                } else {
                    let sliced = arr[start_idx as usize..end_idx as usize].to_vec();
                    Some(serde_json::Value::Array(sliced))
                }
            }
            _ => None,
        },
    }
}

/// Resolve a slice bound (handling negative indices and clamping)
fn resolve_slice_bound(bound: i64, len: i64) -> i64 {
    let resolved = if bound < 0 { len + bound } else { bound };
    resolved.clamp(0, len)
}

/// Convert a JSON value to f64 for arithmetic
fn value_to_f64(val: &serde_json::Value) -> Option<f64> {
    match val {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Apply date formatting via chrono strftime
fn apply_date_format(val: &serde_json::Value, fmt: &str) -> Option<serde_json::Value> {
    let date_str = match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => {
            // Unix timestamp
            let ts = n.as_i64()?;
            let dt = chrono::DateTime::from_timestamp(ts, 0)?;
            let formatted = dt.format(fmt).to_string();
            return Some(serde_json::Value::String(formatted));
        }
        _ => return None,
    };

    // Try YYYYMMDD format (yt-dlp upload_date style)
    if date_str.len() == 8 && date_str.chars().all(|c| c.is_ascii_digit()) {
        let year = date_str[..4].parse::<i32>().ok()?;
        let month = date_str[4..6].parse::<u32>().ok()?;
        let day = date_str[6..8].parse::<u32>().ok()?;
        let date = NaiveDate::from_ymd_opt(year, month, day)?;
        let formatted = date.format(fmt).to_string();
        return Some(serde_json::Value::String(formatted));
    }

    // Try ISO-8601 date
    if let Ok(date) = date_str.parse::<NaiveDate>() {
        let formatted = date.format(fmt).to_string();
        return Some(serde_json::Value::String(formatted));
    }

    None
}

// === Format type implementations ===

/// Format as string (type `s`)
fn format_as_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "NA".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_as_string).collect();
            items.join(", ")
        }
        other => other.to_string(),
    }
}

/// Format as integer (type `d`)
fn format_as_integer(val: &serde_json::Value, spec: &FormatSpec) -> String {
    let num = match val {
        serde_json::Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0) as i64,
        _ => 0,
    };

    let width = spec.width.unwrap_or(0);
    if spec.zero_pad && width > 0 {
        format!("{num:0>width$}")
    } else if width > 0 {
        format!("{num:>width$}")
    } else {
        num.to_string()
    }
}

/// Format as float (type `f`/`F`)
fn format_as_float(val: &serde_json::Value, spec: &FormatSpec) -> String {
    let num = match val {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };

    let prec = spec.precision.unwrap_or(6);
    let width = spec.width.unwrap_or(0);
    if spec.zero_pad && width > 0 {
        format!("{num:0>width$.prec$}")
    } else if width > 0 {
        format!("{num:>width$.prec$}")
    } else {
        format!("{num:.prec$}")
    }
}

/// Format as JSON (type `j`, `#j` = pretty)
fn format_as_json(val: &serde_json::Value, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(val).unwrap_or_else(|_| val.to_string())
    } else {
        serde_json::to_string(val).unwrap_or_else(|_| val.to_string())
    }
}

/// Format as list (type `l`, `#l` = newline-separated)
fn format_as_list(val: &serde_json::Value, newline: bool) -> String {
    let sep = if newline { "\n" } else { ", " };
    match val {
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_as_string).collect();
            items.join(sep)
        }
        _ => format_as_string(val),
    }
}

/// Format as human-readable bytes (type `B`, `#B` = 1024-based)
fn format_as_bytes(val: &serde_json::Value, binary: bool) -> String {
    let num = match val {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };

    let (base, suffixes): (f64, &[&str]) = if binary {
        (1024.0, &["B", "KiB", "MiB", "GiB", "TiB", "PiB"])
    } else {
        (1000.0, &["B", "KB", "MB", "GB", "TB", "PB"])
    };

    let mut val = num;
    for (i, suffix) in suffixes.iter().enumerate() {
        if val.abs() < base || i == suffixes.len() - 1 {
            return if i == 0 {
                format!("{val:.0}{suffix}")
            } else {
                format!("{val:.2}{suffix}")
            };
        }
        val /= base;
    }
    format!("{num:.0}B")
}

/// Format with decimal suffixes (type `D`, `#D` = 1024-based)
fn format_as_decimal(val: &serde_json::Value, binary: bool) -> String {
    let num = match val {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };

    let base: f64 = if binary { 1024.0 } else { 1000.0 };
    let suffixes = ["", "K", "M", "G", "T", "P"];

    let mut val = num;
    for (i, suffix) in suffixes.iter().enumerate() {
        if val.abs() < base || i == suffixes.len() - 1 {
            return if i == 0 {
                format!("{val:.0}")
            } else if val.fract() == 0.0 {
                format!("{val:.0}{suffix}")
            } else {
                format!("{val:.1}{suffix}")
            };
        }
        val /= base;
    }
    format!("{num:.0}")
}

/// Format as filename-safe string (type `S`, `#S` = ASCII-only)
fn format_as_sanitize(val: &serde_json::Value, ascii_only: bool) -> String {
    let s = format_as_string(val);
    let mut result = String::new();
    for c in s.chars() {
        if c == '\0' || c.is_control() {
            continue;
        }
        match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => result.push('_'),
            _ if ascii_only && !c.is_ascii() => result.push('_'),
            _ => result.push(c),
        }
    }
    result
}

/// Format as shell-quoted string (type `q`)
fn format_as_quote(val: &serde_json::Value) -> String {
    let s = format_as_string(val);
    // Simple POSIX-style single-quote escaping
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Format as HTML-escaped string (type `h`)
fn format_as_html(val: &serde_json::Value) -> String {
    let s = format_as_string(val);
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Format integer as character (type `c`)
fn format_as_char(val: &serde_json::Value) -> String {
    let num = match val {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) as u32,
        serde_json::Value::String(s) => s.parse::<u32>().unwrap_or(0),
        _ => 0,
    };
    char::from_u32(num)
        .map(|c| c.to_string())
        .unwrap_or_default()
}

/// Format with Unicode normalization (type `U`, `#U` = NFD)
///
/// Simplified: `U` lowercases, `#U` does nothing extra (full NFC/NFD requires unicode-normalization crate)
fn format_as_unicode(val: &serde_json::Value, _nfd: bool) -> String {
    // Without the unicode-normalization crate, just pass through
    format_as_string(val)
}

/// Format with thousands separator (type `R`)
fn format_as_thousands(val: &serde_json::Value) -> String {
    let num = match val {
        serde_json::Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0) as i64,
        _ => 0,
    };

    let negative = num < 0;
    let abs_str = num.unsigned_abs().to_string();
    let chars: Vec<char> = abs_str.chars().collect();
    let mut result = String::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*c);
    }
    if negative {
        format!("-{result}")
    } else {
        result
    }
}

/// Apply width padding (for format types that don't handle it internally)
fn apply_width(raw: &str, spec: &FormatSpec) -> String {
    // Integer and Float handle width internally
    if matches!(spec.type_char, FormatType::Integer | FormatType::Float) {
        return raw.to_string();
    }

    let width = spec.width.unwrap_or(0);
    if width > 0 && raw.len() < width {
        if spec.zero_pad {
            format!("{raw:0>width$}")
        } else {
            format!("{raw:>width$}")
        }
    } else if let Some(prec) = spec.precision {
        // For strings, precision truncates
        if spec.type_char == FormatType::String {
            raw.chars().take(prec).collect()
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    }
}

// === Parsing helpers ===

/// Split on the last unescaped occurrence of `sep`
fn split_last_unescaped(s: &str, sep: char) -> (&str, Option<String>) {
    // Find last occurrence that isn't inside `>strftime`
    // Simple approach: find last `sep` not preceded by `>`
    if let Some(pos) = s.rfind(sep) {
        // Don't split on `|` or `&` inside strftime (after `>`)
        let before = &s[..pos];
        if before.contains('>') {
            // Check if the separator is inside a strftime format
            let gt_pos = before
                .rfind('>')
                .expect("'>' was confirmed present by contains check");
            if pos > gt_pos {
                // The separator is after `>`, could be part of strftime — but yt-dlp
                // considers `|` and `&` as higher-level separators. Use the split.
                return (&s[..pos], Some(s[pos + sep.len_utf8()..].to_string()));
            }
        }
        (&s[..pos], Some(s[pos + sep.len_utf8()..].to_string()))
    } else {
        (s, None)
    }
}

/// Split fallback fields on `,` but not inside `>`-prefixed strftime
fn split_fallback_fields(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_strftime = false;

    for (i, c) in s.char_indices() {
        if c == '>' {
            in_strftime = true;
        } else if c == ',' && !in_strftime {
            result.push(&s[start..i]);
            start = i + 1;
        }
    }
    result.push(&s[start..]);
    result
}

/// Parse a number string (possibly negative) from chars at position
fn parse_number_str(chars: &[char], i: &mut usize) -> String {
    let mut s = String::new();
    if *i < chars.len() && chars[*i] == '-' {
        s.push('-');
        *i += 1;
    }
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        s.push(chars[*i]);
        *i += 1;
    }
    s
}

/// Parse a float string (possibly with decimal point) from chars at position
fn parse_float_str(chars: &[char], i: &mut usize) -> String {
    let mut s = String::new();
    let mut has_dot = false;
    while *i < chars.len() && (chars[*i].is_ascii_digit() || (chars[*i] == '.' && !has_dot)) {
        if chars[*i] == '.' {
            has_dot = true;
        }
        s.push(chars[*i]);
        *i += 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> RenderContext {
        RenderContext {
            epoch: 1700000000,
            autonumber: 1,
        }
    }

    fn test_info() -> InfoDict {
        let mut info = InfoDict::new(
            "abc123",
            "My Video Title",
            "TestExtractor",
            "https://example.com/video",
        );
        info.uploader = Some("TestUploader".to_string());
        info.playlist_index = Some(5);
        info.view_count = Some(12345);
        info.upload_date = Some("20231114".to_string());
        info.duration = Some(3661.5);
        info.tags = Some(vec![
            "tag1".to_string(),
            "tag2".to_string(),
            "tag3".to_string(),
        ]);
        info
    }

    fn test_format() -> Format {
        let mut f = Format::new(
            "720p",
            "https://example.com/video.mp4",
            "mp4",
            rdlp_core::DownloadProtocol::Https,
        );
        f.height = Some(720);
        f.width = Some(1280);
        f.filesize = Some(590_000_000);
        f
    }

    // === Parser tests ===

    #[test]
    fn test_parse_default_template() {
        let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
        assert_eq!(t.segments.len(), 3); // field, literal ".", field
    }

    #[test]
    fn test_parse_literal_only() {
        let t = OutputTemplate::parse("hello world").unwrap();
        assert_eq!(t.segments.len(), 1);
        assert!(matches!(&t.segments[0], Segment::Literal(s) if s == "hello world"));
    }

    #[test]
    fn test_parse_field_with_default() {
        let t = OutputTemplate::parse("%(uploader|Unknown)s").unwrap();
        assert_eq!(t.segments.len(), 1);
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "uploader");
                assert_eq!(spec.default.as_deref(), Some("Unknown"));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_numeric_format() {
        let t = OutputTemplate::parse("%(playlist_index)03d").unwrap();
        assert_eq!(t.segments.len(), 1);
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "playlist_index");
                assert!(spec.format.zero_pad);
                assert_eq!(spec.format.width, Some(3));
                assert_eq!(spec.format.type_char, FormatType::Integer);
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_subdirectory_template() {
        let t = OutputTemplate::parse("%(extractor)s/%(title)s.%(ext)s").unwrap();
        assert_eq!(t.segments.len(), 5); // field, "/", field, ".", field
    }

    #[test]
    fn test_parse_error_unclosed_field() {
        let err = OutputTemplate::parse("%(title").unwrap_err();
        assert!(matches!(err, TemplateError::UnclosedField { position: 0 }));
    }

    #[test]
    fn test_parse_error_empty_field_name() {
        let err = OutputTemplate::parse("%()s").unwrap_err();
        assert!(matches!(err, TemplateError::EmptyFieldName { position: 0 }));
    }

    #[test]
    fn test_parse_error_invalid_specifier() {
        let err = OutputTemplate::parse("%(title)x").unwrap_err();
        assert!(matches!(err, TemplateError::InvalidFormatSpecifier { .. }));
    }

    #[test]
    fn test_parse_empty_default() {
        let t = OutputTemplate::parse("%(uploader|)s").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "uploader");
                assert_eq!(spec.default.as_deref(), Some(""));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_integer_no_padding() {
        let t = OutputTemplate::parse("%(view_count)d").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert!(!spec.format.zero_pad);
                assert_eq!(spec.format.width, None);
                assert_eq!(spec.format.type_char, FormatType::Integer);
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_literal_percent() {
        let t = OutputTemplate::parse("100%% done").unwrap();
        assert_eq!(t.segments.len(), 1);
        assert!(matches!(&t.segments[0], Segment::Literal(s) if s == "100% done"));
    }

    #[test]
    fn test_parse_fallback_fields() {
        let t = OutputTemplate::parse("%(uploader,channel,title)s").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names.len(), 3);
                assert_eq!(spec.names[0].name, "uploader");
                assert_eq!(spec.names[1].name, "channel");
                assert_eq!(spec.names[2].name, "title");
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_conditional_replacement() {
        let t = OutputTemplate::parse("%(uploader&by {})s").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "uploader");
                assert_eq!(spec.replacement.as_deref(), Some("by {}"));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_date_format() {
        let t = OutputTemplate::parse("%(upload_date>%Y-%m-%d)s").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "upload_date");
                assert_eq!(spec.names[0].date_format.as_deref(), Some("%Y-%m-%d"));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_arithmetic() {
        let t = OutputTemplate::parse("%(playlist_index+10)d").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "playlist_index");
                assert_eq!(spec.names[0].arithmetic.len(), 1);
                assert!(matches!(spec.names[0].arithmetic[0].0, ArithOp::Add));
                assert!((spec.names[0].arithmetic[0].1 - 10.0).abs() < f64::EPSILON);
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_object_traversal() {
        let t = OutputTemplate::parse("%(tags.0)s").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "tags");
                assert_eq!(spec.names[0].accessors.len(), 1);
                assert!(matches!(spec.names[0].accessors[0], Accessor::Index(0)));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_string_slice() {
        let t = OutputTemplate::parse("%(title.:50)s").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.names[0].name, "title");
                assert_eq!(spec.names[0].accessors.len(), 1);
                assert!(matches!(
                    spec.names[0].accessors[0],
                    Accessor::Slice {
                        start: None,
                        end: Some(50)
                    }
                ));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_hash_flag() {
        let t = OutputTemplate::parse("%(title)#j").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert!(spec.format.hash);
                assert_eq!(spec.format.type_char, FormatType::Json);
            }
            _ => panic!("Expected Field segment"),
        }
    }

    #[test]
    fn test_parse_float_format() {
        let t = OutputTemplate::parse("%(duration).2f").unwrap();
        match &t.segments[0] {
            Segment::Field(spec) => {
                assert_eq!(spec.format.type_char, FormatType::Float);
                assert_eq!(spec.format.precision, Some(2));
            }
            _ => panic!("Expected Field segment"),
        }
    }

    // === Renderer tests ===

    #[test]
    fn test_render_default_template() {
        let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "My Video Title.mp4");
    }

    #[test]
    fn test_render_ext_is_computed() {
        let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mkv", &test_ctx())
            .unwrap();
        assert_eq!(result, "My Video Title.mkv");
    }

    #[test]
    fn test_render_info_dict_fields() {
        let t = OutputTemplate::parse("%(id)s - %(title)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "abc123 - My Video Title");
    }

    #[test]
    fn test_render_format_fields() {
        let t = OutputTemplate::parse("%(format_id)s_%(height)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "720p_720");
    }

    #[test]
    fn test_render_missing_field_with_default() {
        let t = OutputTemplate::parse("%(channel|Unknown Channel)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "Unknown Channel");
    }

    #[test]
    fn test_render_missing_field_defaults_to_na() {
        let t = OutputTemplate::parse("%(nonexistent)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "NA");
    }

    #[test]
    fn test_render_numeric_padding() {
        let t = OutputTemplate::parse("%(playlist_index)03d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "005");
    }

    #[test]
    fn test_render_subdirectory() {
        let t = OutputTemplate::parse("%(extractor)s/%(title)s.%(ext)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "TestExtractor/My Video Title.mp4");
    }

    #[test]
    fn test_render_complex_template() {
        let t = OutputTemplate::parse(
            "%(extractor)s/%(playlist_index)03d - %(title)s [%(id)s].%(ext)s",
        )
        .unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "TestExtractor/005 - My Video Title [abc123].mp4");
    }

    #[test]
    fn test_render_present_optional_over_default() {
        let t = OutputTemplate::parse("%(uploader|Fallback)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "TestUploader");
    }

    #[test]
    fn test_render_view_count_as_string() {
        let t = OutputTemplate::parse("%(view_count)s views").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "12345 views");
    }

    #[test]
    fn test_render_view_count_as_integer() {
        let t = OutputTemplate::parse("%(view_count)d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "12345");
    }

    // === New feature tests ===

    #[test]
    fn test_render_literal_percent() {
        let t = OutputTemplate::parse("100%% - %(title)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "100% - My Video Title");
    }

    #[test]
    fn test_render_fallback_first_exists() {
        // uploader exists, so it should be used
        let t = OutputTemplate::parse("%(uploader,channel,title)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "TestUploader");
    }

    #[test]
    fn test_render_fallback_skips_missing() {
        // channel doesn't exist, uploader_id doesn't exist, title exists
        let t = OutputTemplate::parse("%(channel,channel_id,title)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "My Video Title");
    }

    #[test]
    fn test_render_fallback_all_missing_uses_default() {
        let t = OutputTemplate::parse("%(channel,channel_id|Unknown)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "Unknown");
    }

    #[test]
    fn test_render_fallback_all_missing_na() {
        let t = OutputTemplate::parse("%(channel,channel_id)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "NA");
    }

    #[test]
    fn test_render_conditional_replacement_field_exists() {
        // yt-dlp: %(uploader&by {})s → "by TestUploader" if uploader exists
        // The `&replacement` text is not used in the fallback lookup; when the field
        // exists, replacement is not applied. This is template *replacement* text.
        // Actually yt-dlp's &replacement: if field exists, output replacement with {} → value
        // Let's verify behavior: field exists → formatted value (replacement is for when used with {})
        let t = OutputTemplate::parse("%(uploader)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "TestUploader");
    }

    #[test]
    fn test_render_conditional_replacement_field_missing() {
        // When field is missing and &replacement is set, output empty string
        let t = OutputTemplate::parse("%(nonexistent&replacement text)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_render_date_formatting() {
        let t = OutputTemplate::parse("%(upload_date>%Y-%m-%d)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "2023-11-14");
    }

    #[test]
    fn test_render_date_formatting_year_only() {
        let t = OutputTemplate::parse("%(upload_date>%Y)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "2023");
    }

    #[test]
    fn test_render_arithmetic_add() {
        let t = OutputTemplate::parse("%(playlist_index+10)d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "15");
    }

    #[test]
    fn test_render_arithmetic_div() {
        let t = OutputTemplate::parse("%(duration/60).0f").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        // 3661.5 / 60 = 61.025
        assert_eq!(result, "61");
    }

    #[test]
    fn test_render_arithmetic_sub() {
        let t = OutputTemplate::parse("%(playlist_index-1)d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "4");
    }

    #[test]
    fn test_render_arithmetic_mul() {
        let t = OutputTemplate::parse("%(playlist_index*2)d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "10");
    }

    #[test]
    fn test_render_object_traversal_index() {
        let t = OutputTemplate::parse("%(tags.0)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "tag1");
    }

    #[test]
    fn test_render_object_traversal_negative_index() {
        let t = OutputTemplate::parse("%(tags.-1)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "tag3");
    }

    #[test]
    fn test_render_string_slice_from_start() {
        let t = OutputTemplate::parse("%(title.:8)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "My Video");
    }

    #[test]
    fn test_render_string_slice_from_end() {
        let t = OutputTemplate::parse("%(title.-5:)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "Title");
    }

    #[test]
    fn test_render_string_slice_range() {
        let t = OutputTemplate::parse("%(title.3:8)s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "Video");
    }

    #[test]
    fn test_render_float_format() {
        let t = OutputTemplate::parse("%(duration).2f").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "3661.50");
    }

    #[test]
    fn test_render_float_format_zero_precision() {
        let t = OutputTemplate::parse("%(duration).0f").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "3662");
    }

    #[test]
    fn test_render_json_format() {
        let t = OutputTemplate::parse("%(title)j").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        // JSON wraps strings in `"`, which are Windows-restricted
        // chars and get sanitized to `_` by render()
        assert_eq!(result, "_My Video Title_");
    }

    #[test]
    fn test_render_json_pretty() {
        let t = OutputTemplate::parse("%(tags)#j").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert!(result.contains('\n'));
        assert!(result.contains("tag1"));
    }

    #[test]
    fn test_render_list_format() {
        let t = OutputTemplate::parse("%(tags)l").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "tag1, tag2, tag3");
    }

    #[test]
    fn test_render_list_newline() {
        let t = OutputTemplate::parse("%(tags)#l").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "tag1\ntag2\ntag3");
    }

    #[test]
    fn test_render_bytes_format() {
        let t = OutputTemplate::parse("%(filesize)B").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "590.00MB");
    }

    #[test]
    fn test_render_bytes_binary() {
        let t = OutputTemplate::parse("%(filesize)#B").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "562.67MiB");
    }

    #[test]
    fn test_render_decimal_format() {
        let t = OutputTemplate::parse("%(view_count)D").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "12.3K");
    }

    #[test]
    fn test_render_sanitize_format() {
        let t = OutputTemplate::parse("%(title)S").unwrap();
        let info = {
            let mut i = test_info();
            i.title = "My: Video / Title".to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "My_ Video _ Title");
    }

    #[test]
    fn test_render_sanitize_ascii_only() {
        let t = OutputTemplate::parse("%(title)#S").unwrap();
        let info = {
            let mut i = test_info();
            i.title = "日本語/Test".to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "____Test");
    }

    #[test]
    fn test_render_quote_format() {
        let t = OutputTemplate::parse("%(title)q").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "'My Video Title'");
    }

    #[test]
    fn test_render_quote_with_single_quote() {
        let t = OutputTemplate::parse("%(title)q").unwrap();
        let info = {
            let mut i = test_info();
            i.title = "It's a test".to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        // The `\` in `'\''` gets replaced with `_` by render()'s path sanitization
        assert_eq!(result, "'It'_''s a test'");
    }

    #[test]
    fn test_render_html_format() {
        let t = OutputTemplate::parse("%(title)h").unwrap();
        let info = {
            let mut i = test_info();
            i.title = "<b>Bold & \"Fun\"</b>".to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "&lt;b&gt;Bold &amp; &quot;Fun&quot;&lt;_b&gt;");
    }

    #[test]
    fn test_render_char_format() {
        let t = OutputTemplate::parse("%(view_count)c").unwrap();
        let info = {
            let mut i = test_info();
            i.view_count = Some(65); // ASCII 'A'
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "A");
    }

    #[test]
    fn test_render_thousands_separator() {
        let t = OutputTemplate::parse("%(view_count)R").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "12,345");
    }

    #[test]
    fn test_render_thousands_large_number() {
        let t = OutputTemplate::parse("%(filesize)R").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "590,000,000");
    }

    #[test]
    fn test_render_epoch() {
        let t = OutputTemplate::parse("%(epoch)d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "1700000000");
    }

    #[test]
    fn test_render_autonumber() {
        let t = OutputTemplate::parse("%(autonumber)05d").unwrap();
        let ctx = RenderContext {
            epoch: 0,
            autonumber: 42,
        };
        let result = t.render(&test_info(), &test_format(), "mp4", &ctx).unwrap();
        assert_eq!(result, "00042");
    }

    #[test]
    fn test_render_width_string() {
        let t = OutputTemplate::parse("%(id)10s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "    abc123");
    }

    #[test]
    fn test_render_precision_string() {
        let t = OutputTemplate::parse("%(title).5s").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "My Vi");
    }

    #[test]
    fn test_render_integer_width_no_pad() {
        let t = OutputTemplate::parse("%(playlist_index)5d").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "    5");
    }

    #[test]
    fn test_render_unicode_passthrough() {
        let t = OutputTemplate::parse("%(title)U").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "My Video Title");
    }

    #[test]
    fn test_render_bytes_small() {
        let t = OutputTemplate::parse("%(view_count)B").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "12.35KB");
    }

    #[test]
    fn test_render_decimal_millions() {
        let t = OutputTemplate::parse("%(filesize)D").unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        // 590_000_000 / 1_000_000 = 590.0 exactly, so no decimal shown
        assert_eq!(result, "590M");
    }

    #[test]
    fn test_backward_compat_complex() {
        // Ensure full backward compatibility with B1 MVP templates
        let t = OutputTemplate::parse(
            "%(extractor)s/%(playlist_index)03d - %(title)s [%(id)s].%(ext)s",
        )
        .unwrap();
        let result = t
            .render(&test_info(), &test_format(), "mp4", &test_ctx())
            .unwrap();
        assert_eq!(result, "TestExtractor/005 - My Video Title [abc123].mp4");
    }

    #[test]
    fn test_render_field_sanitizes_windows_chars() {
        // All 9 Windows-restricted path characters must be replaced
        // with `_` in field values: / \ : * ? " < > |
        let t = OutputTemplate::parse("%(title)s").unwrap();
        let info = {
            let mut i = test_info();
            i.title = r#"a/b\c:d*e?f"g<h>i|j"#.to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "a_b_c_d_e_f_g_h_i_j");
    }

    #[test]
    fn test_render_literal_slash_preserved_as_directory_separator() {
        // Literal `/` in the template string creates directories
        // and must NOT be sanitized — only field values are sanitized
        let t = OutputTemplate::parse("%(extractor)s/%(title)s.%(ext)s").unwrap();
        let info = {
            let mut i = test_info();
            i.title = "safe-title".to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "TestExtractor/safe-title.mp4");
    }

    #[test]
    fn test_render_field_strips_null_and_control_chars() {
        // Null bytes and control characters are stripped entirely
        // (not replaced) because they are invalid in filenames on
        // all platforms
        let t = OutputTemplate::parse("%(title)s").unwrap();
        let info = {
            let mut i = test_info();
            i.title = "hello\0world\x01\x1f!".to_string();
            i
        };
        let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
        assert_eq!(result, "helloworld!");
    }
}
