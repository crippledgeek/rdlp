//! Shared progress reporting infrastructure for downloaders.
//!
//! This module provides a unified progress reporter that works for both
//! HTTP (byte-based) and HLS (duration-based) downloads, eliminating
//! duplicate progress tracking code.
//!
//! # Lint allowances
//!
//! - `clippy::redundant_clone`: `Arc` clones inside closures are required
//!   for multi-producer progress callbacks.
//! - `clippy::needless_pass_by_value`: `new()` and related constructors use
//!   by-value parameters matching the builder pattern.
//! - `clippy::option_if_let_else`: the `if let (Some(…))` pattern is clearer
//!   than `map_or` for the HLS/HTTP progress branch.

#![allow(
    clippy::redundant_clone,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else
)]

use rdlp_core::{DownloadProgress, ProgressCallback};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Sliding-window duration for the raw rate.
/// Source: yt-dlp `ProgressCalculator.SAMPLING_WINDOW` (`yt_dlp/utils/progress.py`).
const SPEED_WINDOW_SECS: f64 = 3.0;
/// Minimum seconds between accepted samples; debounces bursty parallel-yield
/// observations so one fragment is never divided by a microsecond gap.
/// Source: yt-dlp `ProgressCalculator.SAMPLING_RATE`.
const SPEED_SAMPLE_GATE_SECS: f64 = 0.05;
/// EWMA weight on the newest raw window rate; the prior smoothed value keeps
/// `1 - weight`. Source: yt-dlp `SmoothValue(smoothing=0.7)` -> `1 - 0.7 = 0.3`.
const SPEED_EWMA_NEW_WEIGHT: f64 = 0.3;

/// Sliding-window + EWMA download-rate meter (yt-dlp hybrid model).
///
/// Fed `(cumulative_bytes, now)` on a cadence (timer tick or per-fragment), it
/// reports a smoothed bytes/sec readout immune to bursty parallel fragment
/// completion: the rate is always `Δcumulative / Δwall-clock` over a multi-second
/// window, never one fragment's size over the inter-yield gap. Returns `None`
/// while cold-starting (`< 2` samples) or stalled (window emptied).
#[derive(Debug, Default)]
pub(crate) struct SpeedMeter {
    window: std::collections::VecDeque<(Instant, u64)>,
    smoothed: Option<f64>,
    last_sample: Option<Instant>,
}

impl SpeedMeter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record the cumulative byte total observed at `now`. `now` is injected
    /// (no `Instant::now()` here) so callers control the clock and tests are
    /// deterministic.
    pub(crate) fn update(&mut self, cumulative_bytes: u64, now: Instant) {
        if let Some(last) = self.last_sample
            && now.duration_since(last).as_secs_f64() < SPEED_SAMPLE_GATE_SECS
        {
            return;
        }
        self.last_sample = Some(now);
        self.window.push_back((now, cumulative_bytes));

        let window = Duration::from_secs_f64(SPEED_WINDOW_SECS);
        while let Some(&(t, _)) = self.window.front() {
            if now.duration_since(t) > window {
                self.window.pop_front();
            } else {
                break;
            }
        }

        if self.window.len() < 2 {
            self.smoothed = None;
            return;
        }
        let Some(&(t0, b0)) = self.window.front() else {
            return;
        };
        let Some(&(_, b_last)) = self.window.back() else {
            return;
        };
        let elapsed = now.duration_since(t0).as_secs_f64();
        if elapsed <= 0.0 {
            return;
        }
        // Numerator and denominator both come from the window so the rate stays
        // consistent regardless of how `update` is reordered in future.
        #[allow(clippy::cast_precision_loss)]
        let raw = b_last.saturating_sub(b0) as f64 / elapsed;
        self.smoothed = Some(match self.smoothed {
            None => raw,
            Some(prev) => SPEED_EWMA_NEW_WEIGHT.mul_add(raw, (1.0 - SPEED_EWMA_NEW_WEIGHT) * prev),
        });
    }

    /// Smoothed bytes/sec, or `None` when cold-starting or stalled.
    /// `const fn` is required by the crate's `clippy::missing_const_for_fn` lint.
    pub(crate) const fn bytes_per_sec(&self) -> Option<f64> {
        self.smoothed
    }
}

/// RAII guard for progress reporter tasks.
///
/// Ensures the progress reporter task is aborted when the guard goes out of scope,
/// preventing task leaks on early returns or errors.
pub struct ProgressGuard(Option<tokio::task::JoinHandle<()>>);

impl ProgressGuard {
    /// Create a new progress guard wrapping an optional task handle.
    #[must_use]
    pub const fn new(task: Option<tokio::task::JoinHandle<()>>) -> Self {
        Self(task)
    }

    /// Abort the task explicitly (also happens on drop).
    pub fn abort(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Progress metrics tracked atomically across parallel downloads.
#[derive(Clone)]
pub struct ProgressMetrics {
    /// Bytes downloaded so far
    pub downloaded: Arc<AtomicU64>,
    /// Segments completed (HLS only)
    pub segments_completed: Option<Arc<AtomicU64>>,
    /// Duration completed in centiseconds (HLS only, for f64 precision)
    pub duration_completed: Option<Arc<AtomicU64>>,
}

impl ProgressMetrics {
    /// Create metrics for byte-based progress (HTTP downloads).
    pub const fn bytes_only(downloaded: Arc<AtomicU64>) -> Self {
        Self {
            downloaded,
            segments_completed: None,
            duration_completed: None,
        }
    }

    /// Create metrics for duration-based progress (HLS downloads).
    pub const fn with_duration(
        downloaded: Arc<AtomicU64>,
        segments_completed: Arc<AtomicU64>,
        duration_completed: Arc<AtomicU64>,
    ) -> Self {
        Self {
            downloaded,
            segments_completed: Some(segments_completed),
            duration_completed: Some(duration_completed),
        }
    }
}

/// Configuration for the progress reporter.
pub struct ProgressReporterConfig {
    /// Start time for speed calculation
    pub start_time: Instant,
    /// Bytes already downloaded (for resume)
    pub resume_from: u64,
    /// Total expected size in bytes (None for unknown/streaming)
    pub total_size: Option<u64>,
    /// Total segments (HLS only)
    pub total_segments: Option<u64>,
    /// Total duration in seconds (HLS only)
    pub total_duration: Option<f64>,
    /// Update interval (default: 100ms)
    pub update_interval: Duration,
}

impl ProgressReporterConfig {
    /// Create config for HTTP byte-based progress.
    #[must_use]
    pub const fn http(start_time: Instant, total_size: u64, resume_from: u64) -> Self {
        Self {
            start_time,
            resume_from,
            total_size: Some(total_size),
            total_segments: None,
            total_duration: None,
            update_interval: Duration::from_millis(100),
        }
    }

    /// Create config for HLS duration-based progress.
    #[must_use]
    pub const fn hls(start_time: Instant, total_segments: u64, total_duration: f64) -> Self {
        Self {
            start_time,
            resume_from: 0,
            total_size: None,
            total_segments: Some(total_segments),
            total_duration: Some(total_duration),
            update_interval: Duration::from_millis(100),
        }
    }
}

/// Spawn a progress reporter task that periodically reports download progress.
///
/// Returns a `ProgressGuard` that automatically aborts the task on drop.
///
/// # Arguments
/// * `callback` - Optional progress callback to receive updates
/// * `metrics` - Atomic counters for tracking progress
/// * `config` - Configuration for the reporter
///
/// # Returns
/// A `ProgressGuard` that manages the spawned task's lifecycle.
#[must_use]
pub fn spawn_progress_reporter(
    callback: Option<Arc<dyn ProgressCallback>>,
    metrics: ProgressMetrics,
    config: ProgressReporterConfig,
) -> ProgressGuard {
    let task = callback.map(|cb| {
        tokio::spawn(async move {
            // Shared canonical rate meter (#355): sliding window + EWMA over the
            // cumulative byte counter. Seed at the resume offset so the first
            // window spans real progress.
            let mut speed_meter = SpeedMeter::new();
            speed_meter.update(config.resume_from, config.start_time);

            loop {
                tokio::time::sleep(config.update_interval).await;

                let bytes = metrics.downloaded.load(Ordering::Relaxed);
                let now = Instant::now();
                speed_meter.update(bytes, now);
                let speed = speed_meter.bytes_per_sec().unwrap_or(0.0);

                let progress_info = if let (Some(segments_counter), Some(duration_counter)) =
                    (&metrics.segments_completed, &metrics.duration_completed)
                {
                    // HLS: duration-based progress
                    let segments = segments_counter.load(Ordering::Relaxed);
                    let dur_centis = duration_counter.load(Ordering::Relaxed);
                    let dur_downloaded = dur_centis as f64 / 100.0;

                    DownloadProgress::new_with_duration(
                        bytes,
                        speed,
                        segments,
                        config.total_segments.unwrap_or(0),
                        dur_downloaded,
                        config.total_duration.unwrap_or(0.0),
                    )
                } else {
                    // HTTP: byte-based progress
                    DownloadProgress::new(bytes, config.total_size, speed)
                };

                cb.on_progress(&progress_info);

                // Exit when download is complete (HTTP only - has known total)
                if config.total_size.is_some_and(|total| bytes >= total) {
                    break;
                }
            }
        })
    });

    ProgressGuard::new(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_guard_abort() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let handle = tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(10)).await;
            });

            let mut guard = ProgressGuard::new(Some(handle));
            guard.abort();

            // Task should be aborted
            assert!(guard.0.is_none());
        });
    }

    #[test]
    fn test_progress_metrics_bytes_only() {
        let downloaded = Arc::new(AtomicU64::new(1000));
        let metrics = ProgressMetrics::bytes_only(downloaded.clone());

        assert_eq!(metrics.downloaded.load(Ordering::Relaxed), 1000);
        assert!(metrics.segments_completed.is_none());
        assert!(metrics.duration_completed.is_none());
    }

    #[test]
    fn test_progress_metrics_with_duration() {
        let downloaded = Arc::new(AtomicU64::new(1000));
        let segments = Arc::new(AtomicU64::new(10));
        let duration = Arc::new(AtomicU64::new(500)); // 5.0 seconds in centiseconds

        let metrics =
            ProgressMetrics::with_duration(downloaded.clone(), segments.clone(), duration.clone());

        assert_eq!(metrics.downloaded.load(Ordering::Relaxed), 1000);
        assert_eq!(
            metrics
                .segments_completed
                .as_ref()
                .unwrap()
                .load(Ordering::Relaxed),
            10
        );
        assert_eq!(
            metrics
                .duration_completed
                .as_ref()
                .unwrap()
                .load(Ordering::Relaxed),
            500
        );
    }

    #[test]
    fn test_config_http() {
        let config = ProgressReporterConfig::http(Instant::now(), 1_000_000, 100_000);

        assert_eq!(config.total_size, Some(1_000_000));
        assert_eq!(config.resume_from, 100_000);
        assert!(config.total_segments.is_none());
        assert!(config.total_duration.is_none());
    }

    #[test]
    fn test_config_hls() {
        let config = ProgressReporterConfig::hls(Instant::now(), 100, 600.0);

        assert!(config.total_size.is_none());
        assert_eq!(config.total_segments, Some(100));
        assert_eq!(config.total_duration, Some(600.0));
    }
}

#[cfg(test)]
mod speed_meter_tests {
    use super::SpeedMeter;
    use std::time::{Duration, Instant};

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn cold_start_returns_none_with_fewer_than_two_samples() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        assert_eq!(m.bytes_per_sec(), None);
        m.update(MIB, t0);
        assert_eq!(m.bytes_per_sec(), None, "one sample is not enough to rate");
    }

    #[test]
    fn seed_then_first_tick_rates_from_resume_offset() {
        // spawn_progress_reporter seeds the meter at the resume offset, then the
        // first tick must compute Δ from that offset — not from zero. If it
        // rated from zero this would read ~1010 MiB/s instead of ~10 MiB/s.
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        m.update(100 * MIB, t0); // resume seed
        m.update(101 * MIB, t0 + Duration::from_millis(100)); // +1 MiB in 100ms
        let bps = m.bytes_per_sec().expect("rate after seed + one tick");
        let expected = 10.0 * MIB as f64; // 1 MiB / 0.1 s
        assert!(
            (bps - expected).abs() < 0.01 * expected,
            "rated from resume offset expected ~{expected} B/s, got {bps} B/s"
        );
    }

    #[test]
    fn regression_355_microsecond_burst_is_gated_to_none_not_a_spike() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        let mut cum = 0u64;
        for i in 0..8u32 {
            cum += 5 * MIB;
            m.update(cum, t0 + Duration::from_micros(u64::from(i)));
        }
        assert_eq!(
            m.bytes_per_sec(),
            None,
            "microsecond burst must not yield a rate"
        );
    }

    #[test]
    fn regression_355_realistic_burst_reports_true_aggregate_not_spike() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        m.update(0, t0);
        m.update(20 * MIB, t0 + Duration::from_millis(100));
        m.update(40 * MIB, t0 + Duration::from_millis(200));
        let bps = m.bytes_per_sec().expect("rate after two in-window samples");
        let expected = 200.0 * MIB as f64;
        assert!(
            (bps - expected).abs() < 0.10 * expected,
            "expected ~200 MiB/s, got {bps} B/s"
        );
        assert!(
            bps < 1024.0 * MIB as f64,
            "must never reach GB/s spike: {bps} B/s"
        );
    }

    #[test]
    fn steady_rate_is_reported_after_window_fills() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        for i in 0..=40u64 {
            m.update(i * MIB, t0 + Duration::from_millis(i * 100)); // 10 MiB/s
        }
        let bps = m
            .bytes_per_sec()
            .expect("rate available after window fills");
        let expected = 10.0 * MIB as f64;
        assert!(
            (bps - expected).abs() < 0.05 * expected,
            "got {bps} B/s, expected ~{expected}"
        );
    }

    #[test]
    fn sub_gate_samples_are_debounced() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        m.update(0, t0);
        m.update(MIB, t0 + Duration::from_millis(10)); // <50ms: dropped
        assert_eq!(m.bytes_per_sec(), None);
    }

    #[test]
    fn stale_window_with_one_recent_sample_returns_none() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        m.update(MIB, t0);
        m.update(2 * MIB, t0 + Duration::from_secs(5)); // 5s > 3s window
        assert_eq!(m.bytes_per_sec(), None, "single in-window sample => None");
    }

    #[test]
    fn ewma_smooths_a_step_change_rather_than_jumping() {
        let mut m = SpeedMeter::new();
        let t0 = Instant::now();
        let mut cum = 0u64;
        let mut t = 0u64;
        for _ in 0..40 {
            cum += MIB;
            t += 100;
            m.update(cum, t0 + Duration::from_millis(t)); // 10 MiB/s
        }
        let before = m.bytes_per_sec().expect("rate");
        cum += 5 * MIB;
        t += 100;
        m.update(cum, t0 + Duration::from_millis(t)); // 50 MiB/s instantaneous
        let after = m.bytes_per_sec().expect("rate");
        assert!(after > before, "rate should rise toward the faster sample");
        assert!(
            after < 50.0 * MIB as f64,
            "EWMA must not jump to instantaneous 50 MiB/s; got {after}"
        );
    }
}
