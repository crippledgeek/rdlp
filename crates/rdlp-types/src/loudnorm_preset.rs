//! EBU R128 loudnorm target presets — the single owner of their targets.

use serde::Serialize;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};

use crate::parse_error::ParseEnumError;

/// Builds the `FromStr` error for [`LoudnormPreset`].
///
/// Named by `#[strum(parse_err_fn = ...)]`, replacing strum's fixed
/// "Matching variant not found" so a rejected `--loudnorm-preset` or
/// `config.toml` value is echoed back to the user (#583 pattern).
fn loudnorm_preset_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError::new("loudnorm preset", input)
}

/// The three loudness targets the `loudnorm` filter takes, in the units
/// `FFmpeg` takes them.
///
/// A named struct rather than the `(f64, f64, f64)` tuple it replaces: three
/// same-typed values in a fixed order are exactly the shape that lets a
/// caller swap true-peak and loudness-range without a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LoudnormTargets {
    /// Integrated loudness target (`loudnorm=I=`), in LUFS.
    pub integrated_lufs: f64,
    /// Maximum true peak (`loudnorm=TP=`), in dBTP.
    pub true_peak_dbtp: f64,
    /// Loudness range target (`loudnorm=LRA=`), in LU.
    pub range_lu: f64,
}

/// One preset paired with its targets — the catalogue row a GUI renders as
/// `"Broadcast (-23 LUFS)"`.
///
/// Served over IPC by the desktop's `loudnorm_presets` command from
/// [`LoudnormPreset::describe_all`], so the per-item numbers in a preset
/// picker come from the owner rather than a typed copy (#611).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LoudnormPresetInfo {
    /// The preset, in its lowercase wire spelling.
    pub preset: LoudnormPreset,
    /// Its I/TP/LRA targets.
    pub targets: LoudnormTargets,
}

/// Loudnorm target presets for common delivery targets.
///
/// This enum is the ONLY place the per-preset I/TP/LRA values live (#611):
/// `rdlp-ffmpeg`'s `NormalizeOptions::default`, the post-process stage, the
/// CLI help text and the desktop's settings placeholders all read
/// [`Self::targets`] rather than carrying a copy — the desktop copy was
/// Streaming-only and therefore WRONG whenever `Loud` or `Broadcast` was
/// selected.
///
/// # Where the numbers come from
///
/// - **Broadcast** (−23 LUFS / −2 dBTP / 7 LU) is the EBU R128 programme
///   loudness target, and matches `ffmpeg-normalize`'s shipped defaults.
///   `FFmpeg`'s own `loudnorm` filter defaults differ by 1 LU — `I=-24`,
///   `LRA=7`, `TP=-2` (`libavfilter/af_loudnorm.c`, `loudnorm_options[]`,
///   `FFmpeg` 8.0.1) — so an rdlp Broadcast run is 1 LU louder than a bare
///   `-af loudnorm`; R128 is the authority being followed, not the filter.
/// - **Streaming** I = −14 LUFS and TP = −1 dBTP are the Spotify
///   loudness-normalization figures (its published reference level, and its
///   ceiling for masters at −14). The −1 dBTP ceiling is a deliberate
///   divergence from the conservative −2 dBTP broadcast convention. The
///   LRA of 11 LU has no external citation: it is an rdlp tuning choice.
/// - **Loud** (−11 / −1 / 11) is an rdlp tuning choice for loud masters,
///   backed by no standard.
///
/// `Default` is `Streaming` — the value every unset `loudnorm_preset` used to
/// collapse to at three separate call sites.
///
/// `Deserialize` delegates to strum's `FromStr` so `config.toml`, the CLI and
/// the desktop's `settings.json` accept one case-insensitive vocabulary.
/// `Serialize` stays derived under `#[serde(rename_all = "lowercase")]` on
/// purpose: it is a wire contract (`"streaming"`), and
/// `scripts/check-ts-enum-drift.sh` maps variants to the TypeScript union by
/// lowercasing the identifier under exactly that attribute.
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
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = loudnorm_preset_parse_err)]
pub enum LoudnormPreset {
    /// EBU R128 broadcast: I=−23 LUFS, TP=−2 dBTP, LRA=7 LU.
    #[strum(serialize = "broadcast")]
    Broadcast,
    /// Streaming-platform delivery: I=−14 LUFS, TP=−1 dBTP, LRA=11 LU.
    #[default]
    #[strum(serialize = "streaming")]
    Streaming,
    /// Loud master: I=−11 LUFS, TP=−1 dBTP, LRA=11 LU.
    #[strum(serialize = "loud")]
    Loud,
}

impl LoudnormPreset {
    /// Every preset, in display order, for callers without `strum` in scope
    /// (the CLI renders its `--loudnorm-preset` help from this). Pinned equal
    /// to the `EnumIter` order by a test.
    pub const ALL: [Self; 3] = [Self::Broadcast, Self::Streaming, Self::Loud];

    /// Every preset with its targets, in [`Self::ALL`] order.
    #[must_use]
    pub fn describe_all() -> [LoudnormPresetInfo; 3] {
        Self::ALL.map(|preset| LoudnormPresetInfo {
            preset,
            targets: preset.targets(),
        })
    }

    /// The I/TP/LRA targets this preset stands for. See the type-level doc
    /// for where each number comes from.
    #[must_use]
    pub const fn targets(self) -> LoudnormTargets {
        match self {
            Self::Broadcast => LoudnormTargets {
                integrated_lufs: -23.0,
                true_peak_dbtp: -2.0,
                range_lu: 7.0,
            },
            Self::Streaming => LoudnormTargets {
                integrated_lufs: -14.0,
                true_peak_dbtp: -1.0,
                range_lu: 11.0,
            },
            Self::Loud => LoudnormTargets {
                integrated_lufs: -11.0,
                true_peak_dbtp: -1.0,
                range_lu: 11.0,
            },
        }
    }
}

#[cfg(test)]
// float_cmp: the targets are literal constants read back unchanged, so exact
// equality IS the oracle; an epsilon would accept a drifted value.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::enum_test_support::{
        assert_display_roundtrips, assert_serde_spellings_are_parseable,
        assert_serialize_spellings, assert_toml_accepts_every_from_str_spelling,
        assert_toml_rejects_unknown_spelling,
    };

    /// Pins the documented values. The one place the literals may appear as
    /// a test oracle; every other test compares against `targets()`.
    #[test]
    fn targets_are_the_documented_ones() {
        let LoudnormTargets {
            integrated_lufs,
            true_peak_dbtp,
            range_lu,
        } = LoudnormPreset::Broadcast.targets();
        assert_eq!(
            (integrated_lufs, true_peak_dbtp, range_lu),
            (-23.0, -2.0, 7.0)
        );

        let LoudnormTargets {
            integrated_lufs,
            true_peak_dbtp,
            range_lu,
        } = LoudnormPreset::Streaming.targets();
        assert_eq!(
            (integrated_lufs, true_peak_dbtp, range_lu),
            (-14.0, -1.0, 11.0)
        );

        let LoudnormTargets {
            integrated_lufs,
            true_peak_dbtp,
            range_lu,
        } = LoudnormPreset::Loud.targets();
        assert_eq!(
            (integrated_lufs, true_peak_dbtp, range_lu),
            (-11.0, -1.0, 11.0)
        );
    }

    /// `ALL` is hand-written so `strum`-free crates can iterate it; this pins
    /// it to the real variant set two ways — `EnumIter` (order and count) and
    /// an exhaustive `match` that stops compiling when a variant is added
    /// without a row in `ALL` being considered.
    #[test]
    fn all_lists_every_variant_in_declaration_order() {
        use strum::IntoEnumIterator;
        assert_eq!(
            LoudnormPreset::ALL.to_vec(),
            LoudnormPreset::iter().collect::<Vec<_>>()
        );
        for preset in LoudnormPreset::iter() {
            let listed = match preset {
                LoudnormPreset::Broadcast | LoudnormPreset::Streaming | LoudnormPreset::Loud => {
                    LoudnormPreset::ALL.contains(&preset)
                }
            };
            assert!(listed, "{preset:?} missing from ALL");
        }
    }

    /// The catalogue is `ALL` zipped with `targets()`, and its wire shape is
    /// the field names verbatim with the preset in lowercase.
    #[test]
    fn describe_all_pairs_each_preset_with_its_own_targets() {
        let rows = LoudnormPreset::describe_all();
        assert_eq!(rows.len(), LoudnormPreset::ALL.len());
        for (row, preset) in rows.iter().zip(LoudnormPreset::ALL) {
            assert_eq!(row.preset, preset);
            assert_eq!(row.targets, preset.targets());
        }
        let first = rows.first().copied().expect("three rows");
        let json = serde_json::to_value(first).expect("serialize");
        assert_eq!(json.get("preset"), Some(&"broadcast".into()));
        let targets = json
            .get("targets")
            .and_then(serde_json::Value::as_object)
            .expect("targets object");
        assert_eq!(
            targets.get("integrated_lufs"),
            Some(&LoudnormPreset::Broadcast.targets().integrated_lufs.into())
        );
        assert_eq!(targets.len(), 3, "exactly three targets on the wire");
    }

    #[test]
    fn default_is_streaming() {
        assert_eq!(LoudnormPreset::default(), LoudnormPreset::Streaming);
    }

    #[test]
    fn from_str_is_case_insensitive() {
        assert_eq!(
            "broadcast".parse::<LoudnormPreset>().unwrap(),
            LoudnormPreset::Broadcast
        );
        assert_eq!(
            "Streaming".parse::<LoudnormPreset>().unwrap(),
            LoudnormPreset::Streaming
        );
        assert_eq!(
            "LOUD".parse::<LoudnormPreset>().unwrap(),
            LoudnormPreset::Loud
        );
    }

    /// The rejection names the value, not just "variant not found".
    #[test]
    fn from_str_rejects_and_names_the_input() {
        let err = "quiet".parse::<LoudnormPreset>().unwrap_err();
        assert_eq!(err.to_string(), "unsupported loudnorm preset: quiet");
    }

    #[test]
    fn display_roundtrips_through_from_str() {
        assert_display_roundtrips::<LoudnormPreset>();
        assert_eq!(LoudnormPreset::Broadcast.to_string(), "broadcast");
    }

    #[test]
    fn serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<LoudnormPreset>();
    }

    #[test]
    fn toml_accepts_every_cli_spelling() {
        assert_toml_accepts_every_from_str_spelling::<LoudnormPreset>(&[
            "broadcast",
            "streaming",
            "loud",
            "BROADCAST",
            "Loud",
        ]);
    }

    #[test]
    fn toml_rejects_unknown_spelling() {
        assert_toml_rejects_unknown_spelling::<LoudnormPreset>(
            "quiet",
            "unsupported loudnorm preset: quiet",
        );
    }

    /// The wire form is a contract shared with `config.toml`, the desktop's
    /// `settings.json` and the TypeScript `LoudnormPreset` union.
    #[test]
    fn serialize_emits_the_lowercase_wire_spelling() {
        assert_serialize_spellings(&[
            (LoudnormPreset::Broadcast, "broadcast"),
            (LoudnormPreset::Streaming, "streaming"),
            (LoudnormPreset::Loud, "loud"),
        ]);
    }
}
