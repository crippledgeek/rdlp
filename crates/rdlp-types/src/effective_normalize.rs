//! The materialised audio-normalization settings — one owner for their defaults.

use std::sync::LazyLock;

use serde::Serialize;

use crate::loudnorm_preset::LoudnormPreset;
use crate::postprocess::PostProcess;

/// An inclusive, finite bound on one normalization target, in the unit the
/// filter takes it.
///
/// One owner per field (the `*_RANGE` consts on [`EffectiveNormalize`]);
/// `Config::validate`, the desktop's `AppSettings::validate_security` and the
/// CLI's `value_parser`s all read these rather than restating a table. A
/// value that fails [`Self::contains`] would otherwise reach `FFmpeg`'s
/// filter graph unchecked (`volume=…dB`, `alimiter=limit=…`, `loudnorm=I=…`)
/// and be rejected there — or, for `NaN`/`inf`, silently formatted into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizeRange {
    /// Inclusive lower bound.
    pub min: f64,
    /// Inclusive upper bound.
    pub max: f64,
    /// Unit label for diagnostics (`"dBFS"`, `"LUFS"`, ...).
    pub unit: &'static str,
}

impl NormalizeRange {
    /// `true` when `value` is finite and within `min..=max`.
    ///
    /// Non-finite values are rejected explicitly: `NaN` fails every
    /// comparison and would slip through a naive `min <= v && v <= max`
    /// written as two negated checks, and `inf` formats into a filter
    /// string as `infdB`.
    #[must_use]
    pub fn contains(self, value: f64) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }

    /// The `OutOfRange` reason text, rendered from the bounds so it cannot
    /// drift from them. (Not `const`: `const_format` 0.2 cannot format
    /// floats, so the validators read the `*_REASON` statics below.)
    #[must_use]
    pub fn describe(self) -> String {
        format!(
            "must be a finite number in {}..={} {}",
            self.min, self.max, self.unit
        )
    }
}

/// The resolved normalization settings the post-process stage runs with.
///
/// The same three-layer shape as [`EffectiveNetwork`](crate::EffectiveNetwork):
///
/// 1. **Base** — [`PostProcess`], from `config.toml` or
///    [`PostProcess::default()`]. Its target fields are `Option<f64>` and
///    the preset is `Option<LoudnormPreset>`: `None` means "not set here".
/// 2. **Overlay** — a frontend's per-request or per-profile settings
///    (rdlp-api's `PostProcessOptions`, the desktop's `AppSettings`), where
///    `None` means *inherit* and is never materialised into a stored default
///    (#610).
/// 3. **Effective** — this struct. [`PostProcess::effective_normalize`]
///    collapses the `Option`s once: the preset to
///    [`LoudnormPreset::default()`], each unset I/TP/LRA to the *resolved*
///    preset's [`targets`](LoudnormPreset::targets), and the two
///    preset-independent values to the constants below.
///
/// [`PEAK_TARGET_DB`](Self::PEAK_TARGET_DB) and
/// [`BOOST_GAIN_DB`](Self::BOOST_GAIN_DB) are the *only* place those two
/// defaults live; the per-preset targets live on [`LoudnormPreset`]. Before
/// #611 each was restated in three or four places — the ffmpeg crate's
/// `NormalizeOptions::default`, the stage's `unwrap_or(..)`, the CLI help
/// text and the desktop's placeholders — and the desktop's I/TP/LRA
/// placeholders were Streaming-only, so they were wrong under any other
/// preset.
///
/// This is also the payload the desktop's settings placeholders read over
/// IPC (an `effective_normalize` command taking the draft's preset), so the
/// GUI holds no copy of any of these numbers either.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EffectiveNormalize {
    /// The preset in force — explicit, or [`LoudnormPreset::default()`].
    pub preset: LoudnormPreset,
    /// Integrated loudness target for `loudnorm`, in LUFS.
    pub target_i: f64,
    /// True-peak ceiling for `loudnorm`, in dBTP.
    pub target_tp: f64,
    /// Loudness range target for `loudnorm`, in LU.
    pub target_lra: f64,
    /// Peak-mode target level, in dBFS.
    pub peak_target_db: f64,
    /// Gain applied by the limiter-boost fallback, in dB.
    pub boost_gain_db: f64,
}

impl EffectiveNormalize {
    /// Default peak-mode target, in dBFS. An rdlp tuning choice (1 dB of
    /// headroom below full scale); not taken from a standard.
    pub const PEAK_TARGET_DB: f64 = -1.0;

    /// Default gain for the limiter-boost fallback, in dB. An rdlp tuning
    /// choice for over-compressed sources that linear loudnorm cannot lift;
    /// not taken from a standard.
    pub const BOOST_GAIN_DB: f64 = 12.0;

    /// Allowed peak-mode target (`audio_gain_target`), in dBFS.
    ///
    /// Derived from the limiter, not chosen: the peak target becomes
    /// `alimiter=limit=10^(target/20)`, and `libavfilter/af_alimiter.c`
    /// (`alimiter_options[]`, `FFmpeg` 8.0.1) clamps `limit` to
    /// `0.0625..=1` — i.e. `20·log10(0.0625) = −24.08 dBFS` up to `0 dBFS`.
    /// rdlp's loudnorm path additionally subtracts its 1.5 dB true-peak
    /// headroom (`ALIMITER_TP_HEADROOM_DB` in rdlp-ffmpeg) before building
    /// that same limiter, so the floor that holds for BOTH paths is
    /// `−24.08 + 1.5 = −22.58`, rounded inward to `−22.5`. Below it the
    /// filter graph fails to initialise with an `AVOption` range error.
    pub const PEAK_TARGET_DB_RANGE: NormalizeRange = NormalizeRange {
        min: -22.5,
        max: 0.0,
        unit: "dBFS",
    };

    /// Allowed integrated-loudness target (`loudnorm=I=`), in LUFS —
    /// `libavfilter/af_loudnorm.c` `loudnorm_options[]` (`FFmpeg` 8.0.1):
    /// `-70..=-5`.
    pub const TARGET_I_RANGE: NormalizeRange = NormalizeRange {
        min: -70.0,
        max: -5.0,
        unit: "LUFS",
    };

    /// Allowed true-peak target (`loudnorm=TP=`), in dBTP — same source:
    /// `-9..=0`. Tighter than the alimiter floor above, so it implies it.
    pub const TARGET_TP_RANGE: NormalizeRange = NormalizeRange {
        min: -9.0,
        max: 0.0,
        unit: "dBTP",
    };

    /// Allowed loudness-range target (`loudnorm=LRA=`), in LU — same
    /// source: `1..=50`.
    pub const TARGET_LRA_RANGE: NormalizeRange = NormalizeRange {
        min: 1.0,
        max: 50.0,
        unit: "LU",
    };

    /// Allowed limiter-boost gain (`normalize_boost_db`), in dB. An rdlp
    /// choice: the `volume` filter accepts any gain, but a boost is only
    /// meaningful as a positive lift, and 30 dB (≈31.6×) is already far past
    /// anything a hard limiter can make listenable. No standard applies.
    pub const BOOST_GAIN_DB_RANGE: NormalizeRange = NormalizeRange {
        min: 0.0,
        max: 30.0,
        unit: "dB",
    };
}

/// `OutOfRange` reason for [`EffectiveNormalize::PEAK_TARGET_DB_RANGE`],
/// rendered once from the bounds (a `&'static str` for the error variants).
pub static PEAK_TARGET_DB_REASON: LazyLock<String> =
    LazyLock::new(|| EffectiveNormalize::PEAK_TARGET_DB_RANGE.describe());
/// `OutOfRange` reason for [`EffectiveNormalize::TARGET_I_RANGE`].
pub static TARGET_I_REASON: LazyLock<String> =
    LazyLock::new(|| EffectiveNormalize::TARGET_I_RANGE.describe());
/// `OutOfRange` reason for [`EffectiveNormalize::TARGET_TP_RANGE`].
pub static TARGET_TP_REASON: LazyLock<String> =
    LazyLock::new(|| EffectiveNormalize::TARGET_TP_RANGE.describe());
/// `OutOfRange` reason for [`EffectiveNormalize::TARGET_LRA_RANGE`].
pub static TARGET_LRA_REASON: LazyLock<String> =
    LazyLock::new(|| EffectiveNormalize::TARGET_LRA_RANGE.describe());
/// `OutOfRange` reason for [`EffectiveNormalize::BOOST_GAIN_DB_RANGE`].
pub static BOOST_GAIN_DB_REASON: LazyLock<String> =
    LazyLock::new(|| EffectiveNormalize::BOOST_GAIN_DB_RANGE.describe());

/// The five bounded `PostProcess` targets: `(field name, range, reason)`.
///
/// The single table both validators iterate, so neither can omit a field the
/// other checks. `field` is the `PostProcess`/`AppSettings` identifier.
#[must_use]
pub fn normalize_target_bounds() -> [(&'static str, NormalizeRange, &'static str); 5] {
    [
        (
            "audio_gain_target",
            EffectiveNormalize::PEAK_TARGET_DB_RANGE,
            PEAK_TARGET_DB_REASON.as_str(),
        ),
        (
            "loudnorm_target_i",
            EffectiveNormalize::TARGET_I_RANGE,
            TARGET_I_REASON.as_str(),
        ),
        (
            "loudnorm_target_tp",
            EffectiveNormalize::TARGET_TP_RANGE,
            TARGET_TP_REASON.as_str(),
        ),
        (
            "loudnorm_target_lra",
            EffectiveNormalize::TARGET_LRA_RANGE,
            TARGET_LRA_REASON.as_str(),
        ),
        (
            "normalize_boost_db",
            EffectiveNormalize::BOOST_GAIN_DB_RANGE,
            BOOST_GAIN_DB_REASON.as_str(),
        ),
    ]
}

impl PostProcess {
    /// The first normalization target outside its owning range, as
    /// `(field, reason)` — `None` when all five are unset or in range.
    ///
    /// The ONE range check both `Config::validate` and the desktop's
    /// `AppSettings::validate_security` call, so the bounds and the field
    /// names cannot drift between them. Checked in the order of
    /// [`normalize_target_bounds`].
    #[must_use]
    pub fn first_target_out_of_range(&self) -> Option<(&'static str, &'static str)> {
        let values = [
            self.audio_gain_target,
            self.loudnorm_target_i,
            self.loudnorm_target_tp,
            self.loudnorm_target_lra,
            self.normalize_boost_db,
        ];
        normalize_target_bounds()
            .into_iter()
            .zip(values)
            .find(|((_, range, _), value)| value.is_some_and(|v| !range.contains(v)))
            .map(|((field, _, reason), _)| (field, reason))
    }
}

impl PostProcess {
    /// Materialise the normalization settings the post-process stage reads.
    ///
    /// The single resolution step for the six values: the preset collapses to
    /// [`LoudnormPreset::default()`], each unset I/TP/LRA to the resolved
    /// preset's [`targets`](LoudnormPreset::targets), and the peak target and
    /// boost gain to [`EffectiveNormalize::PEAK_TARGET_DB`] and
    /// [`EffectiveNormalize::BOOST_GAIN_DB`]. Consumers read from here rather
    /// than each carrying an `unwrap_or(<literal>)` (#611).
    #[must_use]
    pub fn effective_normalize(&self) -> EffectiveNormalize {
        let preset = self.loudnorm_preset.unwrap_or_default();
        let targets = preset.targets();
        EffectiveNormalize {
            preset,
            target_i: self.loudnorm_target_i.unwrap_or(targets.integrated_lufs),
            target_tp: self.loudnorm_target_tp.unwrap_or(targets.true_peak_dbtp),
            target_lra: self.loudnorm_target_lra.unwrap_or(targets.range_lu),
            peak_target_db: self
                .audio_gain_target
                .unwrap_or(EffectiveNormalize::PEAK_TARGET_DB),
            boost_gain_db: self
                .normalize_boost_db
                .unwrap_or(EffectiveNormalize::BOOST_GAIN_DB),
        }
    }
}

#[cfg(test)]
// float_cmp: constants compared against themselves after propagation — exact
// equality is the oracle, an epsilon would accept a drifted value.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    /// Pins the documented values. The one place these two literals may
    /// appear as a test oracle.
    #[test]
    fn constants_are_the_documented_ones() {
        assert_eq!(EffectiveNormalize::PEAK_TARGET_DB, -1.0);
        assert_eq!(EffectiveNormalize::BOOST_GAIN_DB, 12.0);
    }

    /// Pins the derived bounds — the one place these literals may appear as a
    /// test oracle. The peak floor is the alimiter's `0.0625` limit in dB
    /// plus rdlp's 1.5 dB headroom, rounded inward.
    #[test]
    fn ranges_are_the_documented_ones() {
        let alimiter_floor_db = 20.0 * 0.0625_f64.log10();
        assert!((alimiter_floor_db - (-24.08)).abs() < 0.01);
        let derived_floor = alimiter_floor_db + 1.5;
        assert!(
            EffectiveNormalize::PEAK_TARGET_DB_RANGE.min >= derived_floor,
            "the peak floor must not go below the alimiter's own: {derived_floor}"
        );
        assert_eq!(EffectiveNormalize::PEAK_TARGET_DB_RANGE.min, -22.5);
        assert_eq!(EffectiveNormalize::PEAK_TARGET_DB_RANGE.max, 0.0);
        assert_eq!(
            (
                EffectiveNormalize::TARGET_I_RANGE.min,
                EffectiveNormalize::TARGET_I_RANGE.max
            ),
            (-70.0, -5.0)
        );
        assert_eq!(
            (
                EffectiveNormalize::TARGET_TP_RANGE.min,
                EffectiveNormalize::TARGET_TP_RANGE.max
            ),
            (-9.0, 0.0)
        );
        assert_eq!(
            (
                EffectiveNormalize::TARGET_LRA_RANGE.min,
                EffectiveNormalize::TARGET_LRA_RANGE.max
            ),
            (1.0, 50.0)
        );
        assert_eq!(
            (
                EffectiveNormalize::BOOST_GAIN_DB_RANGE.min,
                EffectiveNormalize::BOOST_GAIN_DB_RANGE.max
            ),
            (0.0, 30.0)
        );
        // The defaults sit inside their own ranges.
        assert!(
            EffectiveNormalize::PEAK_TARGET_DB_RANGE.contains(EffectiveNormalize::PEAK_TARGET_DB)
        );
        assert!(
            EffectiveNormalize::BOOST_GAIN_DB_RANGE.contains(EffectiveNormalize::BOOST_GAIN_DB)
        );
        for preset in LoudnormPreset::ALL {
            let t = preset.targets();
            assert!(EffectiveNormalize::TARGET_I_RANGE.contains(t.integrated_lufs));
            assert!(EffectiveNormalize::TARGET_TP_RANGE.contains(t.true_peak_dbtp));
            assert!(EffectiveNormalize::TARGET_LRA_RANGE.contains(t.range_lu));
        }
    }

    #[test]
    fn range_contains_is_inclusive_and_rejects_non_finite() {
        let r = NormalizeRange {
            min: -9.0,
            max: 0.0,
            unit: "dBTP",
        };
        assert!(r.contains(-9.0));
        assert!(r.contains(0.0));
        assert!(!r.contains(-9.001));
        assert!(!r.contains(0.001));
        assert!(!r.contains(f64::NAN));
        assert!(!r.contains(f64::INFINITY));
        assert!(!r.contains(f64::NEG_INFINITY));
        assert_eq!(r.describe(), "must be a finite number in -9..=0 dBTP");
    }

    #[test]
    fn first_target_out_of_range_names_the_field_and_its_reason() {
        assert_eq!(PostProcess::default().first_target_out_of_range(), None);
        let bad = PostProcess {
            loudnorm_target_lra: Some(0.5),
            ..PostProcess::default()
        };
        assert_eq!(
            bad.first_target_out_of_range(),
            Some(("loudnorm_target_lra", TARGET_LRA_REASON.as_str()))
        );
    }

    /// The desktop reads this over IPC; the wire shape is the field names as
    /// written, with the preset in its lowercase wire spelling.
    #[test]
    fn serializes_field_names_verbatim() {
        let eff = PostProcess {
            loudnorm_preset: Some(LoudnormPreset::Loud),
            ..PostProcess::default()
        }
        .effective_normalize();
        let json = serde_json::to_value(eff).expect("serialize");
        let fields = json.as_object().expect("a JSON object");
        assert_eq!(fields.get("preset"), Some(&"loud".into()));
        assert_eq!(fields.get("target_i"), Some(&(-11.0).into()));
        assert_eq!(
            fields.get("peak_target_db"),
            Some(&EffectiveNormalize::PEAK_TARGET_DB.into())
        );
        assert_eq!(
            fields.get("boost_gain_db"),
            Some(&EffectiveNormalize::BOOST_GAIN_DB.into())
        );
        assert_eq!(fields.len(), 6, "exactly six fields on the wire");
    }
}
