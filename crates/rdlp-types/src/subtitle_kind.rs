//! Subtitle track kind classification

use serde::Serialize;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};

use crate::parse_error::ParseEnumError;

/// Builds the `FromStr` error for [`SubtitleKind`].
///
/// Named by `#[strum(parse_err_fn = ...)]`. Replaces strum's default
/// `ParseError::VariantNotFound`, whose `Display` is the fixed string
/// "Matching variant not found" — routing `Deserialize` through `FromStr`
/// makes that the deserialization diagnostic, which would be a regression
/// against the derived `Deserialize` it replaces, whose error named both the
/// rejected value and the accepted set (#586, mirroring #540).
///
/// Latent today: `SubtitleKind` reaches serde only through `SubtitleTrack`,
/// which is built in-process and never deserialized from user, plugin or
/// network input. The diagnostic matters when that stops being true.
fn subtitle_kind_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError::new("subtitle kind", input)
}

/// Classification of subtitle track purpose.
///
/// `Deserialize` delegates to strum's `FromStr` so both surfaces accept one
/// vocabulary — the aliases `hearingimpaired`/`hi`/`sdh` and
/// case-insensitivity were `FromStr`-only until #586. `Serialize` stays
/// derived under `#[serde(rename_all)]` on purpose: it is a wire contract,
/// and dropping the attribute would emit `"HearingImpaired"` in place of
/// `"hearing_impaired"`.
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
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = subtitle_kind_parse_err)]
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
    pub const fn as_str(&self) -> &'static str {
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
    use crate::enum_test_support::{
        assert_all_parse_to, assert_display_matches, assert_display_roundtrips,
        assert_serde_spellings_are_parseable, assert_serialize_spellings,
        assert_toml_accepts_every_from_str_spelling, assert_toml_rejects_unknown_spelling,
    };

    #[test]
    fn test_default_is_normal() {
        assert_eq!(SubtitleKind::default(), SubtitleKind::Normal);
    }

    /// Driven by `EnumIter` rather than a hand-listed set, so a newly added
    /// variant is covered without touching this test.
    #[test]
    fn test_display_roundtrip() {
        assert_display_roundtrips::<SubtitleKind>();
    }

    /// `Display` must render the canonical spelling, not whichever alias
    /// happens to be longest — absent an explicit `to_string`, strum picks the
    /// longest `serialize` value. `hearing_impaired` wins that today only by
    /// being one character longer than `hearingimpaired`, so the agreement
    /// between `Display`, `as_str()` and the wire form is a coincidence of
    /// length until pinned here (#545, #580).
    #[test]
    fn test_display_equals_as_str() {
        assert_display_matches::<SubtitleKind>(|kind| kind.as_str(), "as_str()");
    }

    /// The alias table, with case folding exercised for free by the helper.
    #[test]
    fn test_from_str_aliases() {
        assert_all_parse_to(&[
            ("hearing_impaired", SubtitleKind::HearingImpaired),
            ("hearingimpaired", SubtitleKind::HearingImpaired),
            ("hi", SubtitleKind::HearingImpaired),
            ("sdh", SubtitleKind::HearingImpaired),
        ]);
    }

    /// #586 widens `Deserialize` only. The serialized form is a wire contract,
    /// so it must not move; the pre-existing roundtrip test below pinned one
    /// variant, which left five unasserted.
    #[test]
    fn test_serialize_still_emits_the_wire_spelling() {
        assert_serialize_spellings(&[
            (SubtitleKind::Normal, "normal"),
            (SubtitleKind::Forced, "forced"),
            (SubtitleKind::HearingImpaired, "hearing_impaired"),
            (SubtitleKind::Commentary, "commentary"),
            (SubtitleKind::Lyrics, "lyrics"),
            (SubtitleKind::Karaoke, "karaoke"),
        ]);
    }

    /// Precondition for the `Deserialize` -> `FromStr` delegation: no variant
    /// may have a serde spelling that `FromStr` rejects (#586, mirroring #540).
    #[test]
    fn test_serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<SubtitleKind>();
    }

    /// The serde surface must accept every spelling `FromStr` accepts (#586).
    ///
    /// `hi`, `sdh` and `hearingimpaired` are the aliases the derived
    /// `Deserialize` rejected while `FromStr` accepted them (#586). Unlike the
    /// sibling enums this mirrors, `SubtitleKind` has no CLI flag — the
    /// divergence was between `FromStr` and serde, not between CLI and config.
    #[test]
    fn test_toml_accepts_every_from_str_spelling() {
        assert_toml_accepts_every_from_str_spelling::<SubtitleKind>(&[
            "normal",
            "forced",
            "hearing_impaired",
            "commentary",
            "lyrics",
            "karaoke",
            // aliases the derived Deserialize rejected outright
            "hearingimpaired",
            "hi",
            "sdh",
            // case-insensitivity, which serde's rename_all never honoured
            "Normal",
            "SDH",
            "Hearing_Impaired",
        ]);
    }

    /// An unknown spelling must still be an error, and the message must name it
    /// — delegating to `FromStr` moves the diagnostic to `FromStr::Err`, and
    /// strum's default "Matching variant not found" would be a regression
    /// against the derived `Deserialize` it replaces (#586, mirroring #540).
    #[test]
    fn test_toml_rejects_unknown_spelling() {
        assert_toml_rejects_unknown_spelling::<SubtitleKind>(
            "nrmal",
            "unsupported subtitle kind: nrmal",
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
