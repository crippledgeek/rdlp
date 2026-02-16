//! Subtitle track kind classification

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "normal" => Ok(Self::Normal),
            "forced" => Ok(Self::Forced),
            "hearing_impaired" | "hearingimpaired" | "hi" | "sdh" => Ok(Self::HearingImpaired),
            "commentary" => Ok(Self::Commentary),
            "lyrics" => Ok(Self::Lyrics),
            "karaoke" => Ok(Self::Karaoke),
            _ => Err(format!("unsupported subtitle kind: {s}")),
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
