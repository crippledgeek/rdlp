//! Adaptive chunk sizing for downloads.
//!
//! This module implements an AIMD (Additive Increase / Multiplicative Decrease)
//! controller that dynamically adjusts the chunk *size* based on observed
//! throughput. Both HTTP chunked downloads and HLS segment downloads are
//! supported via the [`ControllerMode`] enum.
//!
//! The parallel **connection count is fixed** for a download's lifetime (PRD
//! 2026-06-02 item 7): all target CDNs serve over HTTP/2, so concurrent range
//! requests multiplex as streams over one TCP connection sharing one congestion
//! window — adapting the stream count does not change bulk throughput. The
//! semaphore is sized to [`AdaptiveConfig::max_connections`] and never mutated.
//!
//! # Algorithm Overview
//!
//! The controller operates in two phases (chunk-size only):
//!
//! - **SlowStart**: Aggressively increases chunk level (+2) each adjustment
//!   interval while throughput is growing. Exits to Steady on a throughput drop
//!   or after 3 consecutive plateaus.
//! - **Steady**: Uses classic AIMD — +1 level on stable throughput, −2 levels on
//!   a >30% drop, hold on 10–30% drops.
//!
//! HLS mode skips chunk-level adjustments too (segments have fixed
//! server-determined sizes), leaving it a pure fixed-concurrency gate.

use log::{debug, info};
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

/// Chunk levels shed by one multiplicative decrease.
///
/// Two levels. `CHUNK_LEVELS` doubles at every index, so this quarters the
/// request size; −1 would be the classic AIMD halving. The value predates this
/// comment and its rationale is not recorded — do not infer one from the
/// arithmetic alone.
const MD_CHUNK_DELTA: i8 = -2;

/// Maximum number of throughput samples retained in history.
const MAX_HISTORY: usize = 8;

/// EWMA smoothing factor: weight of the newest sample.
const EWMA_ALPHA: f64 = 0.3;

/// Threshold for "significant drop" (triggers multiplicative decrease).
const MD_THRESHOLD: f64 = 0.30;

/// Lower bound of the hold band. A drop ABOVE this and below `MD_THRESHOLD`
/// is treated as noise and the level holds; at or below it, throughput counts
/// as stable or improving and the level steps up (+1). Pre-existing wording
/// said "within 10 %, hold steady", which named the one case that does not.
const NOISE_THRESHOLD: f64 = 0.10;

/// Number of consecutive plateaus in `SlowStart` before transitioning to Steady.
const SLOW_START_PLATEAU_LIMIT: usize = 3;

// ─── Phase ────────────────────────────────────────────────────────────────────

/// Congestion-control phase for the adaptive controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Aggressive ramp-up: chunk level grows by 2 each interval WHILE
    /// throughput is growing. A plateau interval counts toward the exit
    /// instead and moves the level not at all.
    SlowStart,
    /// AIMD steady state: chunk +1 level on stability, −2 levels on congestion.
    Steady,
}

// ─── ControllerMode ───────────────────────────────────────────────────────────

/// Operating mode of the adaptive controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerMode {
    /// HTTP range-request chunked downloads. Chunk *level* is adjusted; the
    /// connection count is fixed (PRD item 7).
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
    /// Number of parallel connections (FIXED for the download's lifetime).
    ///
    /// Connection-count AIMD was removed (PRD 2026-06-02 item 7): all target
    /// CDNs serve over HTTP/2, so the parallel range requests multiplex as
    /// streams over ONE TCP connection sharing one congestion window — adapting
    /// the stream count does not change bulk throughput (RFC 9113 flow control;
    /// curl/Stenberg). The controller still adapts chunk *size*. The semaphore
    /// is sized to this value and never mutated.
    pub max_connections: usize,
    /// How many completed chunks/segments to wait between AIMD adjustments.
    pub decision_interval: usize,
    /// Index into [`CHUNK_LEVELS`] to use at startup.
    pub initial_chunk_level: u8,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            max_connections: 8,
            decision_interval: 4,
            initial_chunk_level: MIN_CHUNK_LEVEL,
        }
    }
}

// ─── AdaptiveState ────────────────────────────────────────────────────────────

/// Mutable runtime state guarded by [`AdaptiveController`]'s mutex.
struct AdaptiveState {
    current_chunk_level: u8,
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
    fn new(initial_chunk_level: u8) -> Self {
        Self {
            current_chunk_level: initial_chunk_level.clamp(MIN_CHUNK_LEVEL, 7),
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

/// AIMD-based adaptive controller for chunk *sizes*. The connection count is
/// fixed (see [`AdaptiveConfig::max_connections`]).
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
        // Built from the state's level, not the config's: `AdaptiveState::new`
        // clamps to [`MIN_CHUNK_LEVEL`, 7], so reporting the raw config value
        // told the operator `chunk_level=0 (64KB)` for a controller that would
        // run at 2 (256KB) — and indexing `CHUNK_LEVELS` with an out-of-range
        // config value would panic before the clamp could help.
        let state = AdaptiveState::new(config.initial_chunk_level);
        let msg = format!(
            "Adaptive controller: mode={mode:?}, size={:.1} MB, \
             connections={} (fixed), chunk_level={} ({}KB)",
            total_size as f64 / 1024.0 / 1024.0,
            config.max_connections,
            state.current_chunk_level,
            CHUNK_LEVELS[state.current_chunk_level as usize] / 1024,
        );
        info!("{msg}");
        let semaphore = Arc::new(Semaphore::new(config.max_connections.max(1)));
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
    #[cfg(test)]
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
            // No previous measurement — stay in SlowStart and ramp chunk size up.
            let note = self.apply_chunk_delta(state, 2);
            let msg = format!(
                "Adaptive [SlowStart]: initial ramp (ewma={:.1} MB/s){note}",
                current_ewma / 1024.0 / 1024.0,
            );
            debug!("{msg}");
            self.log(&msg);
            return;
        };

        if current_ewma > prev {
            // Throughput is increasing — stay aggressive on chunk size.
            let note = self.apply_chunk_delta(state, 2);
            let msg = format!(
                "Adaptive [SlowStart]: throughput rising {:.1} → {:.1} MB/s{note}",
                prev / 1024.0 / 1024.0,
                current_ewma / 1024.0 / 1024.0,
            );
            debug!("{msg}");
            self.log(&msg);
            state.slow_start_plateaus = 0;
        } else if current_ewma < prev {
            // Throughput dropped — transition to Steady and apply one MD.
            let note = self.apply_md(state);
            let msg = format!(
                "Adaptive [SlowStart → Steady]: throughput drop {:.1} → {:.1} MB/s{note}",
                prev / 1024.0 / 1024.0,
                current_ewma / 1024.0 / 1024.0,
            );
            debug!("{msg}");
            self.log(&msg);
            state.phase = Phase::Steady;
            state.slow_start_plateaus = 0;
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
                debug!("{msg}");
                self.log(&msg);
                state.phase = Phase::Steady;
                state.slow_start_plateaus = 0;
            }
        }
    }

    /// Steady phase AIMD adjustments.
    fn adjust_steady(&self, state: &mut AdaptiveState, current_ewma: f64, prev_ewma: Option<f64>) {
        // `prev_ewma` is always `Some` here: `adjust` assigns `state.last_ewma`
        // before dispatching, and the phase only becomes `Steady` inside
        // `adjust_slow_start`, which runs after that assignment. Folding the
        // `None` case into the zero case states the invariant instead of
        // maintaining a branch nothing can reach — an earlier revision logged
        // from it, describing a trace no run could ever produce.
        debug_assert!(
            prev_ewma.is_some(),
            "adjust sets last_ewma before dispatching to Steady"
        );
        let prev = prev_ewma.unwrap_or(0.0);

        if prev == 0.0 {
            // No usable baseline — treat it as the stable case and step up.
            let note = self.apply_chunk_delta(state, 1);
            let msg = format!("Adaptive [Steady]: no prior throughput{note}");
            debug!("{msg}");
            self.log(&msg);
            return;
        }

        let ratio = (prev - current_ewma) / prev;

        if ratio > MD_THRESHOLD {
            // Throughput dropped > 30 % — multiplicative decrease.
            let note = self.apply_md(state);
            let msg = format!(
                "Adaptive [Steady]: throughput drop {:.0}% ({:.1} → {:.1} MB/s){note}",
                ratio * 100.0,
                prev / 1024.0 / 1024.0,
                current_ewma / 1024.0 / 1024.0,
            );
            debug!("{msg}");
            self.log(&msg);
        } else if ratio > NOISE_THRESHOLD {
            // 10–30 % drop — within noise, hold.
        } else {
            // Stable or improving — additive increase (+1 level).
            let note = self.apply_chunk_delta(state, 1);
            let msg = format!(
                "Adaptive [Steady]: stable at {:.1} MB/s{note}",
                current_ewma / 1024.0 / 1024.0,
            );
            debug!("{msg}");
            self.log(&msg);
        }
    }

    /// Apply multiplicative decrease: chunk level −2.
    ///
    /// The connection count is fixed (PRD item 7) — under universal HTTP/2 the
    /// parallel requests share one TCP congestion window, so reducing the stream
    /// count does not relieve congestion (and would double-penalize a link whose
    /// TCP CWND has already halved). Chunk-size reduction is the correct response.
    fn apply_md(&self, state: &mut AdaptiveState) -> String {
        self.apply_chunk_delta(state, MD_CHUNK_DELTA)
    }

    /// Apply `delta` to the chunk level and describe what actually happened.
    ///
    /// The returned clause is a message's only claim about the chunk level, and
    /// it reports the *applied* transition rather than the requested delta. The
    /// two differ because the level clamps at both ends, and because
    /// `HlsSegments` mode discards the adjustment entirely — there the clause is
    /// empty.
    ///
    /// The saturation wording exists for one structural reason:
    /// `AdaptiveConfig::default().initial_chunk_level` IS `MIN_CHUNK_LEVEL`, so
    /// a −2 requested at the start moves nothing, and an intent-derived clause
    /// would announce a cut that did not occur.
    /// `chunk_clause_reports_saturation_at_the_floor` pins exactly that.
    ///
    /// How OFTEN saturation happens is deliberately not claimed here. Every
    /// previous version of this comment tried to characterise where the level
    /// spends its time, and each one was wrong in a new way; the branch needs no
    /// frequency argument to justify it, so it makes none.
    ///
    /// These messages reach the UI's log pane through `log_callback`, so this
    /// text is read by operators, not only by the log file.
    fn apply_chunk_delta(&self, state: &mut AdaptiveState, delta: i8) -> String {
        if self.mode == ControllerMode::HlsSegments {
            return String::new();
        }
        let old_level = state.current_chunk_level;
        self.bump_chunk_level(state, delta);
        let new_level = state.current_chunk_level;
        if new_level != old_level {
            format!(
                " — chunk level {} → {} ({}KB → {}KB)",
                old_level,
                new_level,
                CHUNK_LEVELS[old_level as usize] / 1024,
                CHUNK_LEVELS[new_level as usize] / 1024,
            )
        } else if delta < 0 {
            format!(
                " — chunk already at the minimum ({}KB)",
                CHUNK_LEVELS[old_level as usize] / 1024,
            )
        } else {
            format!(
                " — chunk already at the maximum ({}KB)",
                CHUNK_LEVELS[old_level as usize] / 1024,
            )
        }
    }

    /// Adjust the chunk level by `delta`, clamped to [`MIN_CHUNK_LEVEL`, 7].
    ///
    /// In HLS mode the chunk level is not adjusted. Pure state mutation —
    /// [`Self::apply_chunk_delta`] is what reports the outcome, so that the
    /// event is described in one place rather than three.
    fn bump_chunk_level(&self, state: &mut AdaptiveState, delta: i8) {
        if self.mode == ControllerMode::HlsSegments {
            return;
        }
        let old_level = state.current_chunk_level;
        let new_level =
            (i16::from(old_level) + i16::from(delta)).clamp(i16::from(MIN_CHUNK_LEVEL), 7) as u8;
        state.current_chunk_level = new_level;
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
