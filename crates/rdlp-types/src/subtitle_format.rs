//! Subtitle format types

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::parse_error::ParseEnumError;

/// Supported subtitle formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleFormat {
    /// SubRip Text
    Srt,
    /// Web Video Text Tracks
    Vtt,
    /// Advanced SubStation Alpha
    Ass,
    /// SubStation Alpha
    Ssa,
    /// LRC lyrics format
    Lrc,
}

impl SubtitleFormat {
    /// File extension for this subtitle format.
    #[must_use]
    pub fn as_ext(&self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass",
            Self::Ssa => "ssa",
            Self::Lrc => "lrc",
        }
    }
}

impl fmt::Display for SubtitleFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ext())
    }
}

impl FromStr for SubtitleFormat {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("srt") {
            Ok(Self::Srt)
        } else if s.eq_ignore_ascii_case("vtt") || s.eq_ignore_ascii_case("webvtt") {
            Ok(Self::Vtt)
        } else if s.eq_ignore_ascii_case("ass") {
            Ok(Self::Ass)
        } else if s.eq_ignore_ascii_case("ssa") {
            Ok(Self::Ssa)
        } else if s.eq_ignore_ascii_case("lrc") {
            Ok(Self::Lrc)
        } else {
            Err(ParseEnumError {
                type_name: "SubtitleFormat",
                input: s.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_roundtrip() {
        for fmt in [
            SubtitleFormat::Srt,
            SubtitleFormat::Vtt,
            SubtitleFormat::Ass,
            SubtitleFormat::Ssa,
            SubtitleFormat::Lrc,
        ] {
            let s = fmt.to_string();
            let parsed: SubtitleFormat = s.parse().unwrap();
            assert_eq!(fmt, parsed);
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let fmt = SubtitleFormat::Srt;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(json, "\"srt\"");
        let parsed: SubtitleFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, parsed);
    }
}
