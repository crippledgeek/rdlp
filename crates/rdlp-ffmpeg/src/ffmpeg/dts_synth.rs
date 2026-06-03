//! Monotonic DTS synthesis for raw-FFI Matroska stream-copy paths.
//!
//! Matroska stores no decode timestamp (RFC 9559 §10/§11.2: a Block carries one
//! presentation timestamp). The Matroska demuxer therefore emits the first
//! `video_delay` packets of a B-frame stream with `dts == AV_NOPTS_VALUE`, and
//! raw/broken sources emit packets with no timestamps at all. libavformat's
//! muxer still requires a monotonically non-decreasing `dts` with `dts <= pts`;
//! otherwise matroskaenc logs "Timestamps are unset ... Fix your code" (once per
//! output) and, when both are unset, hard-fails ("Can't write packet with
//! unknown timestamp"). Verified against `FFmpeg` n8.1.1 `libavformat/mux.c`; the
//! warning becomes a hard `AVERROR(EINVAL)` at libavformat major 63.
//!
//! This mirrors libavformat's `compute_muxer_pkt_fields` reconstruction:
//! warmup dts = `first_pts + (i - delay) * duration` (backward-spaced, <= pts,
//! monotonic); `dts = pts` when `delay == 0`; carry-forward when pts is unset.

/// Reorder window cap, matching libavformat's `MAX_REORDER_DELAY`
/// (`libavformat/internal.h`). Streams reporting a larger `video_delay` are
/// treated as `MAX_REORDER_DELAY` for warmup spacing.
const MAX_REORDER_DELAY: i64 = 16;

/// Per-output-stream DTS synthesizer. `None` == `AV_NOPTS_VALUE`.
// `pub(crate)` is intentional (crate-internal API); clippy::redundant_pub_crate
// fires because the module is private, but widening to `pub` would leak it.
#[allow(clippy::redundant_pub_crate)]
pub(crate) struct DtsSynthesizer {
    /// B-frame reorder depth (`codecpar->video_delay`), clamped to `0..=MAX`.
    delay: i64,
    /// Index of the next packet (warmup window is `idx < delay`).
    idx: i64,
    /// PTS of the first packet, anchor for warmup back-spacing.
    first_pts: Option<i64>,
    /// Last emitted dts; guarantees strict monotonicity.
    last_dts: Option<i64>,
}

impl DtsSynthesizer {
    /// `video_delay` is `codecpar->video_delay` (0 for audio/subtitle/no-B-frame).
    pub(crate) fn new(video_delay: i32) -> Self {
        let delay = i64::from(video_delay).clamp(0, MAX_REORDER_DELAY);
        Self {
            delay,
            idx: 0,
            first_pts: None,
            last_dts: None,
        }
    }

    /// Returns the dts to write for this packet. `pts`/`dts` are `None` for
    /// `AV_NOPTS_VALUE`. `duration` is in the packet's (output) `time_base`;
    /// `<= 0` means unknown.
    pub(crate) fn next_dts(&mut self, pts: Option<i64>, dts: Option<i64>, duration: i64) -> i64 {
        let i = self.idx;
        self.idx += 1;
        // Anchor on the first pts-bearing packet's pts; never overwritten.
        self.first_pts = self.first_pts.or(pts);
        let step = if duration > 0 { duration } else { 1 };

        if let Some(d) = dts {
            return self.bump(d);
        }
        if let Some(p) = pts {
            if self.delay == 0 {
                return self.bump(p);
            }
            // Saturating arithmetic: timestamps originate from downloaded media
            // (attacker-influenced); a crafted pts/duration must not overflow.
            let anchor = self.first_pts.unwrap_or(p);
            let est = anchor
                .saturating_add((i - self.delay).saturating_mul(step))
                .min(p);
            return self.bump(est);
        }
        let est = self.last_dts.map_or(0, |l| l.saturating_add(step));
        self.bump(est)
    }

    /// Enforce strictly non-decreasing dts.
    const fn bump(&mut self, dts: i64) -> i64 {
        let out = match self.last_dts {
            Some(last) if dts <= last => last.saturating_add(1),
            _ => dts,
        };
        self.last_dts = Some(out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No B-frames (delay 0): dts == pts, monotonic.
    #[test]
    fn no_reorder_sets_dts_equal_pts() {
        let mut s = DtsSynthesizer::new(0);
        assert_eq!(s.next_dts(Some(0), None, 40), 0);
        assert_eq!(s.next_dts(Some(40), None, 40), 40);
        assert_eq!(s.next_dts(Some(80), None, 40), 80);
    }

    // B-frame warmup: reproduce FFmpeg's measured sequence -80,-40 then real dts.
    #[test]
    fn bframe_warmup_is_backward_spaced_then_passes_through() {
        let mut s = DtsSynthesizer::new(2); // video_delay = 2
        // Demuxer gives pts but NOPTS dts for the first `delay` packets:
        assert_eq!(s.next_dts(Some(0), None, 40), -80);
        assert_eq!(s.next_dts(Some(160), None, 40), -40);
        // Then the demuxer provides real, monotonic dts — pass through unchanged:
        assert_eq!(s.next_dts(Some(80), Some(0), 40), 0);
        assert_eq!(s.next_dts(Some(40), Some(40), 40), 40);
        assert_eq!(s.next_dts(Some(120), Some(80), 40), 80);
    }

    // Both timestamps unset (raw-bitstream pathology): carry forward, monotonic.
    #[test]
    fn both_unset_carries_forward_monotonically() {
        let mut s = DtsSynthesizer::new(0);
        assert_eq!(s.next_dts(None, None, 40), 0);
        assert_eq!(s.next_dts(None, None, 40), 40);
        assert_eq!(s.next_dts(None, None, 0), 41); // unknown dur -> +1 floor
    }

    // Monotonicity is always enforced even against a non-monotonic real dts.
    #[test]
    fn enforces_monotonic_dts() {
        let mut s = DtsSynthesizer::new(0);
        assert_eq!(s.next_dts(Some(100), Some(100), 40), 100);
        assert_eq!(s.next_dts(Some(50), Some(50), 40), 101); // bumped, never decreases
    }

    // dts must never exceed pts (mux.c errors on pts < dts).
    #[test]
    fn never_exceeds_pts() {
        let mut s = DtsSynthesizer::new(0);
        let d = s.next_dts(Some(10), None, 40);
        assert!(d <= 10, "dts {d} must be <= pts 10");
    }

    // Across a full B-frame stream (warmup + demuxer-supplied dts), every
    // emitted dts must stay strictly monotonic AND <= its packet's pts —
    // the two invariants matroskaenc requires. Exercises the warmup path under
    // realistic reorder pressure (delay=2, IBBP-style non-monotonic pts).
    #[test]
    fn warmup_then_steady_stays_monotonic_and_le_pts() {
        let mut s = DtsSynthesizer::new(2);
        // (pts, dts) as a Matroska demuxer would emit: dts unset during warmup.
        let stream = [
            (0_i64, None),
            (160, None),
            (80, Some(0_i64)),
            (40, Some(40)),
            (120, Some(80)),
            (320, Some(120)),
            (240, Some(160)),
            (200, Some(200)),
        ];
        let mut last = i64::MIN;
        for (pts, dts) in stream {
            let d = s.next_dts(Some(pts), dts, 40);
            assert!(d > last, "dts {d} not strictly increasing after {last}");
            assert!(d <= pts, "dts {d} exceeds pts {pts}");
            last = d;
        }
    }
}
