//! Policy for automatic fixup of downloaded files.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::parse_error::ParseEnumError;

/// Controls how rdlp handles fixable issues in downloaded files.
///
/// Maps to yt-dlp's `--fixup` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FixupPolicy {
    /// Never attempt to fix issues.
    Never,
    /// Warn about fixable issues but do not fix them.
    Warn,
    /// Fix issues if detected, or warn if the fix is not possible (default).
    #[default]
    DetectOrWarn,
}

impl fmt::Display for FixupPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Never => "never",
            Self::Warn => "warn",
            Self::DetectOrWarn => "detect_or_warn",
        })
    }
}

impl FromStr for FixupPolicy {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("never") {
            Ok(Self::Never)
        } else if s.eq_ignore_ascii_case("warn") {
            Ok(Self::Warn)
        } else if s.eq_ignore_ascii_case("detect_or_warn") || s.eq_ignore_ascii_case("detect") {
            Ok(Self::DetectOrWarn)
        } else {
            Err(ParseEnumError {
                type_name: "FixupPolicy",
                input: s.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_detect_or_warn() {
        assert_eq!(FixupPolicy::default(), FixupPolicy::DetectOrWarn);
    }

    #[test]
    fn display_roundtrip() {
        for policy in [FixupPolicy::Never, FixupPolicy::Warn, FixupPolicy::DetectOrWarn] {
            let s = policy.to_string();
            let parsed: FixupPolicy = s.parse().unwrap();
            assert_eq!(policy, parsed, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn from_str_valid() {
        assert_eq!("never".parse::<FixupPolicy>().unwrap(), FixupPolicy::Never);
        assert_eq!("NEVER".parse::<FixupPolicy>().unwrap(), FixupPolicy::Never);
        assert_eq!("warn".parse::<FixupPolicy>().unwrap(), FixupPolicy::Warn);
        assert_eq!("WARN".parse::<FixupPolicy>().unwrap(), FixupPolicy::Warn);
        assert_eq!(
            "detect_or_warn".parse::<FixupPolicy>().unwrap(),
            FixupPolicy::DetectOrWarn
        );
        assert_eq!(
            "detect".parse::<FixupPolicy>().unwrap(),
            FixupPolicy::DetectOrWarn
        );
        assert_eq!(
            "DETECT_OR_WARN".parse::<FixupPolicy>().unwrap(),
            FixupPolicy::DetectOrWarn
        );
    }

    #[test]
    fn from_str_invalid() {
        let err = "invalid".parse::<FixupPolicy>().unwrap_err();
        assert_eq!(err.type_name, "FixupPolicy");
        assert_eq!(err.input, "invalid");
    }

    #[test]
    fn serde_roundtrip_never() {
        let policy = FixupPolicy::Never;
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, "\"never\"");
        let parsed: FixupPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, parsed);
    }

    #[test]
    fn serde_roundtrip_warn() {
        let policy = FixupPolicy::Warn;
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, "\"warn\"");
        let parsed: FixupPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, parsed);
    }

    #[test]
    fn serde_roundtrip_detect_or_warn() {
        let policy = FixupPolicy::DetectOrWarn;
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, "\"detect_or_warn\"");
        let parsed: FixupPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, parsed);
    }
}
