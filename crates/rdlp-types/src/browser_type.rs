//! Browser type for cookie extraction

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

/// Supported browsers for cookie extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
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
