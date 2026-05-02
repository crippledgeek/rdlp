//! Parse DASH `Representation@audioSamplingRate` strings.

/// Parse the first integer value from a DASH audio sampling rate string.
///
/// DASH spec §5.3.7.2 allows space-separated multi-value lists; we take
/// the first value (the canonical mode). Returns `None` if no value parses.
pub fn parse_audio_sampling_rate(raw: &str) -> Option<u32> {
    raw.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_value() {
        assert_eq!(parse_audio_sampling_rate("48000"), Some(48000));
    }

    #[test]
    fn multi_value_takes_first() {
        assert_eq!(parse_audio_sampling_rate("48000 44100"), Some(48000));
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(parse_audio_sampling_rate(" 22050 "), Some(22050));
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(parse_audio_sampling_rate(""), None);
        assert_eq!(parse_audio_sampling_rate("   "), None);
    }

    #[test]
    fn malformed_returns_none() {
        assert_eq!(parse_audio_sampling_rate("abc"), None);
    }
}
