//! FinalizeMetadataStage — post-download probe to backfill or correct
//! `info_dict` fields against the ground-truth bytes on disk.
//!
//! Runs LAST in the pipeline (after FixupStage), so it sees the final
//! container — including any repair, remux, or recode the prior stages
//! applied. Calls `FFmpegRunner::probe()` (a libavformat header read,
//! not a subprocess) on the primary file, then:
//!
//! - **Patches `msg.info.duration`** when probe returns a value AND the
//!   existing duration is `None` OR differs by more than 5% (catches
//!   manifest lies — godresource's playlist had no duration entry; the
//!   site's player UI claimed 2:01:38 against the actual 1h39m56s).
//! - **Logs and pushes warnings for width / height / fps / video_codec /
//!   audio_codec** so GUI and CLI consumers can see the ground-truth values
//!   (audit finding M7: previously only logged, not visible in `msg.warnings`).
//!
//! **Non-fatal.** A probe failure here is informational only — the
//! file is already on disk and playable. We push a warning into
//! `msg.warnings` and pass through unchanged.
//!
//! **No config gate (yet).** Probe overhead on the typical output
//! container is a header read measured in the low ms even for
//! multi-GB files; not worth a flag until someone reports it. If
//! that changes, add `PostProcess.finalize_metadata: bool` defaulting
//! to true and wire it through `should_run`.

use std::sync::Arc;

use async_trait::async_trait;
use log::{info, warn};

use rdlp_ffmpeg::{FFmpegRunner, MediaInfo};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Threshold for considering the manifest's duration "approximately
/// equal" to the probe's. Anything within 5% is treated as the
/// extractor being honest within rounding noise; anything outside
/// gets the probe value as truth.
const DURATION_DRIFT_THRESHOLD: f64 = 0.05;

/// Probe the final output and patch `info_dict.duration` from the
/// container's actual stream metadata. Appends codec/resolution/fps
/// ground-truth values to `msg.warnings` for downstream visibility.
pub struct FinalizeMetadataStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FinalizeMetadataStage {
    /// Create a new `FinalizeMetadataStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }
}

#[async_trait]
impl PipelineStage for FinalizeMetadataStage {
    fn name(&self) -> &str {
        "FinalizeMetadataStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        // Always runs when there's a file to probe. Postprocess
        // pipeline guarantees `current_files` is non-empty for any
        // stage that runs after Merge, but be defensive.
        !msg.tracker.current_files.is_empty()
    }

    fn is_fatal(&self) -> bool {
        false
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        // Announce stage to UI via callback factory (consistent with
        // FixupStage).
        let _stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));

        let primary = msg.tracker.primary();
        info!("FinalizeMetadataStage: probing {}", primary.display());

        let media_info = match self.ffmpeg.probe(&primary).await {
            Ok(m) => m,
            Err(e) => {
                warn!("FinalizeMetadataStage: probe failed: {e}");
                msg.warnings
                    .push(format!("Finalize metadata probe failed: {e}"));
                return Ok(msg);
            }
        };

        // ── duration ──────────────────────────────────────────────
        if let Some(probed) = media_info.duration {
            match msg.info.duration {
                None => {
                    info!(
                        "FinalizeMetadataStage: backfilling duration {probed:.2}s \
                         (manifest had none)"
                    );
                    msg.info.duration = Some(probed);
                }
                Some(claimed) if duration_drifts(claimed, probed) => {
                    info!(
                        "FinalizeMetadataStage: correcting duration \
                         {claimed:.2}s → {probed:.2}s (manifest drift > {pct}%)",
                        pct = (DURATION_DRIFT_THRESHOLD * 100.0) as u32,
                    );
                    msg.info.duration = Some(probed);
                }
                _ => {}
            }
        }

        // ── codec / resolution / fps ──────────────────────────────
        // InfoDict doesn't carry these fields today (they live per-Format
        // and the Format is already consumed by the time postprocess
        // starts). Surface them via `msg.warnings` so the event pipeline,
        // GUI, and CLI consumers can display ground-truth values
        // (audit finding M7: previously only logged, invisible to callers).
        apply_probe_media_info(&media_info, &mut msg);

        Ok(msg)
    }
}

/// Apply width/height/fps/codec information from a probe result to the message.
///
/// Logs each discovered value as INFO and appends it to `msg.warnings` so the
/// information is visible to GUI and CLI consumers (audit finding M7).
///
/// Separated from `process()` so it can be unit-tested without a real
/// `FFmpegRunner` or media file.
pub(crate) fn apply_probe_media_info(probe: &MediaInfo, msg: &mut PipelineMessage) {
    if let (Some(w), Some(h)) = (probe.width, probe.height) {
        let note = format!("FinalizeMetadata: file resolution {w}x{h}");
        info!("{note}");
        msg.warnings.push(note);
    }
    if let Some(fps) = probe.fps {
        let note = format!("FinalizeMetadata: file fps {fps:.3}");
        info!("{note}");
        msg.warnings.push(note);
    }
    if let Some(ref vc) = probe.video_codec {
        let note = format!("FinalizeMetadata: video codec {vc}");
        info!("{note}");
        msg.warnings.push(note);
    }
    if let Some(ref ac) = probe.audio_codec {
        let note = format!("FinalizeMetadata: audio codec {ac}");
        info!("{note}");
        msg.warnings.push(note);
    }
}

/// `true` when `claimed` differs from `probed` by more than
/// [`DURATION_DRIFT_THRESHOLD`]. Both values are in seconds.
///
/// Symmetric — uses `probed` as the denominator (it's the truth).
/// A claimed=0/probed=N case yields `true` (anything is infinitely
/// far from zero), which is the desired behaviour: a manifest that
/// claims zero duration when the file has any is plainly wrong.
fn duration_drifts(claimed: f64, probed: f64) -> bool {
    if probed == 0.0 {
        return claimed != 0.0;
    }
    ((claimed - probed) / probed).abs() > DURATION_DRIFT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use rdlp_ffmpeg::MediaInfo;
    use rdlp_types::{InfoDict, PostProcess};

    use crate::pipeline::{FileTracker, PipelineMessage, TempRegistry};

    fn make_msg() -> PipelineMessage {
        let info = InfoDict::new(
            "id".to_string(),
            "Test".to_string(),
            "TestExtractor".to_string(),
            "https://example.com/v".to_string(),
        );
        let reg = Arc::new(TempRegistry::new());
        let tracker = FileTracker::new(vec![PathBuf::from("/tmp/test.mp4")], reg);
        PipelineMessage {
            info,
            tracker,
            config: Arc::new(PostProcess::default()),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            error_tx: None,
            warnings: Vec::new(),
            encoding_tool: None,
        }
    }

    // ── M7 regression: apply_probe_media_info pushes warnings ───────────────
    //
    // Before the fix, probe results were only logged (INFO). They were invisible
    // to callers consuming `msg.warnings`. These tests verify that each
    // discovered media property adds an entry to `msg.warnings`.

    #[test]
    fn probe_resolution_appended_to_warnings() {
        let mut msg = make_msg();
        let probe = MediaInfo {
            width: Some(1920),
            height: Some(1080),
            ..Default::default()
        };
        apply_probe_media_info(&probe, &mut msg);
        assert!(
            msg.warnings.iter().any(|w| w.contains("1920x1080")),
            "warnings must contain resolution; got: {:?}",
            msg.warnings
        );
    }

    #[test]
    fn probe_fps_appended_to_warnings() {
        let mut msg = make_msg();
        let probe = MediaInfo {
            fps: Some(29.97),
            ..Default::default()
        };
        apply_probe_media_info(&probe, &mut msg);
        assert!(
            msg.warnings.iter().any(|w| w.contains("fps")),
            "warnings must contain fps entry; got: {:?}",
            msg.warnings
        );
    }

    #[test]
    fn probe_video_codec_appended_to_warnings() {
        let mut msg = make_msg();
        let probe = MediaInfo {
            video_codec: Some("h264".to_string()),
            ..Default::default()
        };
        apply_probe_media_info(&probe, &mut msg);
        assert!(
            msg.warnings.iter().any(|w| w.contains("h264")),
            "warnings must contain video codec; got: {:?}",
            msg.warnings
        );
    }

    #[test]
    fn probe_audio_codec_appended_to_warnings() {
        let mut msg = make_msg();
        let probe = MediaInfo {
            audio_codec: Some("aac".to_string()),
            ..Default::default()
        };
        apply_probe_media_info(&probe, &mut msg);
        assert!(
            msg.warnings.iter().any(|w| w.contains("aac")),
            "warnings must contain audio codec; got: {:?}",
            msg.warnings
        );
    }

    #[test]
    fn probe_empty_media_info_adds_no_warnings() {
        let mut msg = make_msg();
        let probe = MediaInfo::default();
        apply_probe_media_info(&probe, &mut msg);
        assert!(
            msg.warnings.is_empty(),
            "no warnings expected for empty probe; got: {:?}",
            msg.warnings
        );
    }

    // ── duration_drifts unit tests ───────────────────────────────────────────

    #[test]
    fn duration_within_threshold_is_not_drift() {
        // 99 min vs 100 min = 1% — below threshold.
        assert!(!duration_drifts(5940.0, 6000.0));
        // Identical values.
        assert!(!duration_drifts(3600.0, 3600.0));
        // 4.9% drift, below the 5% threshold.
        assert!(!duration_drifts(951.0, 1000.0));
    }

    #[test]
    fn duration_above_threshold_is_drift() {
        // godresource case: site claimed 2:01:38 (7298s) vs actual
        // 99:56 (5996s) — 21.7% drift.
        assert!(duration_drifts(7298.0, 5996.0));
        // 6% drift — just over threshold.
        assert!(duration_drifts(940.0, 1000.0));
    }

    #[test]
    fn zero_probed_with_nonzero_claimed_is_drift() {
        assert!(duration_drifts(100.0, 0.0));
    }

    #[test]
    fn zero_probed_with_zero_claimed_is_not_drift() {
        assert!(!duration_drifts(0.0, 0.0));
    }

    #[test]
    fn drift_check_is_symmetric_about_truth() {
        // Probed truth is the denominator. 10% over and 10% under
        // both register as drift.
        assert!(duration_drifts(1100.0, 1000.0));
        assert!(duration_drifts(900.0, 1000.0));
    }
}
