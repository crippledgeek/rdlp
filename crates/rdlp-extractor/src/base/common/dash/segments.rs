//! SegmentTemplate / SegmentTimeline / SegmentList → `Vec<Fragment>`.

/// Substitute DASH SegmentTemplate placeholders.
///
/// Supported tokens (per ISO/IEC 23009-1 §5.3.9.4.4):
/// - `$$` literal `$`
/// - `$RepresentationID$` → `repr_id`
/// - `$Number$` and `$Number%0Nd$` → segment number with optional width
/// - `$Time$` and `$Time%0Nd$` → segment time
/// - `$Bandwidth$` → bandwidth in bits/sec
pub fn substitute_template(
    template: &str,
    repr_id: &str,
    bandwidth: u64,
    number: Option<u64>,
    time: Option<u64>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // Found a '$' — find the next '$' to close the token.
        let Some(end_rel) = template[i + 1..].find('$') else {
            out.push('$');
            i += 1;
            continue;
        };
        let end = i + 1 + end_rel;
        let token = &template[i + 1..end];
        let replacement = render_token(token, repr_id, bandwidth, number, time);
        out.push_str(&replacement);
        i = end + 1;
    }
    out
}

fn render_token(
    token: &str,
    repr_id: &str,
    bandwidth: u64,
    number: Option<u64>,
    time: Option<u64>,
) -> String {
    if token.is_empty() {
        return "$".to_string();
    }
    if token == "RepresentationID" {
        return repr_id.to_string();
    }
    if token == "Bandwidth" {
        return bandwidth.to_string();
    }
    let (name, width) = parse_token(token);
    let value = match name {
        "Number" => number.unwrap_or(0),
        "Time" => time.unwrap_or(0),
        other => return format!("${other}$"),
    };
    match width {
        Some(w) => format!("{value:0width$}", width = w),
        None => value.to_string(),
    }
}

fn parse_token(token: &str) -> (&str, Option<usize>) {
    if let Some((name, fmt)) = token.split_once('%') {
        // fmt is e.g. "05d" or "0d" or "d".
        // Strip the trailing 'd' then parse the numeric width directly.
        // The leading '0' is a zero-pad flag, not extra width digits.
        let digits = fmt.trim_end_matches('d');
        let width = digits.parse::<usize>().ok().filter(|w| *w > 0);
        (name, width)
    } else {
        (token, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_number_with_width() {
        let out = substitute_template(
            "video/$RepresentationID$/seg-$Number%05d$.m4s",
            "v720p",
            2_500_000,
            Some(42),
            None,
        );
        assert_eq!(out, "video/v720p/seg-00042.m4s");
    }

    #[test]
    fn substitutes_bandwidth_and_time() {
        let out = substitute_template(
            "$RepresentationID$/$Bandwidth$/$Time$.m4s",
            "audio_aac",
            128_000,
            None,
            Some(1_234_567),
        );
        assert_eq!(out, "audio_aac/128000/1234567.m4s");
    }

    #[test]
    fn literal_dollar_escapes() {
        let out = substitute_template("a$$b", "x", 1, None, None);
        assert_eq!(out, "a$b");
    }

    #[test]
    fn substitutes_number_without_width() {
        let out = substitute_template("seg-$Number$.m4s", "v", 1, Some(7), None);
        assert_eq!(out, "seg-7.m4s");
    }
}
