//! Subtitle format types

use serde::Serialize;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};

/// Supported subtitle formats.
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
pub enum SubtitleFormat {
    /// `SubRip` Text
    #[strum(serialize = "srt")]
    Srt,
    /// Web Video Text Tracks
    #[strum(to_string = "vtt", serialize = "webvtt")]
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
    use crate::enum_test_support::{
        assert_all_parse_to, assert_display_matches, assert_display_roundtrips,
        assert_serde_spellings_are_parseable, assert_toml_accepts_every_from_str_spelling,
    };

    /// `Display` must render the file extension, not whichever strum alias
    /// happens to be longest (#580, mirroring the `ContainerFormat` guard from
    /// #545). Unlike `AudioFormat`, this enum has a single string projection,
    /// so `as_ext` is the one `Display` must agree with.
    #[test]
    fn test_display_equals_as_ext() {
        assert_display_matches::<SubtitleFormat>(|fmt| fmt.as_ext(), "as_ext()");
    }

    /// Promoting `vtt` from `serialize` to `to_string` must not drop `webvtt`:
    /// strum's `FromStr` table is `serialize` **plus** `to_string` (#580).
    #[test]
    fn test_alias_parsing() {
        assert_all_parse_to(&[
            ("vtt", SubtitleFormat::Vtt),
            ("webvtt", SubtitleFormat::Vtt),
        ]);
    }

    #[test]
    fn test_display_roundtrip() {
        assert_display_roundtrips::<SubtitleFormat>();
    }

    /// Precondition for #540's `Deserialize` -> `FromStr` delegation: no
    /// variant may have a serde spelling that `FromStr` rejects.
    #[test]
    fn test_serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<SubtitleFormat>();
    }

    /// The config file must accept every spelling the CLI accepts (#540).
    #[test]
    fn test_toml_accepts_every_cli_spelling() {
        assert_toml_accepts_every_from_str_spelling::<SubtitleFormat>(&[
            "srt", "vtt", "ass", "ssa", "lrc",
            // alias the config file used to reject outright
            "webvtt", // case-insensitivity, which serde's rename_all never honoured
            "SRT", "WebVTT",
        ]);
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
