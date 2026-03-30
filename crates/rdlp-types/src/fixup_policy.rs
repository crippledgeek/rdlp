//! Policy for automatic fixup of downloaded files.

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

/// Controls how rdlp handles fixable issues in downloaded files.
///
/// Maps to yt-dlp's `--fixup` option.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive)]
pub enum FixupPolicy {
    /// Never attempt to fix issues.
    #[strum(serialize = "never")]
    Never,
    /// Warn about fixable issues but do not fix them.
    #[strum(serialize = "warn")]
    Warn,
    /// Fix issues if detected, or warn if the fix is not possible (default).
    #[default]
    #[strum(serialize = "detect_or_warn", serialize = "detect")]
    DetectOrWarn,
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
        for policy in [
            FixupPolicy::Never,
            FixupPolicy::Warn,
            FixupPolicy::DetectOrWarn,
        ] {
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
        assert!("invalid".parse::<FixupPolicy>().is_err());
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
