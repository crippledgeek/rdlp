//! Shared assertions for the string-projection guards on the format enums.
//!
//! `ContainerFormat`, `AudioFormat` and `SubtitleFormat` each derive
//! `strum::Display` over a variant table that also lists parse aliases, and each
//! needs the same three guarantees pinned:
//!
//! 1. every variant's `Display` output parses back to that variant,
//! 2. `Display` renders the projection the enum actually names — **not**
//!    whichever `#[strum(serialize = ...)]` spelling happens to be longest,
//!    which is what strum picks absent an explicit `to_string` (#545, #580), and
//! 3. promoting a spelling to `to_string` did not drop any other spelling from
//!    the `FromStr` table.
//!
//! Which projection guarantee 2 asserts differs per enum — `ContainerFormat`
//! and `SubtitleFormat` name file extensions, while `AudioFormat` names codecs
//! and its `as_ext()` deliberately returns the *container* the codec is carried
//! in — so the projection is passed in as a function rather than hardcoded here.
//! Callers supply it; this module supplies the loop, the iteration source and
//! the failure message.

use std::fmt::{Debug, Display};
use std::str::FromStr;

use strum::IntoEnumIterator;

/// Asserts every variant's `Display` output parses back to that variant.
///
/// Iterates `EnumIter` rather than a hand-listed set, so a newly added variant
/// is covered without touching the test.
pub fn assert_display_roundtrips<T>()
where
    T: IntoEnumIterator + Display + FromStr + PartialEq + Debug,
    <T as FromStr>::Err: Debug,
{
    for variant in T::iter() {
        let rendered = variant.to_string();
        let parsed = rendered
            .parse::<T>()
            .unwrap_or_else(|e| panic!("Display output {rendered:?} must parse back: {e:?}"));
        assert_eq!(variant, parsed, "roundtrip failed for {rendered}");
    }
}

/// Asserts `Display` equals `projection` for every variant.
///
/// `projection_name` names the accessor in the failure message so a failure
/// reads `Display for Dts must equal codec_name()` rather than pointing at this
/// helper.
pub fn assert_display_matches<T>(projection: impl Fn(T) -> &'static str, projection_name: &str)
where
    T: IntoEnumIterator + Display + Debug + Copy,
{
    for variant in T::iter() {
        assert_eq!(
            variant.to_string(),
            projection(variant),
            "Display for {variant:?} must equal {projection_name}"
        );
    }
}

/// Asserts every `(input, expected)` pair parses to `expected`, in each of its
/// as-written, upper- and lower-case forms.
///
/// Guards the `serialize` -> `to_string` promotions: strum unions `to_string`
/// into the `FromStr` set rather than replacing the `serialize` list, so no
/// spelling should be lost — but that is exactly how such a promotion could
/// silently narrow the accepted CLI/config vocabulary, so it is pinned per enum.
///
/// Case folding is asserted here rather than per enum because all three format
/// enums carry `#[strum(ascii_case_insensitive)]`, which strum applies to every
/// spelling a variant accepts — `to_string` values included, not just
/// `serialize` ones. A promotion that somehow escaped the case-insensitive path
/// would therefore fail here without needing its own test per enum.
pub fn assert_all_parse_to<T>(cases: &[(&str, T)])
where
    T: FromStr + PartialEq + Debug + Copy,
    <T as FromStr>::Err: Debug,
{
    for &(input, expected) in cases {
        for spelling in [input.to_owned(), input.to_uppercase(), input.to_lowercase()] {
            let parsed = spelling
                .parse::<T>()
                .unwrap_or_else(|e| panic!("{spelling:?} must parse: {e:?}"));
            assert_eq!(parsed, expected, "{spelling} must parse to {expected:?}");
        }
    }
}
