//! Size literal parser for format filter expressions.
//!
//! Parses size strings like `500M`, `1.5GiB`, `2G` into byte counts.
//! Matches yt-dlp behaviour: bare units (K/M/G) use SI (1000-based),
//! `iB` suffix (KiB/MiB/GiB) uses binary (1024-based).

/// Parse a size literal into a byte count.
///
/// - `500M` → 500 * 1,000,000 = 500,000,000 (SI)
/// - `500MB` → 500 * 1,000,000 (SI, same as bare M)
/// - `500MiB` → 500 * 1,048,576 = 524,288,000 (binary)
/// - `1.5GiB` → 1.5 * 1,073,741,824 = 1,610,612,736 (binary)
///
/// Returns `None` if the string is not a valid size literal.
pub(crate) fn parse_size(input: &str) -> Option<u64> {
    if input.is_empty() {
        return None;
    }

    // Split numeric prefix from the unit suffix.
    let split_pos = input
        .find(|c: char| c.is_ascii_alphabetic())
        .filter(|&pos| pos > 0)?;

    let number_part = &input[..split_pos];
    let unit_part = &input[split_pos..];

    let number: f64 = number_part.parse().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }

    let unit_char = unit_part.chars().next()?.to_ascii_uppercase();

    // Check suffix to determine SI vs binary
    let remainder = &unit_part[unit_char.len_utf8()..];
    let binary = match remainder {
        "" | "B" => false,  // bare K/M/G or KB/MB/GB → SI (1000-based)
        "i" | "iB" => true, // Ki/Mi/Gi or KiB/MiB/GiB → binary (1024-based)
        _ => return None,
    };

    let base: u64 = if binary { 1024 } else { 1000 };

    let multiplier: u64 = match unit_char {
        'K' => base,
        'M' => base.pow(2),
        'G' => base.pow(3),
        'T' => base.pow(4),
        'P' => base.pow(5),
        _ => return None,
    };

    let bytes = number * multiplier as f64;
    if bytes > u64::MAX as f64 {
        return None;
    }

    Some(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    // SI units (bare M/G/K → 1000-based)
    #[test]
    fn si_megabytes() {
        assert_eq!(parse_size("500M"), Some(500_000_000));
    }

    #[test]
    fn si_megabytes_with_b() {
        assert_eq!(parse_size("500MB"), Some(500_000_000));
    }

    #[test]
    fn si_gigabytes() {
        assert_eq!(parse_size("2G"), Some(2_000_000_000));
    }

    #[test]
    fn si_kilobytes() {
        assert_eq!(parse_size("100K"), Some(100_000));
    }

    #[test]
    fn si_kilobytes_lowercase() {
        assert_eq!(parse_size("100k"), Some(100_000));
    }

    #[test]
    fn si_terabytes() {
        assert_eq!(parse_size("1TB"), Some(1_000_000_000_000));
    }

    // Binary units (MiB/GiB/KiB → 1024-based)
    #[test]
    fn binary_mebibytes() {
        assert_eq!(parse_size("500MiB"), Some(524_288_000));
    }

    #[test]
    fn binary_gibibytes_float() {
        assert_eq!(parse_size("1.5GiB"), Some(1_610_612_736));
    }

    #[test]
    fn binary_kibibytes() {
        assert_eq!(parse_size("100KiB"), Some(102_400));
    }

    #[test]
    fn binary_tebibytes() {
        assert_eq!(parse_size("1TiB"), Some(1_099_511_627_776));
    }

    // yt-dlp compat: 1M = 1,000,000 and 1MiB = 1,048,576
    #[test]
    fn ytdlp_compat_1m_vs_1mib() {
        let si = parse_size("1M").unwrap();
        let binary = parse_size("1MiB").unwrap();
        assert_eq!(si, 1_000_000);
        assert_eq!(binary, 1_048_576);
        assert!(binary > si);
    }

    // Invalid inputs
    #[test]
    fn plain_number_returns_none() {
        assert_eq!(parse_size("12345"), None);
    }

    #[test]
    fn non_numeric_returns_none() {
        assert_eq!(parse_size("abc"), None);
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(parse_size(""), None);
    }
}
