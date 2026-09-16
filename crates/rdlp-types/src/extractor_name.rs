//! The closed set of site extractors rdlp ships, as a type.

use serde_with::{DeserializeFromStr, SerializeDisplay};
use strum_macros::{Display, EnumIter, EnumString, IntoStaticStr};

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
/// A variant's name is spelled exactly once, in its `#[strum(serialize =
/// ...)]` attribute: `Display`/`FromStr`/`as_str` all derive from it
/// (`IntoStaticStr` implements `From<ExtractorName> for &'static str` off the
/// same attribute `Display`/`EnumString` read), and `Serialize` derives from
/// `Display` via `SerializeDisplay` rather than its own
/// `#[serde(rename = ...)]` per variant — the wire spellings are not a
/// uniform case transform of the variant identifiers (`ABXXX`, `EMPFlix`,
/// `9anime`, `PornoXO`, ...), so a single `#[serde(rename_all)]` cannot
/// express them, and three attributes each hand-spelling the same string
/// (`strum`, `serde`, and a hand-written `as_str` match) was exactly the
/// duplication this type exists to remove.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    SerializeDisplay,
    DeserializeFromStr,
    Display,
    EnumString,
    EnumIter,
    IntoStaticStr,
)]
#[strum(ascii_case_insensitive)]
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = extractor_name_parse_err)]
pub enum ExtractorName {
    /// abxxx.com
    #[strum(serialize = "ABXXX")]
    Abxxx,
    /// empflix.com
    #[strum(serialize = "EMPFlix")]
    EmpFlix,
    /// eporner.com
    #[strum(serialize = "EPorner")]
    EPorner,
    /// The generic fallback extractor for unrecognised URLs.
    #[strum(serialize = "Generic")]
    Generic,
    /// hqporner.com
    #[strum(serialize = "HQPorner")]
    HqPorner,
    /// koreanpornmovie.com
    #[strum(serialize = "KoreanPornMovie")]
    KoreanPornMovie,
    /// moviefap.com
    #[strum(serialize = "MovieFap")]
    MovieFap,
    /// 9anime.to. Spelled `9anime` on disk; the search side was aligned to
    /// it in #756.
    #[strum(serialize = "9anime")]
    NineAnime,
    /// pornhub.com
    #[strum(serialize = "PornHub")]
    PornHub,
    /// pornone.com
    #[strum(serialize = "PornOne")]
    PornOne,
    /// pornoxo.com
    #[strum(serialize = "PornoXO")]
    PornoXo,
    /// redtube.com
    #[strum(serialize = "RedTube")]
    RedTube,
    /// spankbang.com
    #[strum(serialize = "SpankBang")]
    SpankBang,
    /// tnaflix.com
    #[strum(serialize = "TNAFlix")]
    TnaFlix,
    /// xnxx.com
    #[strum(serialize = "XNXX")]
    Xnxx,
    /// xtits.com
    #[strum(serialize = "XTits")]
    XTits,
    /// xvideos.com
    #[strum(serialize = "XVideos")]
    XVideos,
}

impl ExtractorName {
    /// The canonical spelling as a `&'static str`, for the APIs that take a
    /// borrowed name (`InfoExtractor::name`, `InfoDict::new`, …).
    ///
    /// Delegates to the derived `IntoStaticStr` impl rather than a
    /// hand-written match, so the `#[strum(serialize = ...)]` attribute
    /// stays the only place a variant's name is spelled. Not `const fn`:
    /// `Into::into` cannot run at compile time, but nothing here needs it to
    /// — a `const NAME: ExtractorName = ExtractorName::PornHub` site only
    /// needs the *value* to be const, not this call-time conversion.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enum_test_support::{
        assert_display_matches, assert_display_roundtrips, assert_serde_spellings_are_parseable,
        assert_serialize_spellings,
    };
    use strum::IntoEnumIterator as _;

    /// The on-disk vocabulary. These strings are archive keys and folder
    /// names for existing users; a changed one is a data-loss bug, so every
    /// variant is pinned by value, not derived — and reused by every test
    /// below rather than each re-deriving its own expectation from the type
    /// under test.
    const EXPECTED: &[(ExtractorName, &str)] = &[
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
        (ExtractorName::Xnxx, "XNXX"),
        (ExtractorName::XTits, "XTits"),
        (ExtractorName::XVideos, "XVideos"),
    ];

    #[test]
    fn every_variant_renders_its_canonical_on_disk_name() {
        assert_eq!(
            EXPECTED.len(),
            ExtractorName::iter().count(),
            "every variant is pinned"
        );
        for &(variant, s) in EXPECTED {
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

    /// xhamster left the built-in set for the rdlp-plugins plugin (#771);
    /// a plugin-served site must not also be a closed-set variant, or the
    /// vocabulary would claim a site the binary no longer implements.
    #[test]
    fn extractor_name_has_no_xhamster() {
        assert!(ExtractorName::iter().all(|n| n.to_string() != "XHamster"));
        assert!("xhamster".parse::<ExtractorName>().is_err());
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

    /// `Serialize` must match `Display`/`as_str` exactly for every variant
    /// (not a 3-of-18 sample): `EXPECTED` is `EnumIter`-driven, so a variant
    /// added without stating its wire spelling here fails loudly rather than
    /// shipping unasserted, mirroring `container.rs`'s
    /// `test_serialize_still_emits_the_wire_spelling`.
    #[test]
    fn serialize_pins_the_wire_spelling_for_every_variant() {
        assert_serialize_spellings(EXPECTED);
    }

    /// The `Deserialize`/`FromStr` vocabulary must accept every spelling
    /// `Serialize` emits — trivially true here since both derive from
    /// `Display`, but pinned the same way every other enum in this crate is,
    /// so a future divergence (e.g. a hand-written `Deserialize`) is caught.
    #[test]
    fn serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<ExtractorName>();
    }
}
