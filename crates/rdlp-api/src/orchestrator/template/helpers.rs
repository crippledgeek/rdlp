//! Parsing helper functions for template field content

/// Split on the last unescaped occurrence of `sep`
pub(super) fn split_last_unescaped(s: &str, sep: char) -> (&str, Option<String>) {
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
pub(super) fn split_fallback_fields(s: &str) -> Vec<&str> {
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
pub(super) fn parse_number_str(chars: &[char], i: &mut usize) -> String {
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
pub(super) fn parse_float_str(chars: &[char], i: &mut usize) -> String {
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
