//! Subtitle format types

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

/// Supported subtitle formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
pub enum SubtitleFormat {
    /// `SubRip` Text
    #[strum(serialize = "srt")]
    Srt,
    /// Web Video Text Tracks
    #[strum(serialize = "vtt", serialize = "webvtt")]
    Vtt,
    /// Advanced `SubStation` Alpha
    #[strum(serialize = "ass")]
    Ass,
    /// `SubStation` Alpha
    #[strum(serialize = "ssa")]
    Ssa,
    /// LRC lyrics format
    #[strum(serialize = "lrc")]
    Lrc,
}

impl SubtitleFormat {
    /// File extension for this subtitle format.
    #[inline]
    #[must_use]
    pub const fn as_ext(&self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Ass => "ass",
            Self::Ssa => "ssa",
            Self::Lrc => "lrc",
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
