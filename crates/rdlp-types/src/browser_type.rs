//! Browser type for cookie extraction

use serde::Serialize;

use crate::parse_error::ParseEnumError;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};

/// Builds the `FromStr` error for [`BrowserType`].
///
/// Named by `#[strum(parse_err_fn = ...)]`, replacing strum's fixed
/// "Matching variant not found" with a message that names the rejected
/// value (#583).
fn browser_type_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError::new("browser", input)
}

/// Supported browsers for cookie extraction.
///
/// `Deserialize` delegates to strum's `FromStr` so `config.toml` accepts the
/// same aliases the CLI does — `chromium`, `google-chrome` and `mozilla` were
/// CLI-only until #583. `Serialize` stays derived under
/// `#[serde(rename_all)]`: it is a wire contract and must not move.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    DeserializeFromStr,
    Display,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = browser_type_parse_err)]
pub enum BrowserType {
    /// Google Chrome / Chromium
    #[strum(
        serialize = "chrome",
        serialize = "chromium",
        serialize = "google-chrome"
    )]
    Chrome,
    /// Mozilla Firefox
    #[strum(serialize = "firefox", serialize = "mozilla")]
    Firefox,
}

impl BrowserType {
    /// Canonical name for display and matching.
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enum_test_support::{
        assert_serde_spellings_are_parseable, assert_toml_accepts_every_from_str_spelling,
        assert_toml_rejects_unknown_spelling,
    };

    /// Back-compat precondition for the #583 delegation, and the drift guard
    /// going forward. Driven by `EnumIter`, so a new variant is covered without
    /// touching this test.
    #[test]
    fn test_serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<BrowserType>();
    }

    /// The config file must accept every spelling the CLI accepts (#583).
    ///
    /// `--cookies-from-browser chromium` worked while
    /// `cookies_from_browser = "chromium"` was an `unknown variant` error.
    #[test]
    fn test_toml_accepts_every_cli_spelling() {
        assert_toml_accepts_every_from_str_spelling::<BrowserType>(&[
            "chrome",
            "firefox",
            // aliases — CLI-only before #583
            "chromium",
            "google-chrome",
            "mozilla",
            // case-insensitivity — CLI-only before #583
            "CHROME",
        ]);
    }

    /// An unknown spelling must still be an error, and the message must name it.
    #[test]
    fn test_toml_rejects_unknown_spelling() {
        assert_toml_rejects_unknown_spelling::<BrowserType>(
            "safari",
            "unsupported browser: safari",
        );
    }

    /// #583 widens `Deserialize` only; the serialized form must not move.
    #[test]
    fn test_serialize_still_emits_the_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&BrowserType::Chrome).expect("serialize"),
            "\"chrome\"",
            "the wire value must not change; only Deserialize was widened"
        );
    }

    #[test]
    fn test_parse_variants() {
        assert_eq!(
            "chrome".parse::<BrowserType>().unwrap(),
            BrowserType::Chrome
        );
        assert_eq!(
            "chromium".parse::<BrowserType>().unwrap(),
            BrowserType::Chrome
        );
        assert_eq!(
            "google-chrome".parse::<BrowserType>().unwrap(),
            BrowserType::Chrome
        );
        assert_eq!(
            "firefox".parse::<BrowserType>().unwrap(),
            BrowserType::Firefox
        );
        assert_eq!(
            "mozilla".parse::<BrowserType>().unwrap(),
            BrowserType::Firefox
        );
        assert!("safari".parse::<BrowserType>().is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let b = BrowserType::Chrome;
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(json, "\"chrome\"");
        let parsed: BrowserType = serde_json::from_str(&json).unwrap();
        assert_eq!(b, parsed);
    }
}
