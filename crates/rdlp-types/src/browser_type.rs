//! Browser type for cookie extraction

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::parse_error::ParseEnumError;

/// Supported browsers for cookie extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserType {
    /// Google Chrome / Chromium
    Chrome,
    /// Mozilla Firefox
    Firefox,
}

impl BrowserType {
    /// Canonical name for display and matching.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
        }
    }
}

impl fmt::Display for BrowserType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BrowserType {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("chrome")
            || s.eq_ignore_ascii_case("chromium")
            || s.eq_ignore_ascii_case("google-chrome")
        {
            Ok(Self::Chrome)
        } else if s.eq_ignore_ascii_case("firefox") || s.eq_ignore_ascii_case("mozilla") {
            Ok(Self::Firefox)
        } else {
            Err(ParseEnumError {
                type_name: "BrowserType",
                input: s.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
