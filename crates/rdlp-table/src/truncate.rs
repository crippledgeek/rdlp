//! Unicode-aware string truncation for table cells.
//!
//! Uses `console::measure_text_width` to correctly handle wide characters
//! (CJK, emoji) and ANSI escape sequences when truncating cell content.

use console::measure_text_width;

/// Truncate `text` to fit within `max_width` display columns.
///
/// If the text fits, returns it unchanged. If it exceeds `max_width`,
/// truncates and appends `…` (single character, 1 display column).
///
/// # Arguments
/// * `text` - The string to potentially truncate
/// * `max_width` - Maximum display width in columns
///
/// # Returns
/// A `String` that is at most `max_width` display columns wide.
///
/// # Examples
/// ```
/// use rdlp_table::truncate::truncate_to_width;
/// assert_eq!(truncate_to_width("hello", 10), "hello");
/// assert_eq!(truncate_to_width("hello world", 5), "hell…");
/// ```
#[must_use]
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let text_width = measure_text_width(text);
    if text_width <= max_width {
        return text.to_string();
    }

    // Need to truncate. Reserve 1 column for '…'.
    let target = max_width.saturating_sub(1);
    let mut current_width = 0;
    let mut byte_end = 0;

    for ch in text.chars() {
        let ch_width = unicode_width(ch);
        if current_width + ch_width > target {
            break;
        }
        current_width += ch_width;
        byte_end += ch.len_utf8();
    }

    let mut result = String::with_capacity(byte_end + 3); // '…' is 3 bytes UTF-8
    result.push_str(&text[..byte_end]);
    result.push('…');
    result
}

/// Pad `text` to exactly `width` display columns.
///
/// If `text` is shorter, pads with spaces on the right (left-align) or
/// left (right-align). If `text` is longer, truncates with ellipsis.
///
/// # Arguments
/// * `text` - The string to pad/truncate
/// * `width` - Target display width
/// * `right_align` - If true, pad on the left (right-align content)
#[must_use]
pub fn pad_to_width(text: &str, width: usize, right_align: bool) -> String {
    let truncated = truncate_to_width(text, width);
    let text_width = measure_text_width(&truncated);

    if text_width >= width {
        return truncated;
    }

    let padding = width - text_width;
    let mut result = String::with_capacity(truncated.len() + padding);

    if right_align {
        for _ in 0..padding {
            result.push(' ');
        }
        result.push_str(&truncated);
    } else {
        result.push_str(&truncated);
        for _ in 0..padding {
            result.push(' ');
        }
    }

    result
}

/// Get the display width of a single character.
///
/// CJK characters are 2 columns wide; most others are 1.
/// Control characters are 0.
fn unicode_width(ch: char) -> usize {
    if ch.is_control() {
        0
    } else {
        // Measure a single character string. This is correct for all Unicode
        // including CJK, emoji, combining marks.
        measure_text_width(&ch.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short_text_unchanged() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact_fit() {
        assert_eq!(truncate_to_width("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_adds_ellipsis() {
        assert_eq!(truncate_to_width("hello world", 5), "hell…");
    }

    #[test]
    fn test_truncate_single_char_width() {
        assert_eq!(truncate_to_width("hello", 1), "…");
    }

    #[test]
    fn test_truncate_zero_width() {
        assert_eq!(truncate_to_width("hello", 0), "");
    }

    #[test]
    fn test_truncate_empty_string() {
        assert_eq!(truncate_to_width("", 10), "");
    }

    #[test]
    fn test_pad_left_align() {
        let result = pad_to_width("hi", 6, false);
        assert_eq!(result, "hi    ");
        assert_eq!(measure_text_width(&result), 6);
    }

    #[test]
    fn test_pad_right_align() {
        let result = pad_to_width("42", 6, true);
        assert_eq!(result, "    42");
        assert_eq!(measure_text_width(&result), 6);
    }

    #[test]
    fn test_pad_truncates_when_too_long() {
        let result = pad_to_width("very long text", 6, false);
        assert_eq!(measure_text_width(&result), 6);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_pad_exact_width() {
        let result = pad_to_width("exact", 5, false);
        assert_eq!(result, "exact");
    }

    #[test]
    fn test_truncate_cjk_characters() {
        // CJK characters are 2 columns wide each.
        // "日本語" = 6 columns. Truncating to 5 means 2 CJK chars (4 cols) + '…' (1 col) = 5.
        let result = truncate_to_width("日本語", 5);
        assert_eq!(result, "日本…");
        assert_eq!(measure_text_width(&result), 5);
    }

    #[test]
    fn test_truncate_cjk_exact_fit() {
        // "日本語" = 6 columns, max_width = 6 → no truncation.
        assert_eq!(truncate_to_width("日本語", 6), "日本語");
    }

    #[test]
    fn test_truncate_cjk_tight() {
        // max_width = 3: target = 2 columns for content + 1 for '…'.
        // One CJK char = 2 cols, fits.
        let result = truncate_to_width("日本語", 3);
        assert_eq!(result, "日…");
        assert_eq!(measure_text_width(&result), 3);
    }

    #[test]
    fn test_truncate_mixed_ascii_cjk() {
        // "hi日本" = 2 + 2 + 2 = 6 columns.
        let result = truncate_to_width("hi日本", 5);
        assert_eq!(result, "hi日…");
        assert_eq!(measure_text_width(&result), 5);
    }

    #[test]
    fn test_pad_cjk_right_align() {
        // "日本" = 4 columns. Padding to 8 → 4 spaces + "日本".
        let result = pad_to_width("日本", 8, true);
        assert_eq!(result, "    日本");
        assert_eq!(measure_text_width(&result), 8);
    }
}
