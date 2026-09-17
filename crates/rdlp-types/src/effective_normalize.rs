//! The materialised audio-normalization settings — one owner for their defaults.

use serde::Serialize;

use crate::loudnorm_preset::LoudnormPreset;
use crate::postprocess::PostProcess;

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
