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
//! - **Logs discrepancies for width / height / fps / video_codec /
//!   audio_codec** as `INFO` lines. InfoDict doesn't carry these fields
//!   today (they live per-Format and the Format is already consumed by
//!   the time postprocess starts), so logging is the value the stage
//!   delivers for those keys. Future scope: extend InfoDict + emit a
//!   new `Event::MetadataCorrected` so the GUI can re-render its
//!   "Quality" column post-download.
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

use rdlp_ffmpeg::FFmpegRunner;

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Threshold for considering the manifest's duration "approximately
/// equal" to the probe's. Anything within 5% is treated as the
/// extractor being honest within rounding noise; anything outside
/// gets the probe value as truth.
const DURATION_DRIFT_THRESHOLD: f64 = 0.05;

/// Probe the final output and patch `info_dict.duration` from the
/// container's actual stream metadata. Logs codec/resolution/fps
/// truths for downstream visibility.
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

        // ── codec / resolution / fps (log-only for now) ───────────
        // InfoDict doesn't carry these fields; surface them so the
        // CLI's --verbose log records the truth and operators have
        // a paper trail for "site claimed X, file is Y" disputes.
        if let (Some(w), Some(h)) = (media_info.width, media_info.height) {
            info!("FinalizeMetadataStage: file resolution {w}x{h}");
        }
        if let Some(fps) = media_info.fps {
            info!("FinalizeMetadataStage: file fps {fps:.3}");
        }
        if let Some(ref vc) = media_info.video_codec {
            info!("FinalizeMetadataStage: video codec {vc}");
        }
        if let Some(ref ac) = media_info.audio_codec {
            info!("FinalizeMetadataStage: audio codec {ac}");
        }

        Ok(msg)
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
