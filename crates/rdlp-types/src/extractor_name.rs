//! The closed set of site extractors rdlp ships, as a type.

use serde::Serialize;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};

use crate::parse_error::ParseEnumError;

/// Builds the `FromStr` error for [`ExtractorName`].
///
/// Named by `#[strum(parse_err_fn = ...)]`. Replaces strum's default
/// `ParseError::VariantNotFound`, whose `Display` is the fixed string
/// "Matching variant not found" — that told a user editing `--search-site`
/// neither which value was rejected nor which field it came from, mirroring
/// `container_format_parse_err` (#540).
fn extractor_name_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError::new("extractor name", input)
}

/// Every built-in extractor's display name (#756).
///
/// `Display`/`as_str` render the canonical spelling — the
/// `--download-archive` key, the `%(extractor)s` template value and the
/// `--dump-json` field, so a variant's string is an on-disk contract: change
/// it only with a migration. Plugins are not here; their names are free
/// strings and the `&str` APIs stay for them.
///
/// `Serialize` is pinned per variant (`#[serde(rename = "...")]`) rather than
/// derived under a single `#[serde(rename_all)]` casing, because the wire
/// spellings are not a uniform case transform of the variant identifiers
/// (`ABXXX`, `EMPFlix`, `9anime`, `PornoXO`, ...) — unlike every other enum in
/// this crate. Serialize therefore equals Display/`as_str` exactly, which the
/// tests below pin.
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
#[strum(ascii_case_insensitive)]
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = extractor_name_parse_err)]
pub enum ExtractorName {
    /// abxxx.com
    #[serde(rename = "ABXXX")]
    #[strum(serialize = "ABXXX")]
    Abxxx,
    /// empflix.com
    #[serde(rename = "EMPFlix")]
    #[strum(serialize = "EMPFlix")]
    EmpFlix,
    /// eporner.com
    #[serde(rename = "EPorner")]
    #[strum(serialize = "EPorner")]
    EPorner,
    /// The generic fallback extractor for unrecognised URLs.
    #[serde(rename = "Generic")]
    #[strum(serialize = "Generic")]
    Generic,
    /// hqporner.com
    #[serde(rename = "HQPorner")]
    #[strum(serialize = "HQPorner")]
    HqPorner,
    /// koreanpornmovie.com
    #[serde(rename = "KoreanPornMovie")]
    #[strum(serialize = "KoreanPornMovie")]
    KoreanPornMovie,
    /// moviefap.com
    #[serde(rename = "MovieFap")]
    #[strum(serialize = "MovieFap")]
    MovieFap,
    /// 9anime.to. Spelled `9anime` on disk; the search side was aligned to
    /// it in #756.
    #[serde(rename = "9anime")]
    #[strum(serialize = "9anime")]
    NineAnime,
    /// pornhub.com
    #[serde(rename = "PornHub")]
    #[strum(serialize = "PornHub")]
    PornHub,
    /// pornone.com
    #[serde(rename = "PornOne")]
    #[strum(serialize = "PornOne")]
    PornOne,
    /// pornoxo.com
    #[serde(rename = "PornoXO")]
    #[strum(serialize = "PornoXO")]
    PornoXo,
    /// redtube.com
    #[serde(rename = "RedTube")]
    #[strum(serialize = "RedTube")]
    RedTube,
    /// spankbang.com
    #[serde(rename = "SpankBang")]
    #[strum(serialize = "SpankBang")]
    SpankBang,
    /// tnaflix.com
    #[serde(rename = "TNAFlix")]
    #[strum(serialize = "TNAFlix")]
    TnaFlix,
    /// xhamster.com
    #[serde(rename = "XHamster")]
    #[strum(serialize = "XHamster")]
    XHamster,
    /// xnxx.com
    #[serde(rename = "XNXX")]
    #[strum(serialize = "XNXX")]
    Xnxx,
    /// xtits.com
    #[serde(rename = "XTits")]
    #[strum(serialize = "XTits")]
    XTits,
    /// xvideos.com
    #[serde(rename = "XVideos")]
    #[strum(serialize = "XVideos")]
    XVideos,
}

impl ExtractorName {
    /// The canonical spelling as a `&'static str`, for the APIs that take a
    /// borrowed name (`InfoExtractor::name`, `InfoDict::new`, …).
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Abxxx => "ABXXX",
            Self::EmpFlix => "EMPFlix",
            Self::EPorner => "EPorner",
            Self::Generic => "Generic",
            Self::HqPorner => "HQPorner",
            Self::KoreanPornMovie => "KoreanPornMovie",
            Self::MovieFap => "MovieFap",
            Self::NineAnime => "9anime",
            Self::PornHub => "PornHub",
            Self::PornOne => "PornOne",
            Self::PornoXo => "PornoXO",
            Self::RedTube => "RedTube",
            Self::SpankBang => "SpankBang",
            Self::TnaFlix => "TNAFlix",
            Self::XHamster => "XHamster",
            Self::Xnxx => "XNXX",
            Self::XTits => "XTits",
            Self::XVideos => "XVideos",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enum_test_support::{assert_display_matches, assert_display_roundtrips};
    use strum::IntoEnumIterator as _;

    /// The on-disk vocabulary. These strings are archive keys and folder
    /// names for existing users; a changed one is a data-loss bug, so every
    /// variant is pinned by value, not derived.
    #[test]
    fn every_variant_renders_its_canonical_on_disk_name() {
        let expected = [
            (ExtractorName::Abxxx, "ABXXX"),
            (ExtractorName::EmpFlix, "EMPFlix"),
            (ExtractorName::EPorner, "EPorner"),
            (ExtractorName::Generic, "Generic"),
            (ExtractorName::HqPorner, "HQPorner"),
            (ExtractorName::KoreanPornMovie, "KoreanPornMovie"),
            (ExtractorName::MovieFap, "MovieFap"),
            (ExtractorName::NineAnime, "9anime"),
            (ExtractorName::PornHub, "PornHub"),
            (ExtractorName::PornOne, "PornOne"),
            (ExtractorName::PornoXo, "PornoXO"),
            (ExtractorName::RedTube, "RedTube"),
            (ExtractorName::SpankBang, "SpankBang"),
            (ExtractorName::TnaFlix, "TNAFlix"),
            (ExtractorName::XHamster, "XHamster"),
            (ExtractorName::Xnxx, "XNXX"),
            (ExtractorName::XTits, "XTits"),
            (ExtractorName::XVideos, "XVideos"),
        ];
        assert_eq!(
            expected.len(),
            ExtractorName::iter().count(),
            "every variant is pinned"
        );
        for (variant, s) in expected {
            assert_eq!(variant.as_str(), s);
            assert_eq!(variant.to_string(), s);
            assert_eq!(s.parse::<ExtractorName>().ok(), Some(variant));
        }
    }

    /// `--search-site nineanime`/`9ANIME` etc. must keep resolving.
    #[test]
    fn parses_ascii_case_insensitively() {
        assert_eq!(
            "pornhub".parse::<ExtractorName>().ok(),
            Some(ExtractorName::PornHub)
        );
        assert_eq!(
            "9ANIME".parse::<ExtractorName>().ok(),
            Some(ExtractorName::NineAnime)
        );
    }

    #[test]
    fn rejects_unknown_names_with_the_field_in_the_error() {
        let err = "youtube".parse::<ExtractorName>().unwrap_err();
        assert!(err.to_string().contains("youtube"), "{err}");
    }

    #[test]
    fn display_roundtrip_and_serde_spellings() {
        assert_display_roundtrips::<ExtractorName>();
        assert_display_matches::<ExtractorName>(|n| n.as_str(), "as_str()");
    }

    /// `Serialize` must match `Display`/`as_str` exactly — pinned literally
    /// (not derived from `as_str()`) because this is the wire contract the
    /// crate-level doc-comment describes; a helper deriving its own
    /// expectation from the type under test could not catch the type
    /// silently drifting from the string already on disk.
    #[test]
    fn serialize_pins_the_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&ExtractorName::NineAnime).unwrap(),
            "\"9anime\""
        );
        assert_eq!(
            serde_json::to_string(&ExtractorName::PornoXo).unwrap(),
            "\"PornoXO\""
        );
        assert_eq!(
            serde_json::to_string(&ExtractorName::Abxxx).unwrap(),
            "\"ABXXX\""
        );
    }
}
