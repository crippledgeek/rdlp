//! Policy for automatic fixup of downloaded files.

use serde::Serialize;

use crate::parse_error::ParseEnumError;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};

/// Builds the `FromStr` error for [`FixupPolicy`].
///
/// Named by `#[strum(parse_err_fn = ...)]`. Replaces strum's default
/// `ParseError::VariantNotFound`, whose `Display` is the fixed string
/// "Matching variant not found" — that told a user editing `config.toml`
/// neither which value was rejected nor which field it came from (#583).
fn fixup_policy_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError::new("fixup policy", input)
}

/// Controls how rdlp handles fixable issues in downloaded files.
///
/// Maps to yt-dlp's `--fixup` option.
///
/// `Deserialize` delegates to strum's `FromStr` so `config.toml` accepts
/// exactly the vocabulary the CLI accepts — the alias `detect` and
/// case-insensitivity were CLI-only until #583. `Serialize` stays derived
/// under `#[serde(rename_all)]` on purpose: it is a wire contract, and
/// dropping the attribute would emit `"Never"` in place of `"never"`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    DeserializeFromStr,
    Default,
    Display,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive)]
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = fixup_policy_parse_err)]
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
    use crate::enum_test_support::{
        assert_serde_spellings_are_parseable, assert_toml_accepts_every_from_str_spelling,
        assert_toml_rejects_unknown_spelling,
    };

    /// Back-compat precondition for the #583 delegation, and the drift guard
    /// going forward: any variant whose serialized spelling `FromStr` rejects
    /// would have its persisted configs silently broken. Driven by `EnumIter`,
    /// so a newly added variant is covered without touching this test.
    #[test]
    fn test_serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<FixupPolicy>();
    }

    /// The config file must accept every spelling the CLI accepts (#583).
    ///
    /// `--fixup=detect` worked while `fixup = "detect"` in `config.toml` was an
    /// `unknown variant` error, because the CLI parsed via strum's `FromStr`
    /// (which knows the alias and is case-insensitive) and the config file
    /// parsed via a derived `Deserialize` (which knew neither).
    #[test]
    fn test_toml_accepts_every_cli_spelling() {
        assert_toml_accepts_every_from_str_spelling::<FixupPolicy>(&[
            "never",
            "warn",
            "detect_or_warn",
            // alias — CLI-only before #583
            "detect",
            // case-insensitivity — CLI-only before #583
            "DETECT",
            "Never",
        ]);
    }

    /// An unknown spelling must still be an error, and the message must name it.
    #[test]
    fn test_toml_rejects_unknown_spelling() {
        assert_toml_rejects_unknown_spelling::<FixupPolicy>(
            "nevr",
            "unsupported fixup policy: nevr",
        );
    }

    /// #583 widens `Deserialize` only. The serialized form is a wire contract,
    /// so it must not move — dropping `#[serde(rename_all)]` would emit
    /// `"Never"` instead of `"never"` and silently break persisted configs.
    #[test]
    fn test_serialize_still_emits_the_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&FixupPolicy::DetectOrWarn).expect("serialize"),
            "\"detect_or_warn\"",
            "the wire value must not change; only Deserialize was widened"
        );
        assert_eq!(
            serde_json::to_string(&FixupPolicy::Never).expect("serialize"),
            "\"never\""
        );
    }

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
