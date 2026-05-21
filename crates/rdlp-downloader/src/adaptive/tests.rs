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
