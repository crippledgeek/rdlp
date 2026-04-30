//! Template rendering: field resolution, accessor application, and value formatting

use super::format_types::*;
use super::{
    Accessor, ArithOp, FieldRef, FieldSpec, FormatSpec, FormatType, OutputTemplate, RenderContext,
    Segment, TemplateError,
};
use chrono::NaiveDate;
use rdlp_types::{Format, InfoDict};

impl OutputTemplate {
    /// Render the template using metadata from `InfoDict`, `Format`, and a computed extension.
    ///
    /// Field resolution priority:
    /// 1. Special computed fields (`ext`, `epoch`, `autonumber`)
    /// 2. InfoDict fields (via serde_json serialization)
    /// 3. Format fields (via serde_json serialization)
    /// 4. Conditional replacement / default / `"NA"`
    pub(in crate::orchestrator) fn render(
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
    /// Returns `None` if the field doesn't exist, is null, or is an
    /// effectively-empty string. Empty/whitespace-only strings are
    /// treated as missing so the `|default` pipe fallback fires for
    /// them — matching yt-dlp's behaviour. Without this, godresource-
    /// style extractors that produce `title: ""` (because the upstream
    /// API returned null) render as a literal empty segment in the
    /// output filename and the user gets `.mp4` instead of `NA.mp4`
    /// or `Unknown.mp4`.
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
                val.filter(|v| !is_missing(v))
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

        // Apply width (for format types that don't handle it internally)
        apply_width(&raw, spec)
    }
}

// === Helper functions ===

/// Returns `true` for JSON values that should be treated as "missing"
/// for `|default` pipe-fallback purposes:
///
/// - `null`
/// - `""` (empty string)
/// - any whitespace-only string (`"   "`, `"\t\n"`, etc.)
///
/// Without the empty/whitespace check, an extractor that produces a
/// blank title (the godresource case: API returns `title: null` and
/// the plugin sets `'title': ''`) renders into the filename as a
/// literal empty segment — `.mp4` instead of `NA.mp4` or
/// `Unknown.mp4`. yt-dlp's `_NO_DEFAULT` sentinel handling treats
/// empty strings the same as missing fields; this matches that.
fn is_missing(val: &serde_json::Value) -> bool {
    match val {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

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
