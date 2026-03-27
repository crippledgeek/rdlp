//! Subtitle track kind classification

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::parse_error::ParseEnumError;

/// Classification of subtitle track purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleKind {
    /// Standard dialogue subtitles
    #[default]
    Normal,
    /// Forced/burn-in subtitles (foreign language parts only)
    Forced,
    /// Subtitles for deaf/hard-of-hearing (includes sound effects)
    HearingImpaired,
    /// Director/cast commentary track
    Commentary,
    /// Lyrics (music content)
    Lyrics,
    /// Karaoke-style timed lyrics
    Karaoke,
}

impl SubtitleKind {
    /// String identifier for this kind.
    ///
    /// # Returns
    ///
    /// A static string slice matching the serde `snake_case` representation.
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::SubtitleKind;
    /// assert_eq!(SubtitleKind::HearingImpaired.as_str(), "hearing_impaired");
    /// ```
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Forced => "forced",
            Self::HearingImpaired => "hearing_impaired",
            Self::Commentary => "commentary",
            Self::Lyrics => "lyrics",
            Self::Karaoke => "karaoke",
        }
    }
}

impl fmt::Display for SubtitleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SubtitleKind {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("normal") {
            Ok(Self::Normal)
        } else if s.eq_ignore_ascii_case("forced") {
            Ok(Self::Forced)
        } else if s.eq_ignore_ascii_case("hearing_impaired")
            || s.eq_ignore_ascii_case("hearingimpaired")
            || s.eq_ignore_ascii_case("hi")
            || s.eq_ignore_ascii_case("sdh")
        {
            Ok(Self::HearingImpaired)
        } else if s.eq_ignore_ascii_case("commentary") {
            Ok(Self::Commentary)
        } else if s.eq_ignore_ascii_case("lyrics") {
            Ok(Self::Lyrics)
        } else if s.eq_ignore_ascii_case("karaoke") {
            Ok(Self::Karaoke)
        } else {
            Err(ParseEnumError {
                type_name: "SubtitleKind",
                input: s.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_normal() {
        assert_eq!(SubtitleKind::default(), SubtitleKind::Normal);
    }

    #[test]
    fn test_display_from_str_roundtrip() {
        for kind in [
            SubtitleKind::Normal,
            SubtitleKind::Forced,
            SubtitleKind::HearingImpaired,
            SubtitleKind::Commentary,
            SubtitleKind::Lyrics,
            SubtitleKind::Karaoke,
        ] {
            let s = kind.to_string();
            let parsed: SubtitleKind = s.parse().unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn test_from_str_aliases() {
        assert_eq!(
            "hi".parse::<SubtitleKind>().unwrap(),
            SubtitleKind::HearingImpaired
        );
        assert_eq!(
            "sdh".parse::<SubtitleKind>().unwrap(),
            SubtitleKind::HearingImpaired
        );
    }

    #[test]
    fn test_serde_roundtrip() {
        let kind = SubtitleKind::HearingImpaired;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"hearing_impaired\"");
        let parsed: SubtitleKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, parsed);
    }
}
