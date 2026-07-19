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
        self.config.postprocess.extract_audio
            || self.config.postprocess.embed_metadata
            || self.config.postprocess.embed_thumbnail
            || self.config.postprocess.embed_subtitles
            || self.config.postprocess.recode_video.is_some()
            || self.config.postprocess.recode_container.is_some()
            || self.config.postprocess.remux_container.is_some()
            || self.config.postprocess.normalize_audio
            || self.config.postprocess.fixup != rdlp_types::FixupPolicy::Never
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
            "[PostProcess] Called: is_hls={is_hls}, pipeline={}",
            self.pipeline.is_some()
        );

        let Some(pipeline) = &self.pipeline else {
            // No pipeline available — return files unchanged.
            if self.needs_postprocessing() || is_hls {
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
    /// this test didn't exist (no classifier fn existed). A `StageFailure` error
    /// classified here would previously have been silently swallowed.
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
