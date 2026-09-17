//! Audio normalization types.
//!
//! Provides `AudioNormMode`, `NormalizeOptions`, `PeakAnalysis`, and
//! `LoudnormMeasurements` used by the normalization pipeline.
//! [`LoudnormPreset`] lives in `rdlp-types`, the single owner of the
//! per-preset targets (#611).

use rdlp_types::{EffectiveNormalize, LoudnormPreset, LoudnormTargets};

/// Audio normalization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioNormMode {
    /// Peak/gain normalization: analyze peak/RMS via astats, apply volume + alimiter.
    Peak,
    /// EBU R128 two-pass loudness normalization via loudnorm filter.
    Loudnorm,
}

/// Options for audio normalization.
///
/// # Lint allowances
///
/// - `clippy::struct_excessive_bools`: the four boolean fields (`salvage`,
///   `force_dynamic`, `precompress`, `boost_enabled`) are independent feature
///   flags, each with a distinct effect. A bitflags/enum refactor would reduce
///   expressiveness without eliminating the need for per-flag documentation.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizeOptions {
    /// Normalization mode (peak or loudnorm).
    pub mode: AudioNormMode,
    /// Target peak level in dBFS (Mode A). Defaults to
    /// [`EffectiveNormalize::PEAK_TARGET_DB`].
    pub target_peak_db: f64,
    /// Target integrated loudness in LUFS (Mode B). Defaults to the
    /// [`LoudnormPreset::default()`] target.
    pub target_i: f64,
    /// Target true peak in dBTP (Mode B). Defaults to the
    /// [`LoudnormPreset::default()`] target.
    pub target_tp: f64,
    /// Target loudness range in LU (Mode B). Defaults to the
    /// [`LoudnormPreset::default()`] target.
    pub target_lra: f64,
    /// Automatically salvage corrupt Matroska/WebM containers before processing.
    ///
    /// When enabled (default), corrupt inputs are detected via EBML log analysis
    /// and automatically remuxed to a clean temporary file before normalization.
    /// Disable for strict mode where corruption should be a hard error.
    pub salvage: bool,
    /// Force dynamic (per-frame compression) mode in loudnorm pass 2.
    ///
    /// By default, loudnorm uses `linear=true` (letting `FFmpeg` fall back to
    /// dynamic internally if needed). This flag forces `linear=false` for users
    /// who explicitly want dynamic compression. Corresponds to `--loudnorm-dynamic`.
    pub force_dynamic: bool,
    /// Prepend a mild acompressor before loudnorm in pass 2.
    ///
    /// Tames extreme peaks before loudnorm, allowing linear mode to apply more
    /// gain without hitting the TP ceiling. Uses a conservative preset:
    /// `threshold=-18dB, ratio=3:1, attack=20ms, release=200ms, makeup=2dB, knee=6dB`.
    /// Corresponds to `--loudnorm-precompress`.
    pub precompress: bool,
    /// Enable limiter-boost fallback for over-compressed content.
    ///
    /// When enabled and loudnorm pass 1 shows shortfall > 6 LU, skips
    /// loudnorm pass 2 and applies a fixed gain with hard limiter instead.
    /// Corresponds to `--normalize-boost`.
    pub boost_enabled: bool,
    /// Gain in dB for limiter-boost fallback. Defaults to
    /// [`EffectiveNormalize::BOOST_GAIN_DB`].
    ///
    /// Only used when `boost_enabled` is true and shortfall exceeds threshold.
    /// Corresponds to `--normalize-boost-db`.
    pub boost_gain_db: f64,
}

impl Default for NormalizeOptions {
    /// Every number comes from its owner in `rdlp-types` (#611): the preset
    /// targets from [`LoudnormPreset::default()`], the peak target and boost
    /// gain from [`EffectiveNormalize`]. No literal is restated here.
    fn default() -> Self {
        let LoudnormTargets {
            integrated_lufs,
            true_peak_dbtp,
            range_lu,
        } = LoudnormPreset::default().targets();
        Self {
            mode: AudioNormMode::Peak,
            target_peak_db: EffectiveNormalize::PEAK_TARGET_DB,
            target_i: integrated_lufs,
            target_tp: true_peak_dbtp,
            target_lra: range_lu,
            salvage: true,
            force_dynamic: false,
            precompress: false,
            boost_enabled: false,
            boost_gain_db: EffectiveNormalize::BOOST_GAIN_DB,
        }
    }
}

/// Results from peak/RMS audio analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct PeakAnalysis {
    /// Peak level in dBFS.
    pub peak_db: f64,
    /// RMS level in dBFS.
    pub rms_db: f64,
    /// Computed gain adjustment in dB.
    pub gain_db: f64,
}

/// Measurements from EBU R128 loudnorm first pass.
#[derive(Debug, Clone, PartialEq)]
pub struct LoudnormMeasurements {
    /// Measured integrated loudness (LUFS).
    pub input_i: f64,
    /// Measured true peak (dBTP).
    pub input_tp: f64,
    /// Measured loudness range (LU).
    pub input_lra: f64,
    /// Measured loudness threshold (LUFS).
    pub input_thresh: f64,
    /// Target offset (LU).
    pub target_offset: f64,
}

impl LoudnormMeasurements {
    /// Predict the gain (dB) that linear mode would apply.
    ///
    /// Linear mode applies a constant gain capped by the true-peak headroom:
    /// `min(target_i - measured_i, target_tp - measured_tp)`.
    #[must_use]
    pub fn predict_linear_gain(&self, target_i: f64, target_tp: f64) -> f64 {
        let desired = target_i - self.input_i;
        let tp_headroom = target_tp - self.input_tp;
        desired.min(tp_headroom)
    }

    /// Compute the shortfall (LU) when using linear mode.
    ///
    /// Returns `target_i - (measured_i + predicted_linear_gain)`.
    /// A value <= 0 means linear mode fully reaches the target.
    #[must_use]
    pub fn linear_shortfall(&self, target_i: f64, target_tp: f64) -> f64 {
        let gain = self.predict_linear_gain(target_i, target_tp);
        target_i - (self.input_i + gain)
    }

    /// Returns `true` if linear mode can reach the target within 0.5 LU.
    #[must_use]
    pub fn linear_sufficient(&self, target_i: f64, target_tp: f64) -> bool {
        self.linear_shortfall(target_i, target_tp) <= 0.5
    }
}
