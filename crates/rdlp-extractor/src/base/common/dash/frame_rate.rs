//! Parse DASH `Representation@frameRate` strings.

/// Parse a DASH frame-rate string (`"30000/1001"`, `"25"`, etc.) to fps.
///
/// Returns `None` for empty / malformed input or zero denominator.
pub fn parse_frame_rate(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((num, den)) = trimmed.split_once('/') {
        let n: f64 = num.trim().parse().ok()?;
        let d: f64 = den.trim().parse().ok()?;
        if d == 0.0 {
            return None;
        }
        return Some(n / d);
    }
    trimmed.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_ntsc() {
        let fps = parse_frame_rate("30000/1001").unwrap();
        assert!((fps - 29.97).abs() < 0.01);
    }

    #[test]
    fn integer_pal() {
        assert_eq!(parse_frame_rate("25"), Some(25.0));
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(parse_frame_rate(" 24 "), Some(24.0));
    }

    #[test]
    fn zero_denominator_returns_none() {
        assert_eq!(parse_frame_rate("60/0"), None);
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(parse_frame_rate(""), None);
    }

    #[test]
    fn malformed_returns_none() {
        assert_eq!(parse_frame_rate("abc"), None);
        assert_eq!(parse_frame_rate("30/abc"), None);
    }
}
