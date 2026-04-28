//! Human-readable rate string parsing.

use thiserror::Error;

/// Errors returned by [`parse_rate_limit`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RateLimitParseError {
    /// Rate limit string is empty (after trimming).
    #[error("Rate limit string is empty")]
    Empty,
    /// Rate limit number could not be parsed.
    #[error("Invalid rate limit '{input}': {reason}")]
    InvalidNumber {
        /// Original user input.
        input: String,
        /// Underlying parse-error message.
        reason: String,
    },
    /// Rate limit was zero or negative.
    #[error("Rate limit must be positive, got '{0}'")]
    NotPositive(String),
}

/// Parse a human-readable rate string into bytes per second.
///
/// Supports suffixes: `K` (kilobytes), `M` (megabytes), `G` (gigabytes).
/// Case-insensitive. Decimal values supported (e.g., `2.5M`). Plain
/// numbers are treated as bytes per second. Uses binary units:
/// 1K = 1024, 1M = 1048576, 1G = 1073741824.
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
pub fn parse_rate_limit(rate_str: &str) -> Result<u64, RateLimitParseError> {
    let rate_str = rate_str.trim();
    if rate_str.is_empty() {
        return Err(RateLimitParseError::Empty);
    }

    let (number_part, multiplier) = if let Some(rest) = rate_str.strip_suffix(['G', 'g']) {
        (rest, 1024u64 * 1024 * 1024)
    } else if let Some(rest) = rate_str.strip_suffix(['M', 'm']) {
        (rest, 1024u64 * 1024)
    } else if let Some(rest) = rate_str.strip_suffix(['K', 'k']) {
        (rest, 1024u64)
    } else {
        (rate_str, 1u64)
    };

    let value: f64 = number_part
        .parse()
        .map_err(
            |e: std::num::ParseFloatError| RateLimitParseError::InvalidNumber {
                input: rate_str.to_string(),
                reason: e.to_string(),
            },
        )?;

    if value <= 0.0 {
        return Err(RateLimitParseError::NotPositive(rate_str.to_string()));
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
