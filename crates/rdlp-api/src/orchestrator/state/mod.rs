//! State machine types for the download workflow

mod download_state;

pub use download_state::DownloadState;

use super::DownloadPlan;
use super::session_state::{self, SessionState, SingleVideoState};
use super::{Orchestrator, errors::*};
use crate::events::Event;
use log::{debug, warn};
use rdlp_types::Format;
use std::fmt;
use std::path::PathBuf;
use tracing::instrument;

#[cfg(test)]
mod tests;

/// Download workflow phases
///
/// This enum represents the explicit state machine for the download workflow.
/// Each phase contains the data needed to transition to the next phase.
///
/// # Memory Optimization
///
/// Large fields (`InfoDict`, `Format`) are boxed to reduce enum size and improve
/// performance when the enum is moved/copied. This reduces stack usage and
/// prevents unnecessary copying of large structs.
#[derive(Debug)]
#[non_exhaustive]
pub enum DownloadPhase {
    /// Extracting video information from URL
    Extracting {
        /// The URL to extract from
        url: String,
    },
    /// Selecting format (interactive or automatic)
    SelectingFormat {
        /// Extracted video metadata
        info: Box<rdlp_types::InfoDict>,
    },
    /// Selecting subtitles (interactive multi-select or pass-through)
    SelectingSubtitles {
        /// Extracted video metadata
        info: Box<rdlp_types::InfoDict>,
        /// Primary format (video format for merge, combined format for single)
        format: Box<Format>,
        /// Download plan (single or merge)
        plan: Box<DownloadPlan>,
    },
    /// Preparing download (checking for resume state)
    Preparing {
        /// Extracted video metadata
        info: Box<rdlp_types::InfoDict>,
        /// Primary format (video format for merge, combined format for single)
        format: Box<Format>,
        /// Subtitles selected for download (empty if none)
        subtitle_selection: Vec<(String, rdlp_types::Subtitle)>,
        /// Download plan (single or merge)
        plan: Box<DownloadPlan>,
    },
    /// Downloading with progress tracking
    Downloading {
        /// Extracted video metadata (for post-processing and thumbnail)
        info: Box<rdlp_types::InfoDict>,
        /// Path where the file will be saved
        output_path: PathBuf,
        /// Primary format (video format for merge, combined format for single)
        format: Box<Format>,
        /// Resume state (fresh or resuming from offset)
        state: DownloadState,
        /// Subtitles selected for download (empty if none)
        subtitle_selection: Vec<(String, rdlp_types::Subtitle)>,
        /// Download plan (single or merge)
        plan: Box<DownloadPlan>,
    },
    /// Download completed successfully.
    ///
    /// For file downloads, `path` is the real output path.
    /// For stdout mode (`-o -`), `path` is the sentinel `"-"`.
    Complete {
        /// Path to the downloaded file, or `"-"` for stdout output.
        path: PathBuf,
    },
    /// User cancelled the operation
    Cancelled,
}

impl fmt::Display for DownloadPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Extracting { url } => {
                let domain = url.split('/').nth(2).unwrap_or(url);
                write!(f, "extracting from {domain}")
            }
            Self::SelectingFormat { .. } => write!(f, "selecting format"),
            Self::SelectingSubtitles { .. } => write!(f, "selecting subtitles"),
            Self::Preparing { format, .. } => {
                write!(f, "preparing {} download", format.format_id)
            }
            Self::Downloading { state, .. } => {
                write!(f, "downloading ({state})")
            }
            Self::Complete { path } => {
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                write!(f, "complete: {filename}")
            }
            Self::Cancelled => write!(f, "cancelled by user"),
        }
    }
}

impl DownloadPhase {
    /// Advance to the next phase in the download workflow
    ///
    /// # State Transitions
    ///
    /// - `Extracting` -> `SelectingFormat` (after successful extraction)
    /// - `SelectingFormat` -> `SelectingSubtitles` (after format selection)
    ///   OR `Cancelled` (user cancels)
    /// - `SelectingSubtitles` -> `Preparing` (after subtitle selection or pass-through)
    ///   OR `Cancelled` (user cancels)
    /// - `Preparing` -> `Downloading` (after determining resume state)
    /// - `Downloading` -> `Complete` (after successful download) OR `Cancelled` (Ctrl+C)
    /// - `Complete` / `Cancelled` -> Self (terminal states)
    ///
    /// **Stdout fast-path (`-o -`):** When `config.output_to_stdout` is true:
    /// - `Extracting` skips loading saved session state (no file to resume).
    /// - `SelectingSubtitles` skips persisting session state.
    /// - `Preparing` skips path generation (uses `"-"` sentinel), rejects merge
    ///   plans, and always starts fresh (no resume).
    /// - `Downloading` streams directly to stdout via `download_to_stdout()`,
    ///   then transitions straight to `Complete`, skipping subtitles,
    ///   thumbnails, and all post-processing.
    ///
    /// # Errors
    ///
    /// Returns an error if any phase transition fails (extraction error, download error, etc.)
    #[instrument(skip_all, fields(phase = %self))]
    pub(super) async fn advance(
        self,
        orchestrator: &Orchestrator,
        interactive: bool,
    ) -> Result<Self> {
        match self {
            Self::Extracting { url } => {
                orchestrator.emit(Event::Started {
                    id: orchestrator.download_id,
                    url: url.clone(),
                });

                let info = orchestrator.extract_video_info(&url).await?;

                orchestrator.emit(Event::MetadataReady {
                    id: orchestrator.download_id,
                    info: Box::new(info.clone()),
                });

                // Try loading saved session state to skip interactive selection.
                // Skip in stdout mode — session state is irrelevant for pipe output
                // and a stale merge state could produce a confusing error.
                let sanitized = orchestrator.sanitize_filename(&info.title);
                let state_path = session_state::single_video_state_path(
                    &orchestrator.config.output_directory,
                    &sanitized,
                );
                if !orchestrator.config.output_to_stdout
                    && let Some(SessionState::SingleVideo(saved)) =
                        SessionState::load(&state_path, &url).await
                {
                    // Try to match the saved format_id against available formats
                    if let Some(format) = info
                        .formats
                        .iter()
                        .find(|f| f.format_id == saved.format_id)
                        .cloned()
                    {
                        debug!(
                            format_id = saved.format_id.as_str();
                            "Resuming with saved selections"
                        );
                        // Resolve subtitles from saved language codes
                        let subtitle_selection = orchestrator
                            .resolve_subtitles_for_episode(&info, &saved.subtitle_langs);
                        // Reconstruct plan: merge if audio format was saved
                        let plan = if let Some(ref audio_id) = saved.audio_format_id {
                            if let Some(audio) = info
                                .formats
                                .iter()
                                .find(|f| f.format_id == *audio_id)
                                .cloned()
                            {
                                debug!(
                                    audio_id = audio_id.as_str();
                                    "Resuming merge plan with saved audio format"
                                );
                                DownloadPlan::Merge {
                                    video: format.clone(),
                                    audio,
                                }
                            } else {
                                warn!(
                                    audio_id = audio_id.as_str();
                                    "Saved audio format no longer available, \
                                     falling back to single format"
                                );
                                DownloadPlan::Single(format.clone())
                            }
                        } else {
                            DownloadPlan::Single(format.clone())
                        };
                        return Ok(Self::Preparing {
                            info: Box::new(info),
                            format: Box::new(format),
                            subtitle_selection,
                            plan: Box::new(plan),
                        });
                    }
                    warn!(
                        format_id = saved.format_id.as_str();
                        "Saved format no longer available, prompting again"
                    );
                }

                Ok(Self::SelectingFormat {
                    info: Box::new(info),
                })
            }

            Self::SelectingFormat { info } => {
                let Some(plan) = orchestrator.select_format(&info, interactive).await? else {
                    orchestrator.emit(Event::Cancelled {
                        id: orchestrator.download_id,
                    });
                    return Ok(Self::Cancelled);
                };

                let primary_format = match &plan {
                    DownloadPlan::Single(f) => f.clone(),
                    DownloadPlan::Merge { video, .. } => video.clone(),
                };

                orchestrator.emit(Event::FormatSelected {
                    id: orchestrator.download_id,
                    format_id: primary_format.format_id.clone(),
                    quality: primary_format
                        .format_note
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                });

                Ok(Self::SelectingSubtitles {
                    info,
                    format: Box::new(primary_format),
                    plan: Box::new(plan),
                })
            }

            Self::SelectingSubtitles { info, format, plan } => {
                let list_subs = orchestrator.config.list_subs;
                let Some(subtitle_selection) = orchestrator
                    .select_subtitles_if_needed(&info, interactive, list_subs)
                    .await?
                else {
                    orchestrator.emit(Event::Cancelled {
                        id: orchestrator.download_id,
                    });
                    return Ok(Self::Cancelled);
                };

                // Save session state so selections survive interruption.
                // Skip for stdout mode — no file to resume, and the state
                // would just be deleted on completion anyway.
                if !orchestrator.config.output_to_stdout {
                    let sanitized = orchestrator.sanitize_filename(&info.title);
                    let state_path = session_state::single_video_state_path(
                        &orchestrator.config.output_directory,
                        &sanitized,
                    );
                    let sub_langs: Vec<String> = subtitle_selection
                        .iter()
                        .map(|(lang, _)| lang.clone())
                        .collect();
                    let audio_format_id = match &*plan {
                        DownloadPlan::Merge { audio, .. } => Some(audio.format_id.clone()),
                        DownloadPlan::Single(_) => None,
                    };
                    let state = SessionState::SingleVideo(SingleVideoState::new(
                        &info.webpage_url,
                        &info.title,
                        &format.format_id,
                        sub_langs,
                        audio_format_id,
                    ));
                    state.save(&state_path).await;
                }

                Ok(Self::Preparing {
                    info,
                    format,
                    subtitle_selection,
                    plan,
                })
            }

            Self::Preparing {
                info,
                format,
                subtitle_selection,
                plan,
            } => {
                // Stdout mode: skip path generation and resume detection.
                // Reject merge plans early — the Downloading phase would
                // also reject, but failing here gives a clearer context.
                if orchestrator.config.output_to_stdout {
                    if matches!(*plan, DownloadPlan::Merge { .. }) {
                        return Err(OrchestratorError::Configuration(
                            "Merge downloads (video+audio) are not supported \
                             with -o - (stdout output)"
                                .to_string(),
                        ));
                    }
                    debug!("Downloading to stdout");
                    return Ok(Self::Downloading {
                        info,
                        output_path: PathBuf::from("-"),
                        format,
                        state: DownloadState::Fresh,
                        subtitle_selection,
                        plan,
                    });
                }

                let output_path = orchestrator.generate_output_path(&info, &format)?;
                // Create the output parent directory via tokio::fs so the
                // runtime thread isn't stalled by std::fs::create_dir_all on
                // slow / network filesystems.
                Orchestrator::ensure_parent_dir(&output_path).await?;
                debug!(path:? = output_path.display(); "Downloading to");

                // Resume detection only applies to Single downloads.
                // Merge downloads create separate stream files (video + audio)
                // at derived paths and always start fresh.
                let state = match *plan {
                    DownloadPlan::Merge { .. } => DownloadState::Fresh,
                    DownloadPlan::Single(_) => {
                        let resume_offset = orchestrator
                            .detect_resume_point(&output_path, format.filesize)
                            .await?;

                        // Check if file is already complete
                        if let Some(expected_size) = format.filesize
                            && resume_offset == expected_size
                        {
                            return Ok(Self::Complete { path: output_path });
                        }

                        if resume_offset > 0 {
                            DownloadState::Resume(resume_offset)
                        } else {
                            DownloadState::Fresh
                        }
                    }
                };

                Ok(Self::Downloading {
                    info,
                    output_path,
                    format,
                    state,
                    subtitle_selection,
                    plan,
                })
            }

            Self::Downloading {
                mut info,
                output_path,
                format,
                state,
                subtitle_selection,
                plan,
            } => {
                // Stdout mode: stream directly, skip post-processing
                if orchestrator.config.output_to_stdout {
                    if matches!(*plan, DownloadPlan::Merge { .. }) {
                        return Err(OrchestratorError::Configuration(
                            "Merge downloads (video+audio) are not supported \
                             with -o - (stdout output)"
                                .to_string(),
                        ));
                    }

                    let Some(_) = orchestrator.download_to_stdout(&format).await? else {
                        orchestrator.emit(Event::Cancelled {
                            id: orchestrator.download_id,
                        });
                        return Ok(Self::Cancelled);
                    };

                    return Ok(Self::Complete {
                        path: PathBuf::from("-"),
                    });
                }

                // Branch on download plan
                let (download_files, is_hls) = match *plan {
                    DownloadPlan::Single(_) => {
                        // Single format: existing path
                        let Some(outcome) = orchestrator
                            .download_with_cdn_fallback(&format, &output_path, state.offset())
                            .await?
                        else {
                            orchestrator.emit(Event::Cancelled {
                                id: orchestrator.download_id,
                            });
                            return Ok(Self::Cancelled);
                        };
                        (vec![output_path.clone()], outcome.is_hls)
                    }
                    DownloadPlan::Merge {
                        ref video,
                        ref audio,
                    } => {
                        // Merge: parallel video + audio download
                        let Some(outcome) = orchestrator
                            .download_merge_pair(video, audio, &output_path)
                            .await?
                        else {
                            orchestrator.emit(Event::Cancelled {
                                id: orchestrator.download_id,
                            });
                            return Ok(Self::Cancelled);
                        };

                        // Set requested_formats so FFmpegMerger::should_run()
                        // returns true
                        info.requested_formats = Some(vec![video.clone(), audio.clone()]);

                        (vec![outcome.video_path, outcome.audio_path], outcome.is_hls)
                    }
                };

                // Download thumbnail for embedding or standalone use
                if orchestrator.config.postprocess.embed_thumbnail
                    || orchestrator.config.postprocess.write_thumbnail
                {
                    orchestrator.download_thumbnail(&info, &output_path).await;
                }

                // Download subtitles (interactive selection or config-based)
                orchestrator
                    .download_subtitles(&info, &output_path, &subtitle_selection)
                    .await?;

                // Run post-processing (FFmpegMerger at priority 100 will
                // merge when it detects 2 files with requested_formats set)
                let final_files = orchestrator
                    .run_postprocessing(&info, download_files, is_hls)
                    .await?;
                let final_path = final_files.into_iter().next().unwrap_or(output_path);

                // Verify the output file actually exists
                if !final_path.exists() {
                    warn!(
                        "Output file missing after post-processing: {}",
                        final_path.display()
                    );
                }

                // Delete session state on successful completion
                let sanitized = orchestrator.sanitize_filename(&info.title);
                let state_path = session_state::single_video_state_path(
                    &orchestrator.config.output_directory,
                    &sanitized,
                );
                SessionState::delete(&state_path).await;

                Ok(Self::Complete { path: final_path })
            }

            Self::Complete { .. } | Self::Cancelled => {
                // Already in terminal state, no further transitions
                Ok(self)
            }
        }
    }
}
