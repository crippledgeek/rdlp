//! Tests for transcode pipeline components.

#![allow(clippy::cast_possible_truncation)] // i128→i64 in sample_clock_rescale: values are within DTS range

use super::mux_timing::{MuxTimingState, fix_audio_timestamps};

#[test]
fn test_fix_audio_timestamps_normal() {
    // Increasing sequence passes through unchanged
    let timing = MuxTimingState::default();
    let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), 21, &timing);
    assert_eq!(d, Some(10));
    assert_eq!(p, Some(10));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(10));

    let timing = MuxTimingState {
        last_dts: Some(10),
        last_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(20), Some(20), 21, &timing);
    assert_eq!(d, Some(20));
    assert_eq!(p, Some(20));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(20));
}

#[test]
fn test_fix_audio_timestamps_duplicate() {
    // Same DTS clamped to prev + expected_duration (or dur.max(1) if no expected)
    let timing = MuxTimingState {
        last_dts: Some(10),
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), 21, &timing);
    assert_eq!(d, Some(31)); // 10 + dur=21
    assert_eq!(p, Some(31));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(31));
}

#[test]
fn test_fix_audio_timestamps_regression() {
    // Backwards DTS clamped to prev + expected_duration
    let timing = MuxTimingState {
        last_dts: Some(10),
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(5), Some(5), 21, &timing);
    assert_eq!(d, Some(31)); // 10 + 21
    assert_eq!(p, Some(31));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(31));
}

#[test]
fn test_fix_audio_timestamps_pts_correction() {
    // PTS < corrected DTS gets bumped
    let timing = MuxTimingState {
        last_dts: Some(10),
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(8), 21, &timing);
    assert_eq!(d, Some(31));
    assert_eq!(p, Some(31)); // PTS bumped from 8 to 31
    assert_eq!(dur, 21);
    assert_eq!(last, Some(31));
}

#[test]
fn test_fix_audio_timestamps_none() {
    // None DTS passes through, last_dts unchanged
    let timing = MuxTimingState {
        last_dts: Some(10),
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(None, Some(42), 21, &timing);
    assert_eq!(d, None);
    assert_eq!(p, Some(42));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(10));

    let timing = MuxTimingState::default();
    let (d, p, dur, last) = fix_audio_timestamps(None, None, 21, &timing);
    assert_eq!(d, None);
    assert_eq!(p, None);
    assert_eq!(dur, 21);
    assert_eq!(last, None);
}

#[test]
fn test_fix_audio_timestamps_pts_ge_dts_no_correction() {
    // Even without DTS correction, ensure pts >= dts
    let timing = MuxTimingState {
        last_dts: Some(10),
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(20), Some(15), 21, &timing);
    assert_eq!(d, Some(20));
    assert_eq!(p, Some(20)); // PTS bumped from 15 to 20
    assert_eq!(dur, 21);
    assert_eq!(last, Some(20));
}

#[test]
fn test_fix_audio_timestamps_cross_call_persistence() {
    // Simulates the cross-call scenario: call 1 ends with last_dts=100,
    // call 2 starts with dts=100 -> must correct.
    let timing = MuxTimingState {
        last_dts: Some(100),
        expected_duration: 21,
        pkt_count: 50,
        ..Default::default()
    };
    let (d, p, dur, updated) = fix_audio_timestamps(Some(100), Some(100), 21, &timing);
    assert_eq!(d, Some(121)); // 100 + 21
    assert_eq!(p, Some(121));
    assert_eq!(dur, 21);
    assert_eq!(updated, Some(121));
}

#[test]
fn test_fix_audio_timestamps_pts_none_gets_set() {
    // dts=Some(50), pts=None -> pts=Some(50)
    let timing = MuxTimingState::default();
    let (d, p, dur, last) = fix_audio_timestamps(Some(50), None, 21, &timing);
    assert_eq!(d, Some(50));
    assert_eq!(p, Some(50));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(50));
}

#[test]
fn test_fix_audio_timestamps_pts_none_with_correction() {
    // dts=Some(5), pts=None, last_dts=Some(10) -> corrected
    let timing = MuxTimingState {
        last_dts: Some(10),
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(5), None, 21, &timing);
    assert_eq!(d, Some(31)); // 10 + 21
    assert_eq!(p, Some(31));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(31));
}

#[test]
fn test_fix_audio_timestamps_expected_duration_step() {
    // AAC at 48kHz in 1/1000 tb: expected_duration=21
    let timing = MuxTimingState {
        last_dts: Some(105),
        last_duration: 21,
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(100), Some(100), 21, &timing);
    assert_eq!(d, Some(126)); // 105 + 21
    assert_eq!(p, Some(126));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(126));
}

#[test]
fn test_fix_audio_timestamps_zero_duration_fixed() {
    // Zero duration gets fixed from expected_duration
    let timing = MuxTimingState {
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(0), Some(0), 0, &timing);
    assert_eq!(d, Some(0));
    assert_eq!(p, Some(0));
    assert_eq!(dur, 21); // Fixed from 0 to expected
    assert_eq!(last, Some(0));
}

#[test]
fn test_fix_audio_timestamps_normal_progression() {
    // Normal progression with expected_duration: no correction needed
    let timing = MuxTimingState {
        last_dts: Some(21),
        last_duration: 21,
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(42), Some(42), 21, &timing);
    assert_eq!(d, Some(42)); // No correction needed
    assert_eq!(p, Some(42));
    assert_eq!(dur, 21);
    assert_eq!(last, Some(42));
}

#[test]
fn test_fix_audio_timestamps_zero_duration_no_expected() {
    // Zero duration, no expected_duration -> fallback to last_duration or 1
    let timing = MuxTimingState {
        last_dts: Some(10),
        last_duration: 0,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), 0, &timing);
    // duration fixed to max(last_duration=0, 1) = 1
    assert_eq!(dur, 1);
    // DTS corrected: 10 + dur.max(1)=1 = 11 (no expected_duration)
    assert_eq!(d, Some(11));
    assert_eq!(p, Some(11));
    assert_eq!(last, Some(11));
}

#[test]
fn test_fix_audio_timestamps_negative_duration_fallback() {
    // Negative duration -> fixed via expected_duration
    let timing = MuxTimingState {
        expected_duration: 21,
        ..Default::default()
    };
    let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), -5, &timing);
    assert_eq!(dur, 21); // Fixed from -5 to expected=21
    assert_eq!(d, Some(10));
    assert_eq!(p, Some(10));
    assert_eq!(last, Some(10));
}

#[test]
fn test_mux_timing_state_default() {
    let timing = MuxTimingState::default();
    assert_eq!(timing.last_dts, None);
    assert_eq!(timing.last_duration, 0);
    assert_eq!(timing.expected_duration, 0);
    assert_eq!(timing.pkt_count, 0);
    assert_eq!(timing.encoder_frame_size, 0);
    assert_eq!(timing.last_pos_check, 0);
    assert_eq!(timing.stall_count, 0);
    assert_eq!(timing.last_file_size, 0);
    assert_eq!(timing.samples_written, 0);
    assert_eq!(timing.sample_rate, 0);
    assert!(!timing.use_sample_clock);
}

#[test]
fn test_mux_timing_state_stall_tracking() {
    let mut timing = MuxTimingState {
        last_pos_check: 1000,
        stall_count: 2,
        ..Default::default()
    };
    // pos advanced
    let new_pos: i64 = 2000;
    if new_pos > timing.last_pos_check {
        timing.last_pos_check = new_pos;
        timing.stall_count = 0;
    }
    assert_eq!(timing.last_pos_check, 2000);
    assert_eq!(timing.stall_count, 0);

    // Simulate pos NOT advancing -- stall_count increments
    let same_pos: i64 = 2000;
    if same_pos > timing.last_pos_check {
        timing.last_pos_check = same_pos;
        timing.stall_count = 0;
    } else {
        timing.stall_count += 1;
    }
    assert_eq!(timing.stall_count, 1);

    // Two more stalls reach threshold
    timing.stall_count += 1;
    timing.stall_count += 1;
    assert_eq!(timing.stall_count, 3);
    assert!(timing.stall_count >= 3, "stall threshold reached");
}

#[test]
fn test_fix_audio_timestamps_with_sample_clock_noop() {
    // When timestamps are already monotonic from sample-clock,
    // fix_audio_timestamps should pass them through unchanged.
    let mut timing = MuxTimingState {
        expected_duration: 21,
        use_sample_clock: true,
        sample_rate: 48000,
        encoder_frame_size: 1024,
        ..Default::default()
    };

    // Simulate 3 packets with perfectly monotonic timestamps
    for i in 0..3 {
        let dts = i * 21;
        let (d, p, dur, last) = fix_audio_timestamps(Some(dts), Some(dts), 21, &timing);
        assert_eq!(d, Some(dts), "pkt {i}: dts passthrough");
        assert_eq!(p, Some(dts), "pkt {i}: pts passthrough");
        assert_eq!(dur, 21, "pkt {i}: dur passthrough");
        assert_eq!(last, Some(dts), "pkt {i}: last_dts updated");
        timing.last_dts = last;
        timing.last_duration = dur;
    }
}

/// Helper: compute expected DTS/duration for sample-clock synthesis
/// using `av_rescale_q(samples, 1/sample_rate, ost_tb)`.
fn sample_clock_rescale(samples: i64, sample_rate: i32, ost_tb_num: i32, ost_tb_den: i32) -> i64 {
    // av_rescale_q(a, bq, cq) = a * bq.num * cq.den / (bq.den * cq.num)
    // bq = {1, sample_rate}, cq = {ost_tb_num, ost_tb_den}
    // = samples * 1 * ost_tb_den / (sample_rate * ost_tb_num)
    let num = i128::from(samples) * i128::from(ost_tb_den);
    let den = i128::from(sample_rate) * i128::from(ost_tb_num);
    // Round to nearest (matching av_rescale_q behavior)
    ((num + den / 2) / den) as i64
}

#[test]
fn test_sample_clock_aac_48000() {
    // AAC: 1024 samples at 48000 Hz, ost_tb=1/1000
    let sr = 48000;
    let frame = 1024i64;

    let dur = sample_clock_rescale(frame, sr, 1, 1000);
    assert_eq!(dur, 21, "AAC 48kHz dur in 1/1000 tb");

    let dts0 = sample_clock_rescale(0, sr, 1, 1000);
    assert_eq!(dts0, 0);
    let dts1 = sample_clock_rescale(frame, sr, 1, 1000);
    assert_eq!(dts1, 21);
    let dts2 = sample_clock_rescale(frame * 2, sr, 1, 1000);
    assert_eq!(dts2, 43); // 2048/48000*1000 = 42.666... -> 43
    let dts3 = sample_clock_rescale(frame * 3, sr, 1, 1000);
    assert_eq!(dts3, 64); // 3072/48000*1000 = 64.0

    // Monotonic
    assert!(dts1 > dts0);
    assert!(dts2 > dts1);
    assert!(dts3 > dts2);
}

#[test]
fn test_sample_clock_opus_48000() {
    // Opus: 960 samples at 48000 Hz, ost_tb=1/1000
    let sr = 48000;
    let frame = 960i64;

    let dur = sample_clock_rescale(frame, sr, 1, 1000);
    assert_eq!(dur, 20, "Opus 48kHz dur in 1/1000 tb");

    let dts0 = sample_clock_rescale(0, sr, 1, 1000);
    assert_eq!(dts0, 0);
    let dts1 = sample_clock_rescale(frame, sr, 1, 1000);
    assert_eq!(dts1, 20);
    let dts2 = sample_clock_rescale(frame * 2, sr, 1, 1000);
    assert_eq!(dts2, 40);
    let dts3 = sample_clock_rescale(frame * 3, sr, 1, 1000);
    assert_eq!(dts3, 60);

    assert!(dts1 > dts0);
    assert!(dts2 > dts1);
    assert!(dts3 > dts2);
}

#[test]
fn test_sample_clock_aac_44100() {
    // AAC: 1024 samples at 44100 Hz, ost_tb=1/1000
    let sr = 44100;
    let frame = 1024i64;

    let dur = sample_clock_rescale(frame, sr, 1, 1000);
    assert_eq!(dur, 23, "AAC 44100Hz dur in 1/1000 tb"); // 1024/44100*1000 = 23.21... -> 23

    let dts0 = sample_clock_rescale(0, sr, 1, 1000);
    assert_eq!(dts0, 0);
    let dts1 = sample_clock_rescale(frame, sr, 1, 1000);
    assert_eq!(dts1, 23);
    let dts2 = sample_clock_rescale(frame * 2, sr, 1, 1000);
    assert_eq!(dts2, 46);
    let dts3 = sample_clock_rescale(frame * 3, sr, 1, 1000);
    assert_eq!(dts3, 70); // 3072/44100*1000 = 69.65... -> 70

    assert!(dts1 > dts0);
    assert!(dts2 > dts1);
    assert!(dts3 > dts2);
}
