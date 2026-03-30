//! Subtitle track kind classification

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

/// Classification of subtitle track purpose.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default, Display, EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(ascii_case_insensitive)]
pub enum SubtitleKind {
    /// Standard dialogue subtitles
    #[default]
    #[strum(serialize = "normal")]
    Normal,
    /// Forced/burn-in subtitles (foreign language parts only)
    #[strum(serialize = "forced")]
    Forced,
    /// Subtitles for deaf/hard-of-hearing (includes sound effects)
    #[strum(
        serialize = "hearing_impaired",
        serialize = "hearingimpaired",
        serialize = "hi",
        serialize = "sdh"
    )]
    HearingImpaired,
    /// Director/cast commentary track
    #[strum(serialize = "commentary")]
    Commentary,
    /// Lyrics (music content)
    #[strum(serialize = "lyrics")]
    Lyrics,
    /// Karaoke-style timed lyrics
    #[strum(serialize = "karaoke")]
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
