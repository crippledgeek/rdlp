//! Shared assertions for the vocabulary, projection and wire-format guards on
//! the enums that cross a user-facing boundary.
//!
//! Six enums use these: `ContainerFormat`, `AudioFormat` and `SubtitleFormat`
//! (which name a string projection — an extension or a codec), plus
//! `FixupPolicy`, `BrowserType` and `SubtitleKind`. The latter three have no
//! projection at all, which is why the guarantees below are not all expressed
//! in terms of one.
//!
//! Four guarantees are pinned here, each for a failure that shipped:
//!
//! 1. **Vocabulary parity.** The serde surface and `FromStr` accept the same
//!    spellings. Before #540, `#[serde(rename_all)]` honoured neither strum's
//!    aliases nor its `ascii_case_insensitive`, so `remux_container = "3gp"`
//!    was a parse error in `config.toml` while `--remux=3gp` worked.
//! 2. **A diagnostic that names the rejected value**, since delegating
//!    `Deserialize` to `FromStr` moves the message to `FromStr::Err`.
//! 3. **`Display` renders the projection the enum names** — not whichever
//!    `#[strum(serialize = ...)]` spelling happens to be longest, which is what
//!    strum picks absent an explicit `to_string` (#545, #580). Which projection
//!    is asserted differs per enum, so callers pass it in.
//! 4. **The serialized form does not move.** It is a wire contract — a
//!    persisted config, or a TypeScript union over Tauri IPC — so unlike the
//!    others it is asserted against hardcoded literals rather than a
//!    projection: an expectation derived from the type under test cannot prove
//!    the type still agrees with what is already on disk.
//!
use std::fmt::{Debug, Display};
use std::str::FromStr;

use serde::Serialize;
use strum::IntoEnumIterator;

/// Minimal TOML document shape for the config-surface guards: `value = "..."`.
///
/// Shared by the accept and reject helpers so the field is genuinely read —
/// two local copies meant the reject helper's was write-only and tripped
/// `dead_code` under `-D warnings`.
#[derive(serde::Deserialize, Debug)]
struct Wrapper<U> {
    value: U,
}

/// Asserts the config-file (serde) and CLI (`FromStr`) surfaces accept exactly
/// the same vocabulary.
///
/// This is #540's central invariant. Before it, `#[serde(rename_all =
/// "lowercase")]` honoured neither strum's aliases nor its
/// `ascii_case_insensitive`, so `remux_container = "3gp"` was a parse error in
/// `config.toml` while `--remux=3gp` worked.
///
/// Deliberately driven through `toml`, not `serde_json`: the config file is the
/// surface that diverged, and the two crates make different borrowed-vs-owned
/// choices when handing a string to a `Visitor`. A delegation that works under
/// `serde_json` can still fail under `toml` (measured — `toml` 0.9 never yields
/// a borrowed `&str` through `from_str`), so the guard must exercise the real
/// format.
pub fn assert_toml_accepts_every_from_str_spelling<T>(spellings: &[&str])
where
    T: FromStr + PartialEq + Debug + serde::de::DeserializeOwned,
    <T as FromStr>::Err: Debug,
{
    for spelling in spellings {
        let expected = spelling
            .parse::<T>()
            .unwrap_or_else(|e| panic!("test bug: {spelling:?} must parse via FromStr: {e:?}"));

        let doc = format!("value = \"{spelling}\"");
        let parsed = toml::from_str::<Wrapper<T>>(&doc).unwrap_or_else(|e| {
            panic!("TOML rejects {spelling:?}, which the CLI accepts: {e}");
        });

        assert!(
            parsed.value == expected,
            "TOML parsed {spelling:?} to {:?}, but FromStr yields {expected:?}",
            parsed.value
        );
    }
}

/// Asserts TOML still *rejects* a spelling that is not in the vocabulary, and
/// that the reported error names the offending value.
///
/// The positive parity guards would all still pass if the delegation silently
/// fell back to a default instead of failing, so the widened surface needs a
/// negative case. The message assertion additionally pins the diagnostic:
/// routing deserialization through `FromStr` means the error text now comes
/// from `FromStr::Err`, and a bare "matching variant not found" would be a
/// regression against the derived `Deserialize` it replaced, which named both
/// the bad value and the accepted set.
pub fn assert_toml_rejects_unknown_spelling<T>(unknown: &str, expected_message: &str)
where
    T: FromStr + Debug + serde::de::DeserializeOwned,
    <T as FromStr>::Err: Debug,
{
    let doc = format!("value = \"{unknown}\"");
    let err = toml::from_str::<Wrapper<T>>(&doc)
        .err()
        .unwrap_or_else(|| panic!("TOML must reject {unknown:?}, but it parsed"));

    // Asserted against the *message*, not merely the rendered error: `toml`
    // echoes the offending source line, so a `contains(unknown)` check would
    // pass even when the message itself says nothing useful — which is exactly
    // what `strum::ParseError`'s "Matching variant not found" did.
    let rendered = err.to_string();
    assert!(
        rendered.contains(expected_message),
        "the error message must be {expected_message:?} so the user learns what \
         was wrong, not just where; got:\n{rendered}"
    );
}

/// Asserts every spelling the serde representation accepts is also in the
/// `FromStr` table.
///
/// This is the back-compat precondition for #540, which delegates `Deserialize`
/// to `FromStr` so the config file and the CLI stop accepting different
/// vocabularies. Any variant whose serialized spelling `FromStr` rejects would
/// have its persisted configs silently broken by that delegation — which is
/// exactly what `ContainerFormat::ThreeGp` (`"threegp"`) would have done.
///
/// Derived from the serialized form rather than a hand-listed set, so it cannot
/// drift as variants are added.
pub fn assert_serde_spellings_are_parseable<T>()
where
    T: IntoEnumIterator + Serialize + FromStr + PartialEq + Debug,
    <T as FromStr>::Err: Debug,
{
    for variant in T::iter() {
        let serialized = serde_json::to_string(&variant).expect("variant must serialize");
        let spelling = serialized.trim_matches('"');
        let parsed = spelling.parse::<T>().ok();
        assert!(
            parsed.as_ref() == Some(&variant),
            "serde accepts {spelling:?} for {variant:?}, but FromStr rejects it — \
             delegating Deserialize to FromStr would break persisted configs"
        );
    }
}

/// Asserts the serialized spelling of every variant, and that `expected`
/// covers every variant.
///
/// The delegation of `Deserialize` to `FromStr` (#540/#583/#586) widens the
/// *read* vocabulary only. The serialized form stays a wire contract: dropping
/// `#[serde(rename_all)]` would emit `"Never"` in place of `"never"` and
/// silently break persisted configs, and nothing else in the derive list would
/// complain.
///
/// Takes an explicit `(variant, spelling)` list rather than a projection
/// function, because the enums needing this guard do not share one: `as_str()`
/// exists on `BrowserType` and `SubtitleKind` but not on `FixupPolicy`, and
/// `Display` equals the wire spelling for `FixupPolicy` while differing for
/// `BrowserType` (whose longest alias is `google-chrome`, not `chrome`).
/// Asserting against `Display` would therefore pin a coincidence of strum's
/// longest-`serialize` rule rather than the contract.
///
/// The coverage half is what a hand-written `assert_eq!` per variant cannot do:
/// iterating `EnumIter` means a newly added variant fails this test until its
/// wire spelling is stated, instead of shipping unasserted.
pub fn assert_serialize_spellings<T>(expected: &[(T, &str)])
where
    T: IntoEnumIterator + Serialize + PartialEq + Debug + Copy,
{
    for variant in T::iter() {
        let (_, spelling) = expected
            .iter()
            .find(|(candidate, _)| *candidate == variant)
            .unwrap_or_else(|| {
                panic!(
                    "{variant:?} has no expected wire spelling — a new variant must \
                     state its serialized form here, since only Deserialize was widened"
                )
            });

        let serialized = serde_json::to_string(&variant).expect("variant must serialize");
        assert_eq!(
            serialized,
            format!("\"{spelling}\""),
            "the wire value for {variant:?} must not change; only Deserialize was widened"
        );
    }
}

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
