//! Shared progress reporting infrastructure for downloaders.
//!
//! This module provides a unified progress reporter that works for both
//! HTTP (byte-based) and HLS (duration-based) downloads, eliminating
//! duplicate progress tracking code.

use rdlp_core::{DownloadProgress, ProgressCallback};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// RAII guard for progress reporter tasks.
///
/// Ensures the progress reporter task is aborted when the guard goes out of scope,
/// preventing task leaks on early returns or errors.
pub struct ProgressGuard(Option<tokio::task::JoinHandle<()>>);

impl ProgressGuard {
    /// Create a new progress guard wrapping an optional task handle.
    #[must_use]
    pub fn new(task: Option<tokio::task::JoinHandle<()>>) -> Self {
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
    pub fn bytes_only(downloaded: Arc<AtomicU64>) -> Self {
        Self {
            downloaded,
            segments_completed: None,
            duration_completed: None,
        }
    }

    /// Create metrics for duration-based progress (HLS downloads).
    pub fn with_duration(
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
    pub fn http(start_time: Instant, total_size: u64, resume_from: u64) -> Self {
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
    pub fn hls(start_time: Instant, total_segments: u64, total_duration: f64) -> Self {
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
            // EWMA speed state: per-tick delta smoothed with alpha=0.3.
            // Matches wget/curl responsiveness without a ring buffer.
            const EWMA_ALPHA: f64 = 0.3;
            let mut prev_bytes: u64 = config.resume_from;
            let mut prev_time: Instant = config.start_time;
            let mut smooth_speed: f64 = 0.0;

            loop {
                tokio::time::sleep(config.update_interval).await;

                let bytes = metrics.downloaded.load(Ordering::Relaxed);
                let now = Instant::now();
                let delta_bytes = bytes.saturating_sub(prev_bytes);
                let delta_secs = now.duration_since(prev_time).as_secs_f64();

                let speed = if delta_secs > 0.01 {
                    let instant_speed = delta_bytes as f64 / delta_secs;
                    smooth_speed = if smooth_speed == 0.0 {
                        instant_speed
                    } else {
                        EWMA_ALPHA * instant_speed + (1.0 - EWMA_ALPHA) * smooth_speed
                    };
                    prev_bytes = bytes;
                    prev_time = now;
                    smooth_speed
                } else {
                    smooth_speed
                };

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
