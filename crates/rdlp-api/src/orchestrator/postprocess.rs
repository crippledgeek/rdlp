//! Post-processing pipeline for downloaded files
//!
//! Handles `FFmpeg` remuxing, metadata embedding, and audio extraction.

use super::{Orchestrator, Result};
use crate::events::Event;
use crate::handle::DownloadId;
use crate::orchestrator::errors::OrchestratorError;
use crate::orchestrator::eta::EtaEstimator;
use log::{debug, error, warn};
use rdlp_core::{PostProcessCallback, PostProcessCallbackFactory};
use rdlp_postprocess::PipelineRunOptions;
use rdlp_postprocess::pipeline::PipelineError;
use rdlp_types::Progress;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Bridges post-processor progress into [`Event::PostProcessProgress`] events.
///
/// One instance is created per post-processing stage. Progress values in
/// `[0.0, 1.0]` are forwarded to the event channel via `try_send`.
struct PostProcessBridge {
    event_tx: mpsc::Sender<Event>,
    download_id: DownloadId,
    stage: String,
    eta: EtaEstimator,
}

impl PostProcessCallback for PostProcessBridge {
    fn on_progress(&self, progress: Progress) {
        let eta = self.eta.update(f64::from(progress.fraction()));
        let _ = self.event_tx.try_send(Event::PostProcessProgress {
            id: self.download_id,
            stage: self.stage.clone(),
            progress,
            eta,
        });
    }

    fn on_log(&self, message: &str) {
        let _ = self.event_tx.try_send(Event::Debug {
            id: self.download_id,
            message: message.to_owned(),
        });
    }
}

/// Build a [`PostProcessCallbackFactory`] that emits progress events for the
/// given download.
///
/// The factory is called once per post-processing stage with the stage name,
/// returning a fresh bridge for that stage. It also emits an
/// [`Event::PostProcessing`] when each stage starts so the frontend can
/// display stage names in the log panel.
fn make_callback_factory(
    event_tx: mpsc::Sender<Event>,
    download_id: DownloadId,
) -> PostProcessCallbackFactory {
    Arc::new(move |stage_name: &str| -> Arc<dyn PostProcessCallback> {
        // Notify the frontend that a new post-processing stage has started.
        let _ = event_tx.try_send(Event::PostProcessing {
            id: download_id,
            stage: stage_name.to_owned(),
        });
        Arc::new(PostProcessBridge {
            event_tx: event_tx.clone(),
            download_id,
            stage: stage_name.to_owned(),
            eta: EtaEstimator::new(),
        })
    })
}

/// Clean stem for sidecar (thumbnail/subtitle) discovery: the first file's name
/// with any temp marker stripped, so a `.rdlp-tmp-{uuid}` / `.rdlp-part` pipeline
/// input still resolves the originally-named sidecars (#406 slice 2).
fn original_stem_for(files: &[PathBuf]) -> String {
    files
        .first()
        .and_then(|f| f.file_name())
        .and_then(|n| n.to_str())
        .map_or_else(
            || "video".to_owned(),
            |name| {
                super::naming::strip_temp_marker(name).map_or_else(
                    || {
                        std::path::Path::new(name)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("video")
                            .to_owned()
                    },
                    ToOwned::to_owned,
                )
            },
        )
}

/// Classify a pipeline error as either a user-cancel or a fatal failure.
///
/// This is a pure function extracted for testability. The cancel vs.
/// non-cancel distinction is the only branching: a [`PipelineError::Cancelled`]
/// becomes [`OrchestratorError::UserCancelled`] so the UI surfaces a deliberate
/// cancel correctly; every other error becomes
/// [`OrchestratorError::PostProcessingFailed`] so it propagates as
/// [`Event::Failed`] rather than being silently swallowed.
pub fn classify_pipeline_err(e: &anyhow::Error) -> OrchestratorError {
    if matches!(
        e.downcast_ref::<PipelineError>(),
        Some(PipelineError::Cancelled)
    ) {
        OrchestratorError::UserCancelled
    } else {
        error!("Post-processing pipeline failed: {e:#}");
        OrchestratorError::PostProcessingFailed(format!("{e:#}"))
    }
}

impl Orchestrator {
    /// Check if post-processing is needed based on configuration
    pub(super) fn needs_postprocessing(&self) -> bool {
        self.postprocessing_alters_the_media()
            || self.config.postprocess.embed_metadata
            || self.config.postprocess.embed_thumbnail
            || self.config.postprocess.embed_subtitles
            || self.config.postprocess.fixup != rdlp_types::FixupPolicy::Never
    }

    /// Whether post-processing would change the media itself, rather than
    /// decorate it.
    ///
    /// One half of the predicate that decides whether an unusable `FFmpeg`
    /// fails a run (rdlp#727) — the other half is the plan's own shape, in
    /// [`DownloadPlan::requires_ffmpeg`](super::DownloadPlan::requires_ffmpeg). Drawn by consequence rather than by
    /// which options happen to be set: skipping a remux, a recode, an audio
    /// extract or a normalize yields a file that is not what was asked for —
    /// the wrong container, the wrong codec, no audio track. Skipping an
    /// embedded thumbnail, a metadata tag or embedded subtitles yields the
    /// right media with something missing around it, which the warning in
    /// `run_postprocessing` covers. `fixup` sits with those not because it
    /// cannot change the media — `DetectOrWarn` repairs a broken file — but
    /// because it is a conditional repair that is on by default, so treating
    /// it as a request would refuse every unconfigured download.
    ///
    /// Config defaults are why the line cannot be "did the user ask": both
    /// `embed_thumbnail` and `fixup` are on in [`Config::default`], so a
    /// request-shaped test would call every unconfigured run a request and
    /// refuse it.
    pub(super) fn postprocessing_alters_the_media(&self) -> bool {
        self.config.postprocess.extract_audio
            || self.config.postprocess.recode_video.is_some()
            || self.config.postprocess.recode_container.is_some()
            || self.config.postprocess.remux_container.is_some()
            || self.config.postprocess.normalize_audio
    }

    /// Run post-processing pipeline on downloaded file(s)
    ///
    /// # Arguments
    /// * `info` - Video metadata
    /// * `files` - Downloaded file paths
    /// * `is_hls` - Whether this was an HLS download (triggers automatic remux)
    /// * `keep_inputs` - When `true`, input files are borrowed (not owned) by the
    ///   pipeline: the originals are preserved on both success and cancel. Set for
    ///   user-supplied source files that must not be deleted (e.g. `process_local_file`);
    ///   `false` for files rdlp downloaded itself (the default everywhere else).
    ///
    /// # Returns
    /// * `Ok(paths)` - Processed file paths (may differ from input if conversion occurred)
    /// * `Err(e)` - Post-processing failed
    pub(crate) async fn run_postprocessing(
        &self,
        info: &rdlp_types::InfoDict,
        files: Vec<PathBuf>,
        is_hls: bool,
        keep_inputs: bool,
    ) -> Result<Vec<PathBuf>> {
        debug!(
            "[PostProcess] Called: is_hls={is_hls}, pipeline={:?}",
            self.pipeline
        );

        let Some(pipeline) = self.pipeline.pipeline() else {
            let needed = self.needs_postprocessing() || is_hls;

            // An ABI-skewed FFmpeg is installed and refusing to be called.
            //
            // A run that needed it was already refused before the download
            // started (`refuse_plan_needing_unusable_ffmpeg`), so what reaches
            // here either needed nothing or holds bytes on disk. Those bytes
            // are why this branch does not fail when it owns them: the caller
            // finalizes the clean name only on the `Ok` path, so returning
            // `Err` here would abandon a complete download under its
            // `.rdlp-tmp-` seam name for `cleanup_stale` to delete. Borrowed
            // inputs (`keep_inputs`, e.g. `process_local_file`) are the user's
            // own files and are never ours to lose, so that path can fail
            // honestly.
            if let Some(mismatches) = self.pipeline.abi_mismatch() {
                if keep_inputs && self.postprocessing_alters_the_media() {
                    return Err(OrchestratorError::FFmpegAbiMismatch(mismatches.clone()));
                }
                // More than one file means a merge, and the caller finalizes
                // only the first — so reaching here with a skewed FFmpeg would
                // hand back a video and abandon its audio. `GatedPlan` makes
                // that unreachable: a merge plan cannot enter the download on a
                // skewed machine. If it ever does, the gate has been defeated
                // and that is worth saying loudly, because the symptom on its
                // own looks like a successful download.
                if files.len() > 1 {
                    error!(
                        "BUG: a merge reached post-processing with an unusable FFmpeg; \
                         the pre-download gate was bypassed. Streams are left unmerged \
                         rather than one of them being presented as the result"
                    );
                }
                if needed {
                    warn!(
                        "Post-processing skipped: FFmpeg is installed but unusable (ABI mismatch). \
                         The download is complete and unprocessed; the FFmpeg ABI \
                         error logged earlier in this session carries the remedy"
                    );
                }
                return Ok(files);
            }

            // No FFmpeg at all — degrade, as rdlp always has.
            if needed {
                warn!("Post-processing unavailable (FFmpeg not found)");
                if is_hls {
                    warn!("HLS downloads may have container issues without FFmpeg remux");
                }
            }
            return Ok(files);
        };

        // For HLS downloads always run (RemuxStage handles TS → MP4 via is_hls flag).
        // For other downloads, only run if explicitly configured.
        let needs_processing = self.needs_postprocessing() || is_hls;
        if !needs_processing {
            return Ok(files);
        }

        let pp_config = self.config.postprocess.clone();

        debug!("Running post-processing pipeline...");

        let original_stem = original_stem_for(&files);

        // Build a per-stage progress callback factory.
        let callback_factory = Some(make_callback_factory(
            self.event_tx.clone(),
            self.download_id,
        ));

        match pipeline
            .run(
                info.clone(),
                files.clone(),
                PipelineRunOptions {
                    keep_inputs,
                    is_hls,
                    verbose: self.config.verbose,
                },
                Arc::new(pp_config),
                original_stem,
                callback_factory,
                Some(self.cancel_token.clone()),
            )
            .await
        {
            Ok(output_files) => {
                if output_files != files {
                    debug!("Post-processing complete");
                    if self.config.verbose {
                        for file in &output_files {
                            let msg = format!("Output: {}", file.display());
                            debug!("{msg}");
                            self.emit(Event::Debug {
                                id: self.download_id,
                                message: msg,
                            });
                        }
                    }
                }
                Ok(output_files)
            }
            Err(e) => Err(classify_pipeline_err(&e)),
        }
    }
}

#[cfg(test)]
mod classify_tests {
    use super::*;
    use rdlp_postprocess::pipeline::PipelineError;

    /// A [`PipelineError::Cancelled`] MUST classify as `UserCancelled`, not
    /// `PostProcessingFailed`. Regression guard: the cancel→Failed bug.
    #[test]
    fn cancelled_pipeline_error_maps_to_user_cancelled() {
        let err = anyhow::Error::new(PipelineError::Cancelled);
        let result = classify_pipeline_err(&err);
        assert!(
            matches!(result, OrchestratorError::UserCancelled),
            "PipelineError::Cancelled must map to UserCancelled, got {result:?}",
        );
    }

    /// Any non-cancel pipeline error MUST classify as `PostProcessingFailed`,
    /// NOT as `Ok(files)`. This pins the fix: before the change the non-cancel
    /// arm silently returned `Ok(files)` — the error was swallowed entirely so
    /// this test didn't exist (no classifier fn existed). A stage failure
    /// classified here would previously have been silently swallowed. Since
    /// #632 such a failure arrives as the stage's own `anyhow` chain rather
    /// than a typed variant, which is what this test constructs.
    #[test]
    fn non_cancel_pipeline_error_maps_to_postprocessing_failed() {
        let err = anyhow::anyhow!("stage failed: remux codec error");
        let result = classify_pipeline_err(&err);
        assert!(
            matches!(result, OrchestratorError::PostProcessingFailed(_)),
            "non-cancel error must map to PostProcessingFailed, got {result:?}",
        );
    }

    /// The error message is preserved in `PostProcessingFailed` so operators
    /// can read the root cause from `Event::Failed.error.user_message()`.
    #[test]
    fn postprocessing_failed_preserves_message() {
        let err = anyhow::anyhow!("codec unavailable");
        let result = classify_pipeline_err(&err);
        match result {
            OrchestratorError::PostProcessingFailed(msg) => {
                assert!(
                    msg.contains("codec unavailable"),
                    "message not preserved: {msg}",
                );
            }
            other => panic!("expected PostProcessingFailed, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod original_stem_tests {
    use super::*;

    #[test]
    fn original_stem_strips_temp_marker() {
        let files = vec![PathBuf::from("/v/My.Video.rdlp-tmp-abc.mp4")];
        assert_eq!(original_stem_for(&files), "My.Video");
    }

    #[test]
    fn original_stem_strips_part_marker() {
        let files = vec![PathBuf::from("/v/Clip.rdlp-part.ts")];
        assert_eq!(original_stem_for(&files), "Clip");
    }

    #[test]
    fn original_stem_plain_name_unchanged() {
        let files = vec![PathBuf::from("/v/My.Video.mp4")];
        assert_eq!(original_stem_for(&files), "My.Video");
    }

    #[test]
    fn original_stem_empty_defaults_to_video() {
        let files: Vec<PathBuf> = vec![];
        assert_eq!(original_stem_for(&files), "video");
    }
}
