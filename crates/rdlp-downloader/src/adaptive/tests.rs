use super::*;
use std::sync::{Arc, Mutex};

/// Captures `on_log` so a test can assert on the controller's message text.
struct RecordingCallback {
    logs: Arc<Mutex<Vec<String>>>,
}

impl rdlp_core::ProgressCallback for RecordingCallback {
    fn on_progress(&self, _p: &rdlp_core::DownloadProgress) {}
    fn on_complete(&self, _s: &rdlp_core::DownloadStats) {}
    fn on_error(&self, _e: &str) {}
    fn on_log(&self, msg: &str) {
        self.logs.lock().expect("lock").push(msg.to_string());
    }
}

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
        initial_chunk_level: 2, // MIN_CHUNK_LEVEL (was 0)
    };
    let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

    // First interval — no prev_ewma, so it bumps chunk +2 (connection count is
    // fixed; PRD item 7).
    drive(&ctrl, 4, 10_000_000.0);
    let level1 = ctrl.state.lock().unwrap().current_chunk_level;
    assert_eq!(
        level1, 4,
        "chunk level should have increased by 2 from floor"
    );
    // Connection count is fixed at max_connections — the semaphore is never
    // mutated by the controller.
    assert_eq!(ctrl.semaphore().available_permits(), 8);

    // Second interval — throughput still high (increasing) → another chunk ramp.
    drive(&ctrl, 4, 12_000_000.0);
    let level2 = ctrl.state.lock().unwrap().current_chunk_level;
    assert!(level2 >= level1, "chunk level should not decrease");
    assert_eq!(
        ctrl.semaphore().available_permits(),
        8,
        "connection count stays fixed across adjustments"
    );
}

#[test]
fn test_slow_start_to_steady_on_decrease() {
    let cfg = AdaptiveConfig {
        max_connections: 8,
        decision_interval: 4,
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
        initial_chunk_level: 5,
    };
    let ctrl = make_controller_cfg(500 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

    // Force into Steady with high prev_ewma.
    {
        let mut state = ctrl.state.lock().unwrap();
        state.phase = Phase::Steady;
        state.last_ewma = Some(10_000_000.0);
        state.current_chunk_level = 5;
        for _ in 0..MAX_HISTORY {
            state.throughput_history.push_back(10_000_000.0);
        }
    }

    // Drive at very low throughput (>30 % drop from prev_ewma).
    drive(&ctrl, 4, 1_000_000.0);

    let state = ctrl.state.lock().unwrap();
    // MD now only reduces chunk size (connections are fixed; PRD item 7).
    assert!(
        state.current_chunk_level <= 5,
        "MD: chunk level should have decreased"
    );
    drop(state);
    assert_eq!(
        ctrl.semaphore().available_permits(),
        8,
        "MD must NOT touch the (fixed) connection count"
    );
}

#[test]
fn test_steady_noise_tolerance() {
    let cfg = AdaptiveConfig {
        max_connections: 8,
        decision_interval: 4,
        initial_chunk_level: 3,
    };
    let ctrl = make_controller_cfg(500 * 1024 * 1024, cfg, ControllerMode::HttpChunked);

    // Force Steady with prev_ewma = 10 MB/s.
    {
        let mut state = ctrl.state.lock().unwrap();
        state.phase = Phase::Steady;
        state.last_ewma = Some(10_000_000.0);
        state.current_chunk_level = 3;
        // Seed history with ~15% lower throughput to land in noise band.
        for _ in 0..MAX_HISTORY {
            state.throughput_history.push_back(8_500_000.0);
        }
    }

    let level_before = ctrl.state.lock().unwrap().current_chunk_level;

    // Trigger one adjustment interval manually.
    {
        let mut state = ctrl.state.lock().unwrap();
        state.chunks_since_last_adjust = ctrl.config.decision_interval;
        ctrl.adjust(&mut state);
    }

    // In the noise band: hold — chunk level unchanged (connections always fixed).
    assert_eq!(
        ctrl.state.lock().unwrap().current_chunk_level,
        level_before,
        "noise: chunk level should be held"
    );
}

// ── bounds tests ──────────────────────────────────────────────────────────

#[test]
fn test_bounds_chunk_level() {
    let cfg = AdaptiveConfig {
        max_connections: 2,
        decision_interval: 1,
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
fn test_connection_count_is_fixed() {
    // PRD item 7: connection-count AIMD removed. The semaphore is sized to
    // `max_connections` and is NEVER mutated by the controller — driving
    // through slow-start ramp-up AND multiplicative decrease leaves the permit
    // count unchanged. (No `increase_connections`/`decrease_connections` exist.)
    let cfg = AdaptiveConfig {
        max_connections: 4,
        decision_interval: 1,
        initial_chunk_level: 0,
    };
    let ctrl = make_controller_cfg(100 * 1024 * 1024, cfg, ControllerMode::HttpChunked);
    assert_eq!(
        ctrl.semaphore().available_permits(),
        4,
        "sized to max_connections"
    );

    // Ramp up (rising throughput) then crash (triggers MD) — connections fixed.
    drive(&ctrl, 4, 10_000_000.0);
    drive(&ctrl, 4, 100_000.0);
    assert_eq!(
        ctrl.semaphore().available_permits(),
        4,
        "connection count must stay fixed across ramp + MD"
    );
}

// ── HLS mode test ─────────────────────────────────────────────────────────

#[test]
fn test_hls_mode_skips_chunk_level() {
    let cfg = AdaptiveConfig {
        max_connections: 8,
        decision_interval: 4,
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

/// The controller's per-adjustment chatter stays at DEBUG.
///
/// It ran at INFO, which produced 96 lines in a single 287-line session of the
/// desktop's log file — with a 5 MiB rotation window, sustained downloading
/// could rotate a genuine WARN out of the file before anyone read it.
///
/// The messages are not lost. Every one still goes to `log_callback`, the
/// channel the UI reads, so the `log::` record was a duplicate wherever a
/// caller supplies a progress callback — which is all three controller
/// construction sites (`http/parallel.rs`, `dash/download.rs`, and
/// `fragments.rs`, the last of which passed `None` until this change and so
/// was the one path where the demotion would genuinely have lost them).
/// On the CLI they also remain reachable with `-v`, whose filter admits DEBUG.
///
/// The startup summary (one line per download, naming mode/size/connections)
/// stays at INFO deliberately — that one is operator-relevant.
#[test]
fn per_adjustment_messages_are_debug_not_info() {
    testing_logger::setup();
    let ctrl = make_controller(64 * 1024 * 1024);

    // Twice `AdaptiveConfig::default().decision_interval`, so `adjust` runs at
    // least once however that default moves — deriving the count is what keeps
    // it that way, where a hard-coded loop bound would silently stop reaching
    // `adjust` the first time the interval was raised.
    let chunks = 2 * AdaptiveConfig::default().decision_interval;
    for _ in 0..chunks {
        ctrl.report_chunk_complete(1024 * 1024, std::time::Duration::from_millis(500));
    }

    testing_logger::validate(|captured| {
        let infos: Vec<&String> = captured
            .iter()
            .filter(|l| l.level == log::Level::Info)
            .map(|l| &l.body)
            .collect();
        // The one-per-download summary may appear; nothing else may.
        for body in &infos {
            assert!(
                body.starts_with("Adaptive controller:"),
                "only the startup summary belongs at INFO, got: {body}"
            );
        }
        assert!(
            infos.len() <= 1,
            "expected at most the startup summary at INFO, got {} lines",
            infos.len()
        );

        // The positive half. Without it the upper bound above is satisfied
        // just as well by an adjust path that logged nothing at all — by the
        // call sites being deleted, or by `adjust` never being reached — so
        // the test would keep passing while covering nothing.
        let debugs = captured
            .iter()
            .filter(|l| l.level == log::Level::Debug)
            .count();
        assert!(
            debugs >= 1,
            "the adjust path must have logged at DEBUG; without a record here \
             the INFO bound above proves nothing"
        );
    });
}

/// In `HlsSegments` mode the tuning messages must not claim a chunk change.
///
/// `bump_chunk_level` discards the adjustment there (segment sizes are
/// server-determined), so a "chunk +2" clause describes something that did not
/// happen. These messages reach the desktop's log pane through `log_callback`,
/// so the text is read by operators — this became visible when the HLS path
/// started passing a callback.
#[test]
fn hls_mode_messages_claim_no_chunk_change() {
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(RecordingCallback {
        logs: Arc::clone(&logged),
    });
    let ctrl = AdaptiveController::new(
        0,
        AdaptiveConfig::default(),
        ControllerMode::HlsSegments,
        Some(sink),
    );

    // Drive several decision intervals across rising, falling and flat
    // throughput so every phase-transition message is exercised.
    for bps in [4.0, 8.0, 8.0, 1.0, 1.0, 4.0] {
        for _ in 0..AdaptiveConfig::default().decision_interval {
            let bytes = (bps * 1024.0 * 1024.0) as u64;
            ctrl.report_chunk_complete(bytes, std::time::Duration::from_secs(1));
        }
    }

    let msgs = logged.lock().expect("lock").clone();
    assert!(
        msgs.len() > 1,
        "expected tuning messages beyond the startup summary, got {msgs:?}"
    );
    for m in &msgs {
        // Matches "chunk " rather than the "+"/"level" spellings: a partial
        // regression restoring only the negative clause would slip past those
        // and still tell an HLS operator the chunk size had changed.
        assert!(
            !m.contains("chunk "),
            "HLS mode must not claim a chunk adjustment, got: {m}"
        );
    }
}

/// The chunk clause reports the transition that was applied.
///
/// Positive half of the pair below: when the level genuinely moves, the message
/// names the real before/after rather than the requested delta.
#[test]
fn chunk_clause_reports_the_applied_transition() {
    let ctrl = make_controller(64 * 1024 * 1024);
    let mut state = ctrl.state.lock().unwrap();

    // From the default (== MIN_CHUNK_LEVEL) an increase does move the level.
    let note = ctrl.apply_chunk_delta(&mut state, 1);

    assert!(
        note.contains("chunk level 2 → 3"),
        "expected the applied transition, got: {note}"
    );
    assert_eq!(
        state.current_chunk_level,
        MIN_CHUNK_LEVEL + 1,
        "the clause must describe a transition that really happened"
    );
}

/// At the floor the clause says so instead of claiming a decrease.
///
/// This is the regression the intent-derived version shipped: `MIN_CHUNK_LEVEL`
/// is also the default starting level, so a −2 requested at the start clamps
/// and moves nothing — while the message announced "chunk -2" regardless.
/// Saturation is the state an operator most needs during a throughput collapse
/// that nothing relieves, so it is reported rather than omitted.
///
/// No claim is made here about how often that happens; see `apply_chunk_delta`
/// for why this comment set stopped making frequency arguments.
///
/// Fails against the intent-derived version, which had no floor branch at all.
#[test]
fn chunk_clause_reports_saturation_at_the_floor() {
    let ctrl = make_controller(64 * 1024 * 1024);
    let mut state = ctrl.state.lock().unwrap();
    assert_eq!(
        state.current_chunk_level, MIN_CHUNK_LEVEL,
        "precondition: the default start IS the floor, so this delta has \
         nowhere to go"
    );

    let note = ctrl.apply_chunk_delta(&mut state, MD_CHUNK_DELTA);

    assert!(
        note.contains("already at the minimum"),
        "the floor must be reported, got: {note}"
    );
    assert!(
        !note.contains('→'),
        "no transition may be claimed when the level did not move, got: {note}"
    );
    assert_eq!(
        state.current_chunk_level, MIN_CHUNK_LEVEL,
        "the level must not have moved"
    );
}

/// Pins the +2 ramp and the floor clamp through the controller's own phase
/// machine.
///
/// The clause doc no longer narrates this sequence — repeated attempts to put
/// it in prose were each wrong — so it is asserted here instead. The
/// load-bearing half is that the FIRST adjustment of a real
/// download ramps by +2 — so the ramp is driven through the controller's own
/// phase machine (`report_chunk_complete` → `adjust` → `adjust_slow_start`'s
/// `prev_ewma == None` arm) rather than by calling `apply_chunk_delta` with a
/// hard-coded 2, which would leave the doc's claim unpinned if that arm's delta
/// ever changed.
#[test]
fn floor_is_one_decrease_below_the_first_ramp() {
    let ctrl = make_controller(64 * 1024 * 1024);
    assert_eq!(
        ctrl.state.lock().unwrap().current_chunk_level,
        MIN_CHUNK_LEVEL,
        "a default download starts on the floor"
    );

    // One full decision interval — the controller's first adjustment. The lock
    // must NOT be held here: `report_chunk_complete` takes it itself.
    drive(
        &ctrl,
        AdaptiveConfig::default().decision_interval,
        10_000_000.0,
    );
    assert_eq!(
        ctrl.state.lock().unwrap().current_chunk_level,
        MIN_CHUNK_LEVEL + 2,
        "the first adjustment takes the SlowStart ramp (+2)"
    );

    let mut state = ctrl.state.lock().unwrap();

    // One decrease returns it to the floor — and says so.
    let first = ctrl.apply_chunk_delta(&mut state, MD_CHUNK_DELTA);
    assert_eq!(state.current_chunk_level, MIN_CHUNK_LEVEL);
    assert!(
        first.contains('→'),
        "that decrease does move the level, got: {first}"
    );

    // A further decrease, with no increase in between, is the saturating one.
    let second = ctrl.apply_chunk_delta(&mut state, MD_CHUNK_DELTA);
    assert_eq!(state.current_chunk_level, MIN_CHUNK_LEVEL);
    assert!(
        second.contains("already at the minimum"),
        "the next consecutive decrease saturates, got: {second}"
    );
}

/// The startup summary names the level the controller will actually run.
///
/// `AdaptiveState::new` clamps to `[MIN_CHUNK_LEVEL, 7]`, but the summary was
/// built from the raw config value — so a controller configured below the floor
/// announced `chunk_level=0 (64KB)` while running at 2 (256KB). Operator-facing
/// text describing a state the process is not in is the same defect this branch
/// exists to remove, one layer down.
#[test]
fn startup_summary_reports_the_clamped_level() {
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let below_floor = AdaptiveConfig {
        initial_chunk_level: 0,
        ..AdaptiveConfig::default()
    };
    let _ctrl = AdaptiveController::new(
        1024 * 1024,
        below_floor,
        ControllerMode::HttpChunked,
        Some(Arc::new(RecordingCallback {
            logs: Arc::clone(&logged),
        })),
    );

    let summary = logged.lock().expect("lock")[0].clone();
    assert!(
        summary.contains(&format!("chunk_level={MIN_CHUNK_LEVEL}")),
        "the summary must name the clamped level, got: {summary}"
    );
    assert!(
        summary.contains(&format!(
            "({}KB)",
            CHUNK_LEVELS[MIN_CHUNK_LEVEL as usize] / 1024
        )),
        "and the byte size that goes with it, got: {summary}"
    );
}

/// The clamp holds at the ceiling too — and that half used to panic.
///
/// Above-ceiling companion to `startup_summary_reports_the_clamped_level`. The
/// pre-fix summary indexed `CHUNK_LEVELS` with the raw config value, so a level
/// past the array's end aborted the process before the clamp could apply. This
/// pins the case the fix's own comment cites; it fails by panic, not assertion,
/// against the unpatched code.
#[test]
fn startup_summary_clamps_above_the_ceiling() {
    let logged = Arc::new(Mutex::new(Vec::<String>::new()));
    let above_ceiling = AdaptiveConfig {
        initial_chunk_level: 9,
        ..AdaptiveConfig::default()
    };
    let _ctrl = AdaptiveController::new(
        1024 * 1024,
        above_ceiling,
        ControllerMode::HttpChunked,
        Some(Arc::new(RecordingCallback {
            logs: Arc::clone(&logged),
        })),
    );

    let summary = logged.lock().expect("lock")[0].clone();
    assert!(
        summary.contains("chunk_level=7"),
        "the summary must name the clamped ceiling, got: {summary}"
    );
    assert!(
        summary.contains(&format!("({}KB)", CHUNK_LEVELS[7] / 1024)),
        "and the byte size that goes with it, got: {summary}"
    );
}
