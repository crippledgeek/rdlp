//! Post-processing pipeline for downloaded files
//!
//! Handles FFmpeg remuxing, metadata embedding, and audio extraction.

use super::{Orchestrator, Result};
use crate::events::Event;
use crate::handle::DownloadId;
use log::{debug, warn};
use rdlp_core::{PostProcessCallback, PostProcessCallbackFactory};
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
}

impl PostProcessCallback for PostProcessBridge {
    fn on_progress(&self, progress: Progress) {
        let _ = self.event_tx.try_send(Event::PostProcessProgress {
            id: self.download_id,
            stage: self.stage.clone(),
            progress,
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
        })
    })
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
    ///
    /// # Returns
    /// * `Ok(paths)` - Processed file paths (may differ from input if conversion occurred)
    /// * `Err(e)` - Post-processing failed
    pub(crate) async fn run_postprocessing(
        &self,
        info: &rdlp_types::InfoDict,
        files: Vec<PathBuf>,
        is_hls: bool,
    ) -> Result<Vec<PathBuf>> {
        debug!(
            "[PostProcess] Called: is_hls={is_hls}, pipeline={}",
            self.pipeline.is_some()
        );

        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => {
                // No pipeline available — return files unchanged.
                if self.needs_postprocessing() || is_hls {
                    warn!("Post-processing unavailable (FFmpeg not found)");
                    if is_hls {
                        warn!("HLS downloads may have container issues without FFmpeg remux");
                    }
                }
                return Ok(files);
            }
        };

        // For HLS downloads always run (RemuxStage handles TS → MP4 via is_hls flag).
        // For other downloads, only run if explicitly configured.
        let needs_processing = self.needs_postprocessing() || is_hls;
        if !needs_processing {
            return Ok(files);
        }

        let pp_config = self.config.postprocess.clone();

        debug!("Running post-processing pipeline...");

        let original_stem = files
            .first()
            .and_then(|f| f.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("video")
            .to_owned();

        // Build a per-stage progress callback factory.
        let callback_factory = Some(make_callback_factory(
            self.event_tx.clone(),
            self.download_id,
        ));

        match pipeline
            .run(
                info.clone(),
                files.clone(),
                Arc::new(pp_config),
                original_stem,
                is_hls,
                self.config.verbose,
                callback_factory,
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
            Err(e) => {
                warn!("Post-processing pipeline failed: {e}");
                // Return original files on failure.
                Ok(files)
            }
        }
    }

    /// Clean up leftover HLS segment files from interrupted downloads
    ///
    /// When HLS downloads are interrupted via Ctrl+C, segment files like
    /// `filename.part0`, `filename.part1`, etc. may be left behind.
    /// This function removes them before starting a new download.
    pub(super) async fn cleanup_leftover_segments(&self, dir: &std::path::Path, base_name: &str) {
        if !dir.exists() {
            return;
        }

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!(
                    "cleanup_leftover_segments: could not enumerate {} ({e}); \
                     stale .partN files may persist and confuse the next download's \
                     resume detection",
                    dir.display()
                );
                return;
            }
        };

        let mut deleted = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
                continue;
            };

            // Match pattern: base_name.part{number}
            if !filename.starts_with(base_name) || !filename.contains(".part") {
                continue;
            }

            // Verify it's a segment file (has numeric suffix after .part)
            if let Some(part_idx) = filename.rfind(".part") {
                let suffix = &filename[part_idx + 5..];
                if suffix.chars().all(|c| c.is_ascii_digit())
                    && tokio::fs::remove_file(&path).await.is_ok()
                {
                    deleted += 1;
                }
            }
        }

        if deleted > 0 {
            debug!(deleted; "Cleaned up leftover segment files");
        }
    }
}
