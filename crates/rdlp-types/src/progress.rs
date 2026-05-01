//! Normalized progress value (`0.0..=1.0`).
//!
//! [`Progress`] unifies the previously-divergent progress scales used across
//! the workspace (`DownloadProgress.percentage` was `0.0..=100.0`,
//! `Event::PostProcessProgress.progress` was `0.0..=1.0`). All progress is
//! now carried as a clamped `f32` fraction; UI surfaces call [`Progress::percent`]
//! for display.

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// A clamped progress fraction in `[0.0, 1.0]`.
///
/// Construct via [`Progress::new`] (clamps and treats NaN as `0.0`) or
/// [`Progress::try_new`] (rejects NaN / out-of-range). The newtype prevents
/// accidental scale mixing — the fraction can only be read via [`Progress::fraction`]
/// (`0.0..=1.0`) or [`Progress::percent`] (`0.0..=100.0`).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Progress(f32);

impl Progress {
    /// `0.0` (no progress).
    pub const ZERO: Self = Self(0.0);

    /// `1.0` (complete).
    pub const COMPLETE: Self = Self(1.0);

    /// Construct from a fraction, clamping out-of-range values into `[0.0, 1.0]`.
    ///
    /// NaN is mapped to `0.0` so callers always get a usable value.
    #[must_use]
    pub const fn new(fraction: f32) -> Self {
        if fraction.is_nan() {
            Self(0.0)
        } else {
            Self(fraction.clamp(0.0, 1.0))
        }
    }

    /// Construct from a fraction, returning `None` for NaN or out-of-range input.
    ///
    /// Use this when an out-of-range value indicates a producer bug worth
    /// surfacing rather than silently clamping.
    #[must_use]
    pub fn try_new(fraction: f32) -> Option<Self> {
        if fraction.is_nan() || !(0.0..=1.0).contains(&fraction) {
            None
        } else {
            Some(Self(fraction))
        }
    }

    /// Construct from a `f64` fraction with the same clamping semantics as [`Progress::new`].
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub const fn from_f64(fraction: f64) -> Self {
        Self::new(fraction as f32)
    }

    /// Construct from a numerator / denominator pair.
    ///
    /// Returns [`Progress::ZERO`] if `total == 0`.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_ratio(done: u64, total: u64) -> Self {
        if total == 0 {
            Self::ZERO
        } else {
            Self::new(done as f32 / total as f32)
        }
    }

    /// The underlying fraction in `[0.0, 1.0]`.
    #[must_use]
    pub const fn fraction(self) -> f32 {
        self.0
    }

    /// The fraction as a percentage in `[0.0, 100.0]`. Use this at display sites.
    #[must_use]
    pub fn percent(self) -> f32 {
        self.0 * 100.0
    }

    /// `true` when the fraction has reached `1.0`.
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.0 >= 1.0
    }
}

impl fmt::Display for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}%", self.percent())
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<'de> Deserialize<'de> for Progress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fraction = f32::deserialize(deserializer)?;
        Ok(Self::new(fraction))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    clippy::missing_docs_in_private_items,
    clippy::float_cmp
)]
mod tests {
    use super::*;

    #[test]
    fn new_clamps_into_unit_interval() {
        assert_eq!(Progress::new(-0.5).fraction(), 0.0);
        assert_eq!(Progress::new(0.0).fraction(), 0.0);
        assert_eq!(Progress::new(0.42).fraction(), 0.42);
        assert_eq!(Progress::new(1.0).fraction(), 1.0);
        assert_eq!(Progress::new(2.5).fraction(), 1.0);
    }

    #[test]
    fn new_treats_nan_as_zero() {
        assert_eq!(Progress::new(f32::NAN).fraction(), 0.0);
    }

    #[test]
    fn try_new_rejects_out_of_range() {
        assert!(Progress::try_new(-0.1).is_none());
        assert!(Progress::try_new(1.1).is_none());
        assert!(Progress::try_new(f32::NAN).is_none());
        assert_eq!(Progress::try_new(0.5).unwrap().fraction(), 0.5);
    }

    #[test]
    fn percent_scales_to_hundred() {
        assert_eq!(Progress::new(0.0).percent(), 0.0);
        assert_eq!(Progress::new(0.5).percent(), 50.0);
        assert_eq!(Progress::new(1.0).percent(), 100.0);
    }

    #[test]
    fn from_ratio_handles_zero_total() {
        assert_eq!(Progress::from_ratio(0, 0), Progress::ZERO);
        assert_eq!(Progress::from_ratio(50, 100).fraction(), 0.5);
    }

    #[test]
    fn is_complete_at_one() {
        assert!(!Progress::new(0.99).is_complete());
        assert!(Progress::COMPLETE.is_complete());
    }

    #[test]
    fn display_prints_percent() {
        assert_eq!(format!("{}", Progress::new(0.5)), "50.0%");
        assert_eq!(format!("{}", Progress::new(0.123)), "12.3%");
    }

    #[test]
    fn serializes_as_fraction() {
        let json = serde_json::to_string(&Progress::new(0.5)).unwrap();
        assert_eq!(json, "0.5");
    }

    #[test]
    fn deserializes_clamps_out_of_range() {
        let p: Progress = serde_json::from_str("0.5").unwrap();
        assert_eq!(p.fraction(), 0.5);
        // Out-of-range deserialises by clamping (legacy data tolerance).
        let p: Progress = serde_json::from_str("1.5").unwrap();
        assert_eq!(p.fraction(), 1.0);
    }
}
