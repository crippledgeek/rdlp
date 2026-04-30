//! Audio normalization types and presets.
//!
//! Provides `AudioNormMode`, `LoudnormPreset`, `NormalizeOptions`,
//! `PeakAnalysis`, and `LoudnormMeasurements` used by the normalization
//! pipeline.

/// Audio normalization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioNormMode {
    /// Peak/gain normalization: analyze peak/RMS via astats, apply volume + alimiter.
    Peak,
    /// EBU R128 two-pass loudness normalization via loudnorm filter.
    Loudnorm,
}

/// Loudnorm target presets for common use cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoudnormPreset {
    /// Broadcast standard: I=-23 LUFS, TP=-2 dBTP, LRA=7 LU
    Broadcast,
    /// Streaming standard: I=-14 LUFS, TP=-1 dBTP, LRA=11 LU
    Streaming,
    /// Loud master: I=-11 LUFS, TP=-1 dBTP, LRA=11 LU
    Loud,
}

impl LoudnormPreset {
    /// Returns `(target_i, target_tp, target_lra)` for this preset.
    #[must_use]
    pub const fn targets(self) -> (f64, f64, f64) {
        match self {
            Self::Broadcast => (-23.0, -2.0, 7.0),
            Self::Streaming => (-14.0, -1.0, 11.0),
            Self::Loud => (-11.0, -1.0, 11.0),
        }
    }
}

impl std::str::FromStr for LoudnormPreset {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("broadcast") {
            Ok(Self::Broadcast)
        } else if s.eq_ignore_ascii_case("streaming") {
            Ok(Self::Streaming)
        } else if s.eq_ignore_ascii_case("loud") {
            Ok(Self::Loud)
        } else {
            Err(format!(
                "unknown loudnorm preset '{s}': expected broadcast, streaming, or loud"
            ))
        }
    }
}

impl std::fmt::Display for LoudnormPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broadcast => write!(f, "broadcast"),
            Self::Streaming => write!(f, "streaming"),
            Self::Loud => write!(f, "loud"),
        }
    }
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
    /// Target peak level in dBFS (Mode A, default -1.0).
    pub target_peak_db: f64,
    /// Target integrated loudness in LUFS (Mode B, default -16.0).
    pub target_i: f64,
    /// Target true peak in dBTP (Mode B, default -1.5).
    pub target_tp: f64,
    /// Target loudness range in LU (Mode B, default 11.0).
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
    /// Gain in dB for limiter-boost fallback (default 12.0).
    ///
    /// Only used when `boost_enabled` is true and shortfall exceeds threshold.
    /// Corresponds to `--normalize-boost-db`.
    pub boost_gain_db: f64,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        let (i, tp, lra) = LoudnormPreset::Streaming.targets();
        Self {
            mode: AudioNormMode::Peak,
            target_peak_db: -1.0,
            target_i: i,
            target_tp: tp,
            target_lra: lra,
            salvage: true,
            force_dynamic: false,
            precompress: false,
            boost_enabled: false,
            boost_gain_db: 12.0,
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
