//! Smoothed ETA estimation for post-processing stages.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Fractions below this are too noisy to derive a stable ETA from (the first
/// frames of a slow encoder, or a lookahead-fill phase). Below it, return None.
const ETA_WARMUP_FRAC: f64 = 0.005; // 0.5%
/// EWMA smoothing factor for the ETA (mirrors `adaptive::EWMA_ALPHA = 0.3`).
const ETA_EWMA_ALPHA: f64 = 0.3;

/// Average-rate ETA: `elapsed * (1 - frac) / frac`. `None` when `frac` is
/// non-positive or already complete (`>= 1.0`). Uses `try_from_secs_f64`
/// (stable since 1.66; workspace floor is 1.85) so a non-finite OR finite-but-
/// overflowing result yields `None` instead of panicking — never panics /
/// divides by zero. (Validated 2026-06-07: `from_secs_f64` panics on
/// negative/NaN/inf AND finite overflow; the checked variant covers all four.)
#[must_use]
pub fn estimate_eta(elapsed: Duration, frac: f64) -> Option<Duration> {
    if frac <= 0.0 || frac >= 1.0 {
        return None;
    }
    Duration::try_from_secs_f64(elapsed.as_secs_f64() * (1.0 - frac) / frac).ok()
}

/// EWMA convex combination: `alpha * raw + (1 - alpha) * prev`. The result
/// always lies between `prev` and `raw` for `alpha` in `[0, 1]`.
fn smooth(prev: f64, raw: f64, alpha: f64) -> f64 {
    alpha.mul_add(raw, (1.0 - alpha) * prev)
}

/// Per-stage smoothed ETA estimator. `update` takes `&self` (the progress
/// callback is `&self`), so state lives behind a `Mutex`. The clock starts
/// lazily on the first `update`, so setup time before the first progress tick
/// does not inflate the estimate.
pub struct EtaEstimator {
    inner: Mutex<EtaState>,
}

#[derive(Default)]
struct EtaState {
    start: Option<Instant>,
    smoothed_secs: Option<f64>,
}

impl EtaEstimator {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(EtaState::default()),
        }
    }

    /// Update with the latest progress fraction; return a smoothed ETA, or
    /// `None` during warm-up / when not computable.
    pub(crate) fn update(&self, frac: f64) -> Option<Duration> {
        let now = Instant::now();
        let mut s = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let start = *s.start.get_or_insert(now);
        if frac < ETA_WARMUP_FRAC {
            return None;
        }
        let raw_secs = estimate_eta(now.duration_since(start), frac)?.as_secs_f64();
        let smoothed = s
            .smoothed_secs
            .map_or(raw_secs, |prev| smooth(prev, raw_secs, ETA_EWMA_ALPHA));
        s.smoothed_secs = Some(smoothed);
        drop(s); // release the lock before the (cheap) f64→Duration conversion
        Duration::try_from_secs_f64(smoothed).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn estimate_eta_uses_average_rate_formula() {
        // elapsed * (1 - frac) / frac
        assert_eq!(
            estimate_eta(Duration::from_secs(10), 0.5),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            estimate_eta(Duration::from_secs(10), 0.25),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn estimate_eta_none_on_nonpositive_or_complete_frac() {
        assert_eq!(estimate_eta(Duration::from_secs(10), 0.0), None);
        assert_eq!(estimate_eta(Duration::from_secs(10), -0.5), None);
        assert_eq!(estimate_eta(Duration::from_secs(10), 1.0), None);
        assert_eq!(estimate_eta(Duration::from_secs(10), 1.5), None);
    }

    #[test]
    fn estimator_warmup_returns_none_below_threshold_then_some() {
        let est = EtaEstimator::new();
        // Below warm-up fraction → None (early ratios too noisy).
        assert!(est.update(0.0).is_none());
        assert!(est.update(0.001).is_none());
        // Above warm-up → Some (a real, bounded estimate).
        let eta = est.update(0.5);
        assert!(eta.is_some(), "past warm-up the estimator yields an ETA");
    }

    #[test]
    fn estimator_smoothing_returns_finite_nonnegative() {
        // The exact raw estimates depend on Instant timing (non-deterministic),
        // so this only guards that smoothing yields a finite, non-negative
        // duration and never panics; the convex-combination bound is checked
        // deterministically by `smooth_is_a_convex_combination`.
        let est = EtaEstimator::new();
        let _ = est.update(0.5);
        let second = est.update(0.6);
        assert!(second.is_some());
        // Bounded + finite (no NaN/inf leaking through).
        let secs = second
            .unwrap_or_else(|| Duration::from_secs(0))
            .as_secs_f64();
        assert!(secs.is_finite() && secs >= 0.0);
    }

    #[test]
    fn smooth_is_a_convex_combination() {
        // 0.3*20 + 0.7*10 = 13.0 — strictly between prev and raw.
        assert!((smooth(10.0, 20.0, 0.3) - 13.0).abs() < 1e-9);
        // Bounded by the two inputs regardless of order.
        let s = smooth(100.0, 5.0, 0.3);
        assert!(
            (5.0..=100.0).contains(&s),
            "smoothed must lie within [raw, prev]"
        );
        // alpha = 0 → keeps prev; alpha = 1 → takes raw.
        assert!((smooth(10.0, 20.0, 0.0) - 10.0).abs() < 1e-9);
        assert!((smooth(10.0, 20.0, 1.0) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn estimator_warmup_threshold_is_inclusive() {
        // Guard is `frac < ETA_WARMUP_FRAC`, so frac == 0.005 is NOT gated.
        let est = EtaEstimator::new();
        assert!(
            est.update(0.005).is_some(),
            "warm-up threshold is inclusive"
        );
    }

    #[test]
    fn estimator_completion_tick_past_warmup_is_none() {
        // frac >= 1.0 (a completion tick) past warm-up → estimate_eta None → update None.
        let est = EtaEstimator::new();
        let _ = est.update(0.5);
        assert!(
            est.update(1.0).is_none(),
            "nothing remaining at frac == 1.0"
        );
    }
}
