//! Adaptive chunk sizing and connection tuning for downloads.
//!
//! This module implements an AIMD (Additive Increase / Multiplicative Decrease)
//! controller that dynamically adjusts chunk sizes and parallel connection counts
//! based on observed throughput. Both HTTP chunked downloads and HLS segment
//! downloads are supported via the [`ControllerMode`] enum.
//!
//! # Algorithm Overview
//!
//! The controller operates in two phases:
//!
//! - **SlowStart**: Aggressively increases chunk level (+2) and adds a connection
//!   each adjustment interval while throughput is growing. Exits to Steady on a
//!   throughput drop or after 3 consecutive plateaus.
//! - **Steady**: Uses classic AIMD — +1 level on stable throughput, halve
//!   connections and −2 levels on a >30% drop, hold on 10–30% drops.
//!
//! HLS mode skips chunk-level adjustments (segments have fixed server-determined
//! sizes) and only tunes the connection count.

use log::info;
use rdlp_core::ProgressCallback;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Power-of-two chunk size levels (bytes), from 64 KB to 8 MB.
///
/// The level index is an index into this array. Level 0 = 64 KB (minimum),
/// level 7 = 8 MB (maximum).
pub const CHUNK_LEVELS: [usize; 8] = [
    64 * 1024,       // 64 KB
    128 * 1024,      // 128 KB
    256 * 1024,      // 256 KB
    512 * 1024,      // 512 KB
    1024 * 1024,     // 1 MB
    2 * 1024 * 1024, // 2 MB
    4 * 1024 * 1024, // 4 MB
    8 * 1024 * 1024, // 8 MB
];

/// Minimum chunk level for multiplicative decrease.
///
/// Prevents the AIMD death spiral where ever-smaller chunks increase HTTP
/// overhead, further reducing throughput, triggering more decreases. At level 2
/// (256KB), per-request overhead is <1% of payload.
const MIN_CHUNK_LEVEL: u8 = 2;

/// Maximum number of throughput samples retained in history.
const MAX_HISTORY: usize = 8;

/// EWMA smoothing factor: weight of the newest sample.
const EWMA_ALPHA: f64 = 0.3;

/// Threshold for "significant drop" (triggers multiplicative decrease).
const MD_THRESHOLD: f64 = 0.30;

/// Threshold for "mild noise" (within 10 %, hold steady).
const NOISE_THRESHOLD: f64 = 0.10;

/// Number of consecutive plateaus in `SlowStart` before transitioning to Steady.
const SLOW_START_PLATEAU_LIMIT: usize = 3;

// ─── Phase ────────────────────────────────────────────────────────────────────

/// Congestion-control phase for the adaptive controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Aggressive ramp-up: chunk level grows by 2 each interval.
    SlowStart,
    /// AIMD steady state: +1 on stability, halve on congestion.
    Steady,
}

// ─── ControllerMode ───────────────────────────────────────────────────────────

/// Operating mode of the adaptive controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerMode {
    /// HTTP range-request chunked downloads. Both chunk level and connection
    /// count are adjusted.
    HttpChunked,
    /// HLS segment downloads. Chunk-level adjustments are skipped because
    /// segment sizes are determined by the server playlist.
    HlsSegments,
}

// ─── ChunkRequest ─────────────────────────────────────────────────────────────

/// A byte-range request produced by [`AdaptiveController::next_chunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRequest {
    /// Inclusive byte offset of the first byte.
    pub start: u64,
    /// Exclusive byte offset of the last byte (i.e. `end - start` == chunk length).
    pub end: u64,
}

// ─── AdaptiveConfig ───────────────────────────────────────────────────────────

/// Configuration for [`AdaptiveController`].
#[derive(Debug, Clone)]
pub struct AdaptiveConfig {
    /// Maximum number of parallel connections permitted.
    pub max_connections: usize,
    /// How many completed chunks/segments to wait between AIMD adjustments.
    pub decision_interval: usize,
    /// Number of connections to start with.
    pub initial_connections: usize,
    /// Index into [`CHUNK_LEVELS`] to use at startup.
    pub initial_chunk_level: u8,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 2,
            initial_chunk_level: MIN_CHUNK_LEVEL,
        }
    }
}

// ─── AdaptiveState ────────────────────────────────────────────────────────────

/// Mutable runtime state guarded by [`AdaptiveController`]'s mutex.
struct AdaptiveState {
    current_chunk_level: u8,
    current_connections: usize,
    throughput_history: VecDeque<f64>,
    chunks_since_last_adjust: usize,
    bytes_assigned: u64,
    phase: Phase,
    /// Segment-to-realtime ratio (bytes/s / segment-bitrate).
    realtime_ratio: Option<f64>,
    /// Consecutive plateau count during `SlowStart`.
    slow_start_plateaus: usize,
    /// EWMA throughput from the previous adjustment, used to detect trend.
    last_ewma: Option<f64>,
}

impl AdaptiveState {
    fn new(initial_connections: usize, initial_chunk_level: u8) -> Self {
        Self {
            current_chunk_level: initial_chunk_level.clamp(MIN_CHUNK_LEVEL, 7),
            current_connections: initial_connections.max(1),
            throughput_history: VecDeque::with_capacity(MAX_HISTORY),
            chunks_since_last_adjust: 0,
            bytes_assigned: 0,
            phase: Phase::SlowStart,
            realtime_ratio: None,
            slow_start_plateaus: 0,
            last_ewma: None,
        }
    }
}

// ─── AdaptiveController ───────────────────────────────────────────────────────

/// AIMD-based adaptive controller for chunk sizes and connection counts.
///
/// Callers interact with this controller as follows:
///
/// 1. Call [`next_chunk`](Self::next_chunk) to obtain the next byte-range to
///    download. Returns `None` when the file is fully assigned.
/// 2. Acquire a permit from [`semaphore`](Self::semaphore) before starting each
///    parallel worker (this gates concurrency).
/// 3. After completing a chunk, call [`report_chunk_complete`](Self::report_chunk_complete)
///    (HTTP) or [`report_segment_complete`](Self::report_segment_complete) (HLS)
///    to record throughput and trigger potential AIMD adjustments.
pub struct AdaptiveController {
    state: Mutex<AdaptiveState>,
    semaphore: Arc<Semaphore>,
    config: AdaptiveConfig,
    total_size: u64,
    mode: ControllerMode,
    log_callback: Option<Arc<dyn ProgressCallback>>,
}

impl AdaptiveController {
    /// Create a new controller for a download of `total_size` bytes.
    ///
    /// # Arguments
    /// * `total_size` - Total number of bytes to download.
    /// * `config` - Tuning parameters (connections, intervals, initial level).
    /// * `mode` - Whether this is an HTTP chunked or HLS segment download.
    /// * `log_callback` - Optional progress callback for forwarding AIMD log messages.
    ///
    /// # Returns
    /// A new `AdaptiveController` ready to issue chunk requests.
    pub fn new(
        total_size: u64,
        config: AdaptiveConfig,
        mode: ControllerMode,
        log_callback: Option<Arc<dyn ProgressCallback>>,
    ) -> Self {
        let msg = format!(
            "Adaptive controller: mode={mode:?}, size={:.1} MB, \
             connections={}, chunk_level={} ({}KB)",
            total_size as f64 / 1024.0 / 1024.0,
            config.initial_connections,
            config.initial_chunk_level,
            CHUNK_LEVELS[config.initial_chunk_level as usize] / 1024,
        );
        info!("{msg}");
        let semaphore = Arc::new(Semaphore::new(config.initial_connections));
        let state = AdaptiveState::new(config.initial_connections, config.initial_chunk_level);
        let ctrl = Self {
            state: Mutex::new(state),
            semaphore,
            config,
            total_size,
            mode,
            log_callback,
        };
        ctrl.log(&msg);
        ctrl
    }

    /// Forward a log message to the progress callback if one is set.
    fn log(&self, message: &str) {
        if let Some(ref cb) = self.log_callback {
            cb.on_log(message);
        }
    }

    /// Returns a reference to the shared semaphore Arc.
    ///
    /// Callers should call [`Semaphore::acquire_owned`] on the returned value so
    /// that the permit is automatically released when the worker future completes.
    ///
    /// # Returns
    /// A reference to the `Arc<Semaphore>` controlling parallelism.
    pub const fn semaphore(&self) -> &Arc<Semaphore> {
        &self.semaphore
    }

    /// Returns the [`ControllerMode`] this controller was constructed with.
    ///
    /// Primarily used in tests to assert that a call site wired the correct
    /// mode (e.g. `HlsSegments` for the pre-resolved fragment path).
    pub const fn mode(&self) -> ControllerMode {
        self.mode
    }

    /// Lazily generate the next chunk request based on the current chunk level.
    ///
    /// # Returns
    /// `Some(ChunkRequest)` with the next byte range, or `None` when all bytes
    /// have been assigned.
    pub fn next_chunk(&self) -> Option<ChunkRequest> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if state.bytes_assigned >= self.total_size {
            return None;
        }

        let chunk_size = CHUNK_LEVELS[state.current_chunk_level as usize] as u64;
        let start = state.bytes_assigned;
        let end = (start + chunk_size).min(self.total_size);

        state.bytes_assigned = end;
        Some(ChunkRequest { start, end })
    }

    /// Record the completion of an HTTP chunk and potentially trigger AIMD.
    ///
    /// # Arguments
    /// * `bytes` - Number of bytes downloaded in this chunk.
    /// * `duration` - Wall-clock time taken to download the chunk.
    pub fn report_chunk_complete(&self, bytes: u64, duration: Duration) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        self.record_sample(&mut state, bytes, duration);
    }

    /// Record the completion of an HLS segment and potentially trigger AIMD.
    ///
    /// # Arguments
    /// * `bytes` - Number of bytes downloaded for this segment.
    /// * `duration` - Wall-clock time taken to download the segment.
    /// * `segment_duration_secs` - Declared playback duration of the segment (from
    ///   the m3u8 playlist), used to compute the realtime ratio. `None` if unknown.
    pub fn report_segment_complete(
        &self,
        bytes: u64,
        duration: Duration,
        segment_duration_secs: Option<f64>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Compute the realtime ratio if the segment's declared duration is known.
        if let Some(seg_dur) = segment_duration_secs
            && seg_dur > 0.0
        {
            let download_dur = duration.as_secs_f64();
            if download_dur > 0.0 {
                // ratio > 1.0 means we're downloading faster than realtime.
                state.realtime_ratio = Some(seg_dur / download_dur);
            }
        }

        self.record_sample(&mut state, bytes, duration);
    }

    // ─── Private helpers ──────────────────────────────────────────────────────

    /// Record a throughput sample and trigger AIMD if the interval has elapsed.
    fn record_sample(&self, state: &mut AdaptiveState, bytes: u64, duration: Duration) {
        let secs = duration.as_secs_f64();
        if secs > 0.0 {
            let throughput = bytes as f64 / secs;
            if state.throughput_history.len() >= MAX_HISTORY {
                state.throughput_history.pop_front();
            }
            state.throughput_history.push_back(throughput);
        }

        state.chunks_since_last_adjust += 1;
        if state.chunks_since_last_adjust >= self.config.decision_interval {
            self.adjust(state);
        }
    }

    /// Compute an EWMA over the throughput history.
    ///
    /// Returns `None` if the history is empty.
    fn compute_ewma(history: &VecDeque<f64>) -> Option<f64> {
        let mut iter = history.iter();
        let first = iter.next()?;
        let ewma = iter.fold(*first, |acc, &sample| {
            EWMA_ALPHA.mul_add(sample, (1.0 - EWMA_ALPHA) * acc)
        });
        Some(ewma)
    }

    /// Core AIMD adjustment logic.
    ///
    /// Called after `decision_interval` chunks/segments have completed.
    fn adjust(&self, state: &mut AdaptiveState) {
        state.chunks_since_last_adjust = 0;

        let Some(current_ewma) = Self::compute_ewma(&state.throughput_history) else {
            return;
        };

        let prev_ewma = state.last_ewma;
        state.last_ewma = Some(current_ewma);

        match state.phase {
            Phase::SlowStart => self.adjust_slow_start(state, current_ewma, prev_ewma),
            Phase::Steady => self.adjust_steady(state, current_ewma, prev_ewma),
        }
    }

    /// `SlowStart` phase AIMD adjustments.
    fn adjust_slow_start(
        &self,
        state: &mut AdaptiveState,
        current_ewma: f64,
        prev_ewma: Option<f64>,
    ) {
        let Some(prev) = prev_ewma else {
            // No previous measurement — stay in SlowStart and ramp up.
            let msg = format!(
                "Adaptive [SlowStart]: initial ramp — chunk +2, +1 connection \
                 (ewma={:.1} MB/s)",
                current_ewma / 1024.0 / 1024.0,
            );
            info!("{msg}");
            self.log(&msg);
            self.bump_chunk_level(state, 2);
            self.increase_connections(state);
            return;
        };

        if current_ewma > prev {
            // Throughput is increasing — stay aggressive.
            let msg = format!(
                "Adaptive [SlowStart]: throughput rising {:.1} → {:.1} MB/s — \
                 chunk +2, +1 connection",
                prev / 1024.0 / 1024.0,
                current_ewma / 1024.0 / 1024.0,
            );
            info!("{msg}");
            self.log(&msg);
            state.slow_start_plateaus = 0;
            self.bump_chunk_level(state, 2);
            self.increase_connections(state);
        } else if current_ewma < prev {
            // Throughput dropped — transition to Steady and apply one MD.
            let msg = format!(
                "Adaptive [SlowStart → Steady]: throughput drop {:.1} → {:.1} MB/s — \
                 applying multiplicative decrease",
                prev / 1024.0 / 1024.0,
                current_ewma / 1024.0 / 1024.0,
            );
            info!("{msg}");
            self.log(&msg);
            state.phase = Phase::Steady;
            state.slow_start_plateaus = 0;
            self.apply_md(state);
        } else {
            // Plateau: throughput unchanged.
            state.slow_start_plateaus += 1;
            if state.slow_start_plateaus >= SLOW_START_PLATEAU_LIMIT {
                let msg = format!(
                    "Adaptive [SlowStart → Steady]: {} consecutive plateaus at \
                     {:.1} MB/s — transitioning",
                    SLOW_START_PLATEAU_LIMIT,
                    current_ewma / 1024.0 / 1024.0,
                );
                info!("{msg}");
                self.log(&msg);
                state.phase = Phase::Steady;
                state.slow_start_plateaus = 0;
            }
        }
    }

    /// Steady phase AIMD adjustments.
    fn adjust_steady(&self, state: &mut AdaptiveState, current_ewma: f64, prev_ewma: Option<f64>) {
        let Some(prev) = prev_ewma else {
            // First measurement in Steady — apply additive increase.
            self.bump_chunk_level(state, 1);
            return;
        };

        if prev == 0.0 {
            self.bump_chunk_level(state, 1);
            return;
        }

        let ratio = (prev - current_ewma) / prev;

        if ratio > MD_THRESHOLD {
            // Throughput dropped > 30 % — multiplicative decrease.
            let msg = format!(
                "Adaptive [Steady]: throughput drop {:.0}% ({:.1} → {:.1} MB/s) — \
                 multiplicative decrease",
                ratio * 100.0,
                prev / 1024.0 / 1024.0,
                current_ewma / 1024.0 / 1024.0,
            );
            info!("{msg}");
            self.log(&msg);
            self.apply_md(state);
        } else if ratio > NOISE_THRESHOLD {
            // 10–30 % drop — within noise, hold.
        } else {
            // Stable or improving — additive increase (+1 level).
            let msg = format!(
                "Adaptive [Steady]: stable at {:.1} MB/s — chunk +1",
                current_ewma / 1024.0 / 1024.0,
            );
            info!("{msg}");
            self.log(&msg);
            self.bump_chunk_level(state, 1);
        }
    }

    /// Apply multiplicative decrease: halve connections, chunk level −2.
    fn apply_md(&self, state: &mut AdaptiveState) {
        let before_conns = state.current_connections;
        let before_level = state.current_chunk_level;
        let target = (state.current_connections / 2).max(1);
        self.decrease_connections(state, target);
        self.bump_chunk_level(state, -(2i8));
        let msg = format!(
            "Adaptive MD: connections {} → {}, chunk level {} → {} ({}KB)",
            before_conns,
            state.current_connections,
            before_level,
            state.current_chunk_level,
            CHUNK_LEVELS[state.current_chunk_level as usize] / 1024,
        );
        info!("{msg}");
        self.log(&msg);
    }

    /// Adjust the chunk level by `delta`, clamped to [`MIN_CHUNK_LEVEL`, 7].
    ///
    /// In HLS mode the chunk level is not adjusted.
    fn bump_chunk_level(&self, state: &mut AdaptiveState, delta: i8) {
        if self.mode == ControllerMode::HlsSegments {
            return;
        }
        let old_level = state.current_chunk_level;
        let new_level =
            (i16::from(old_level) + i16::from(delta)).clamp(i16::from(MIN_CHUNK_LEVEL), 7) as u8;
        state.current_chunk_level = new_level;
        if new_level != old_level {
            let msg = format!(
                "Adaptive: chunk level {} → {} ({}KB → {}KB)",
                old_level,
                new_level,
                CHUNK_LEVELS[old_level as usize] / 1024,
                CHUNK_LEVELS[new_level as usize] / 1024,
            );
            info!("{msg}");
            self.log(&msg);
        }
    }

    /// Increase connections by 1, up to `max_connections`.
    ///
    /// Adds a permit to the semaphore so an additional worker can be dispatched.
    fn increase_connections(&self, state: &mut AdaptiveState) {
        if state.current_connections < self.config.max_connections {
            state.current_connections += 1;
            self.semaphore.add_permits(1);
            let msg = format!(
                "Adaptive: connections → {} (max {})",
                state.current_connections, self.config.max_connections,
            );
            info!("{msg}");
            self.log(&msg);
        }
    }

    /// Reduce connections to `target` (minimum 1).
    ///
    /// For each permit to remove, attempts a `try_acquire` and forgets the
    /// permit so it is permanently removed from the semaphore budget.
    fn decrease_connections(&self, state: &mut AdaptiveState, target: usize) {
        let target = target.max(1);
        while state.current_connections > target {
            // try_acquire is non-blocking; if no permit is immediately
            // available the semaphore is already under pressure — stop.
            if let Ok(permit) = self.semaphore.try_acquire() {
                permit.forget();
                state.current_connections -= 1;
            } else {
                break;
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a default controller for HTTP mode with given `total_size`.
    fn make_controller(total_size: u64) -> AdaptiveController {
        AdaptiveController::new(
            total_size,
            AdaptiveConfig::default(),
            ControllerMode::HttpChunked,
            None,
        )
    }

    /// Build a controller with explicit config.
    fn make_controller_cfg(
        total_size: u64,
        config: AdaptiveConfig,
        mode: ControllerMode,
    ) -> AdaptiveController {
        AdaptiveController::new(total_size, config, mode, None)
    }

    /// Drive the controller through `n` chunks and report each at `throughput` bytes/s.
    fn drive(ctrl: &AdaptiveController, n: usize, throughput_bps: f64) {
        for _ in 0..n {
            let chunk = ctrl.next_chunk();
            if let Some(req) = chunk {
                let bytes = req.end - req.start;
                let dur = Duration::from_secs_f64(bytes as f64 / throughput_bps);
                ctrl.report_chunk_complete(bytes, dur);
            }
        }
    }

    // ── next_chunk tests ──────────────────────────────────────────────────────

    #[test]
    fn test_next_chunk_basic() {
        let ctrl = make_controller(1024 * 1024); // 1 MB
        let chunk = ctrl.next_chunk().unwrap();
        // Level 2 (MIN_CHUNK_LEVEL) = 256 KB
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, 256 * 1024);
    }

    #[test]
    fn test_next_chunk_variable_sizes() {
        let ctrl = make_controller(10 * 1024 * 1024);
        let first = ctrl.next_chunk().unwrap();
        assert_eq!(first.start, 0);
        let first_size = first.end - first.start;

        // Manually bump level via a state mutation.
        {
            let mut state = ctrl.state.lock().unwrap();
            state.current_chunk_level = 3; // 512 KB
        }

        let second = ctrl.next_chunk().unwrap();
        let second_size = second.end - second.start;
        assert_ne!(
            first_size, second_size,
            "chunk sizes should differ after level change"
        );
        assert_eq!(second_size, 512 * 1024);
    }

    #[test]
    fn test_next_chunk_exhaustion() {
        let total = 200 * 1024; // 200 KB
        let ctrl = make_controller(total as u64);

        // Drain all chunks.
        let mut total_bytes = 0u64;
        while let Some(req) = ctrl.next_chunk() {
            total_bytes += req.end - req.start;
        }

        assert_eq!(total_bytes, total as u64);
        // Further calls return None.
        assert!(ctrl.next_chunk().is_none());
    }

    // ── slow-start tests ──────────────────────────────────────────────────────

    #[test]
    fn test_slow_start_ramp_up() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 2,
            initial_chunk_level: 2, // MIN_CHUNK_LEVEL (was 0)
        };
        let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // First interval — no prev_ewma, so it bumps +2 and +1 connection.
        drive(&ctrl, 4, 10_000_000.0);
        let (level1, conns1) = {
            let state = ctrl.state.lock().unwrap();
            (state.current_chunk_level, state.current_connections)
        };
        assert_eq!(
            level1, 4,
            "chunk level should have increased by 2 from floor"
        );
        assert_eq!(conns1, 3, "connections should have increased by 1");

        // Second interval — throughput still high (increasing) → another ramp.
        drive(&ctrl, 4, 12_000_000.0);
        let (level2, conns2) = {
            let state = ctrl.state.lock().unwrap();
            (state.current_chunk_level, state.current_connections)
        };
        assert!(level2 >= level1, "chunk level should not decrease");
        assert!(conns2 >= conns1, "connections should not decrease");
    }

    #[test]
    fn test_slow_start_to_steady_on_decrease() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 4,
            initial_chunk_level: 4,
        };
        let ctrl = make_controller_cfg(200 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // First interval at high throughput.
        drive(&ctrl, 4, 10_000_000.0);

        // Second interval at drastically lower throughput — should trigger MD
        // and transition to Steady.
        drive(&ctrl, 4, 1_000_000.0);

        let state = ctrl.state.lock().unwrap();
        assert_eq!(
            state.phase,
            Phase::Steady,
            "should have transitioned to Steady"
        );
    }

    #[test]
    fn test_slow_start_plateau_exit() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 2,
            initial_chunk_level: 0,
        };
        let ctrl = make_controller_cfg(200 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Seed state so that current_ewma == prev_ewma every time adjust runs.
        // Pre-fill history with a constant value so the EWMA converges exactly.
        {
            let mut state = ctrl.state.lock().unwrap();
            for _ in 0..MAX_HISTORY {
                state.throughput_history.push_back(5_000_000.0);
            }
            // Pre-set last_ewma to the same value so the first call is already
            // a plateau (not a "no prev" ramp-up).
            state.last_ewma = Some(5_000_000.0);
        }

        // Trigger SLOW_START_PLATEAU_LIMIT adjustment rounds, each with a
        // plateau (current_ewma == prev_ewma == 5 MB/s).
        for _ in 0..SLOW_START_PLATEAU_LIMIT {
            let mut state = ctrl.state.lock().unwrap();
            ctrl.adjust(&mut state);
        }

        let state = ctrl.state.lock().unwrap();
        // After 3 plateaus the phase should be Steady.
        assert_eq!(
            state.phase,
            Phase::Steady,
            "should transition to Steady after 3 plateaus (got {} plateaus so far)",
            state.slow_start_plateaus
        );
    }

    // ── steady-state tests ────────────────────────────────────────────────────

    #[test]
    fn test_steady_additive_increase() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 4,
            initial_chunk_level: 2,
        };
        let ctrl = make_controller_cfg(500 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Force into Steady phase with a known last_ewma.
        {
            let mut state = ctrl.state.lock().unwrap();
            state.phase = Phase::Steady;
            state.last_ewma = Some(8_000_000.0);
            for _ in 0..MAX_HISTORY {
                state.throughput_history.push_back(8_000_000.0);
            }
        }

        let initial_level = ctrl.state.lock().unwrap().current_chunk_level;

        // Drive one interval at the same throughput — stable → AI.
        drive(&ctrl, 4, 8_000_000.0);

        let final_level = ctrl.state.lock().unwrap().current_chunk_level;
        assert!(
            final_level >= initial_level,
            "AI: chunk level should not decrease in stable throughput"
        );
    }

    #[test]
    fn test_steady_multiplicative_decrease() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 6,
            initial_chunk_level: 5,
        };
        let ctrl = make_controller_cfg(500 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Force into Steady with high prev_ewma.
        {
            let mut state = ctrl.state.lock().unwrap();
            state.phase = Phase::Steady;
            state.last_ewma = Some(10_000_000.0);
            state.current_connections = 6;
            state.current_chunk_level = 5;
            for _ in 0..MAX_HISTORY {
                state.throughput_history.push_back(10_000_000.0);
            }
        }

        // Drive at very low throughput (>30 % drop from prev_ewma).
        drive(&ctrl, 4, 1_000_000.0);

        let state = ctrl.state.lock().unwrap();
        // Connections should have halved (from 6 → 3).
        assert!(
            state.current_connections <= 4,
            "MD: connections should have halved (was 6, now {})",
            state.current_connections
        );
        // Chunk level should have dropped by 2.
        assert!(
            state.current_chunk_level <= 5,
            "MD: chunk level should have decreased"
        );
    }

    #[test]
    fn test_steady_noise_tolerance() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 4,
            initial_chunk_level: 3,
        };
        let ctrl = make_controller_cfg(500 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Force Steady with prev_ewma = 10 MB/s.
        {
            let mut state = ctrl.state.lock().unwrap();
            state.phase = Phase::Steady;
            state.last_ewma = Some(10_000_000.0);
            state.current_chunk_level = 3;
            state.current_connections = 4;
            // Seed history with ~15% lower throughput to land in noise band.
            for _ in 0..MAX_HISTORY {
                state.throughput_history.push_back(8_500_000.0);
            }
        }

        let level_before = ctrl.state.lock().unwrap().current_chunk_level;
        let conns_before = ctrl.state.lock().unwrap().current_connections;

        // Trigger one adjustment interval manually.
        {
            let mut state = ctrl.state.lock().unwrap();
            state.chunks_since_last_adjust = ctrl.config.decision_interval;
            ctrl.adjust(&mut state);
        }

        let state = ctrl.state.lock().unwrap();
        // In the noise band: hold — level and connections unchanged.
        assert_eq!(
            state.current_chunk_level, level_before,
            "noise: chunk level should be held"
        );
        assert_eq!(
            state.current_connections, conns_before,
            "noise: connections should be held"
        );
    }

    // ── bounds tests ──────────────────────────────────────────────────────────

    #[test]
    fn test_bounds_chunk_level() {
        let cfg = AdaptiveConfig {
            max_connections: 2,
            decision_interval: 1,
            initial_connections: 1,
            initial_chunk_level: 2, // MIN_CHUNK_LEVEL (was 0)
        };
        let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Repeatedly drive with increasing throughput to try to exceed level 7.
        for i in 0..20 {
            drive(&ctrl, 1, (i + 1) as f64 * 1_000_000.0);
        }
        let level = ctrl.state.lock().unwrap().current_chunk_level;
        assert!(level <= 7, "chunk level must not exceed 7, got {level}");

        // Now force into Steady and apply many MDs to try to go below MIN_CHUNK_LEVEL.
        {
            let mut state = ctrl.state.lock().unwrap();
            state.phase = Phase::Steady;
            state.current_chunk_level = 2; // MIN_CHUNK_LEVEL (was 0)
            state.last_ewma = Some(10_000_000.0);
            for _ in 0..MAX_HISTORY {
                state.throughput_history.push_back(100.0);
            }
            state.chunks_since_last_adjust = ctrl.config.decision_interval;
            ctrl.adjust(&mut state);
        }
        let level = ctrl.state.lock().unwrap().current_chunk_level;
        assert_eq!(level, 2, "chunk level must not go below MIN_CHUNK_LEVEL");
    }

    #[test]
    fn test_bounds_connections() {
        let cfg = AdaptiveConfig {
            max_connections: 4,
            decision_interval: 1,
            initial_connections: 2,
            initial_chunk_level: 0,
        };
        let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Try to exceed max_connections.
        for _ in 0..10 {
            let mut state = ctrl.state.lock().unwrap();
            ctrl.increase_connections(&mut state);
        }
        let conns = ctrl.state.lock().unwrap().current_connections;
        assert!(
            conns <= 4,
            "connections must not exceed max (4), got {conns}"
        );

        // Try to go below 1.
        for _ in 0..10 {
            let mut state = ctrl.state.lock().unwrap();
            ctrl.decrease_connections(&mut state, 0);
        }
        let conns = ctrl.state.lock().unwrap().current_connections;
        assert!(conns >= 1, "connections must be at least 1, got {conns}");
    }

    // ── HLS mode test ─────────────────────────────────────────────────────────

    #[test]
    fn test_hls_mode_skips_chunk_level() {
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 4,
            initial_connections: 2,
            initial_chunk_level: 0,
        };
        let ctrl = make_controller_cfg(500 * 1024 * 1024, cfg, ControllerMode::HlsSegments);
        let initial_level = ctrl.state.lock().unwrap().current_chunk_level;

        // Drive many intervals to trigger multiple adjustments.
        for i in 0..20 {
            let bytes = 512u64 * 1024;
            let throughput = (i + 1) as f64 * 500_000.0;
            let dur = Duration::from_secs_f64(bytes as f64 / throughput);
            ctrl.report_segment_complete(bytes, dur, Some(2.0));
        }

        let final_level = ctrl.state.lock().unwrap().current_chunk_level;
        assert_eq!(
            final_level, initial_level,
            "HLS mode must not adjust chunk level"
        );
    }

    // ── chunk level floor tests ───────────────────────────────────────────────

    #[test]
    fn test_chunk_level_floor_on_multiplicative_decrease() {
        // Controller at level 3 with many MD triggers should never go below MIN_CHUNK_LEVEL (2).
        let cfg = AdaptiveConfig {
            max_connections: 8,
            decision_interval: 2,
            initial_connections: 4,
            initial_chunk_level: 3,
        };
        let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Drive into Steady phase first with good throughput.
        drive(&ctrl, 8, 10_000_000.0);

        // Now simulate severe throughput drops to trigger repeated MD.
        for _ in 0..10 {
            drive(&ctrl, 2, 100_000.0); // Very low throughput → MD triggers
        }

        let state = ctrl.state.lock().unwrap();
        assert!(
            state.current_chunk_level >= 2,
            "Chunk level {} dropped below floor 2",
            state.current_chunk_level
        );
    }

    #[test]
    fn test_initial_chunk_level_clamped_to_floor() {
        // Constructing with initial_chunk_level=0 should clamp to MIN_CHUNK_LEVEL (2).
        let cfg = AdaptiveConfig {
            initial_chunk_level: 0,
            ..Default::default()
        };
        let ctrl = make_controller_cfg(1024 * 1024, cfg, ControllerMode::HttpChunked);

        let state = ctrl.state.lock().unwrap();
        assert_eq!(
            state.current_chunk_level, 2,
            "Initial chunk level should be clamped to floor"
        );
    }

    #[test]
    fn test_floor_does_not_block_upward_movement() {
        // From level 2 (the floor), bumping +1 should reach level 3.
        let cfg = AdaptiveConfig {
            initial_chunk_level: 2,
            ..Default::default()
        };
        let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

        // Manually bump up.
        {
            let mut state = ctrl.state.lock().unwrap();
            ctrl.bump_chunk_level(&mut state, 1);
            assert_eq!(state.current_chunk_level, 3);
        }
    }

    // ── realtime ratio test ───────────────────────────────────────────────────

    #[test]
    fn test_realtime_ratio_calculation() {
        let ctrl = AdaptiveController::new(
            10 * 1024 * 1024,
            AdaptiveConfig::default(),
            ControllerMode::HlsSegments,
            None,
        );

        // A 4-second segment downloaded in 1 second → ratio = 4.0.
        let bytes = 512u64 * 1024;
        let download_duration = Duration::from_secs(1);
        ctrl.report_segment_complete(bytes, download_duration, Some(4.0));

        let ratio = ctrl.state.lock().unwrap().realtime_ratio;
        assert!(ratio.is_some(), "realtime_ratio should be set");
        let ratio = ratio.unwrap();
        assert!(
            (ratio - 4.0).abs() < 1e-6,
            "realtime_ratio should be 4.0, got {ratio}"
        );
    }
}
