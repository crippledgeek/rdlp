//! Human-readable rate string parsing.

/// Parse a human-readable rate string into bytes per second.
///
/// Supports suffixes: `K` (kilobytes), `M` (megabytes), `G` (gigabytes).
/// Case-insensitive. Decimal values supported (e.g., `2.5M`).
/// Plain numbers are treated as bytes per second.
///
/// Uses binary units: 1K = 1024, 1M = 1048576, 1G = 1073741824.
///
/// # Examples
///
/// ```
/// use rdlp_ratelimit::parse_rate_limit;
///
/// assert_eq!(parse_rate_limit("1M").unwrap(), 1_048_576);
/// assert_eq!(parse_rate_limit("500K").unwrap(), 512_000);
/// assert_eq!(parse_rate_limit("2.5M").unwrap(), 2_621_440);
/// assert_eq!(parse_rate_limit("1048576").unwrap(), 1_048_576);
/// ```
pub fn parse_rate_limit(rate_str: &str) -> Result<u64, String> {
    let rate_str = rate_str.trim();
    if rate_str.is_empty() {
        return Err("Rate limit string is empty".to_string());
    }

    let (number_part, multiplier) = if let Some(num) = rate_str
        .strip_suffix('G')
        .or_else(|| rate_str.strip_suffix('g'))
    {
        (num, 1024u64 * 1024 * 1024)
    } else if let Some(num) = rate_str
        .strip_suffix('M')
        .or_else(|| rate_str.strip_suffix('m'))
    {
        (num, 1024u64 * 1024)
    } else if let Some(num) = rate_str
        .strip_suffix('K')
        .or_else(|| rate_str.strip_suffix('k'))
    {
        (num, 1024u64)
    } else {
        (rate_str, 1u64)
    };

    let value: f64 = number_part
        .parse()
        .map_err(|e| format!("Invalid rate limit '{rate_str}': {e}"))?;

    if value <= 0.0 {
        return Err(format!("Rate limit must be positive, got '{rate_str}'"));
    }

    Ok((value * multiplier as f64) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_megabytes() {
        assert_eq!(parse_rate_limit("1M").unwrap(), 1_048_576);
        assert_eq!(parse_rate_limit("2.5M").unwrap(), 2_621_440);
        assert_eq!(parse_rate_limit("10m").unwrap(), 10_485_760);
    }

    #[test]
    fn test_parse_kilobytes() {
        assert_eq!(parse_rate_limit("500K").unwrap(), 512_000);
        assert_eq!(parse_rate_limit("1k").unwrap(), 1024);
    }

    #[test]
    fn test_parse_gigabytes() {
        assert_eq!(parse_rate_limit("1G").unwrap(), 1_073_741_824);
    }

    #[test]
    fn test_parse_plain_number() {
        assert_eq!(parse_rate_limit("1048576").unwrap(), 1_048_576);
    }

    #[test]
    fn test_parse_errors() {
        assert!(parse_rate_limit("").is_err());
        assert!(parse_rate_limit("abc").is_err());
        assert!(parse_rate_limit("-1M").is_err());
        assert!(parse_rate_limit("0M").is_err());
    }
}
