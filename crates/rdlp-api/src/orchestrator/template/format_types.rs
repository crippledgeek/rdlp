//! Format type implementations for template rendering

use super::{FormatSpec, FormatType};

/// Format as string (type `s`)
pub(super) fn format_as_string(val: &serde_json::Value) -> String {
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
pub(super) fn format_as_integer(val: &serde_json::Value, spec: &FormatSpec) -> String {
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
pub(super) fn format_as_float(val: &serde_json::Value, spec: &FormatSpec) -> String {
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
pub(super) fn format_as_json(val: &serde_json::Value, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(val).unwrap_or_else(|_| val.to_string())
    } else {
        serde_json::to_string(val).unwrap_or_else(|_| val.to_string())
    }
}

/// Format as list (type `l`, `#l` = newline-separated)
pub(super) fn format_as_list(val: &serde_json::Value, newline: bool) -> String {
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
pub(super) fn format_as_bytes(val: &serde_json::Value, binary: bool) -> String {
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
pub(super) fn format_as_decimal(val: &serde_json::Value, binary: bool) -> String {
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
pub(super) fn format_as_sanitize(val: &serde_json::Value, ascii_only: bool) -> String {
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
pub(super) fn format_as_quote(val: &serde_json::Value) -> String {
    let s = format_as_string(val);
    // Simple POSIX-style single-quote escaping
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Format as HTML-escaped string (type `h`)
pub(super) fn format_as_html(val: &serde_json::Value) -> String {
    let s = format_as_string(val);
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Format integer as character (type `c`)
pub(super) fn format_as_char(val: &serde_json::Value) -> String {
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
pub(super) fn format_as_unicode(val: &serde_json::Value, _nfd: bool) -> String {
    // Without the unicode-normalization crate, just pass through
    format_as_string(val)
}

/// Format with thousands separator (type `R`)
pub(super) fn format_as_thousands(val: &serde_json::Value) -> String {
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
pub(super) fn apply_width(raw: &str, spec: &FormatSpec) -> String {
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
