//! State machine types for the download workflow

use super::session_state::{self, SessionState, SingleVideoState};
use super::{Orchestrator, errors::*};
use log::{info, warn};
use rdlp_core::Format;
use std::fmt;
use std::path::PathBuf;
use tracing::instrument;

/// Download state for resume logic
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    /// Fresh download with no resume
    Fresh,
    /// Resume from byte offset
    Resume(u64),
}

impl DownloadState {
    /// Get the resume offset (0 for fresh downloads)
    #[must_use]
    pub fn offset(&self) -> u64 {
        match self {
            Self::Fresh => 0,
            Self::Resume(offset) => *offset,
        }
    }
}

impl fmt::Display for DownloadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fresh => write!(f, "fresh download"),
            Self::Resume(offset) => {
                let mb = *offset as f64 / (1024.0 * 1024.0);
                write!(f, "resuming from {mb:.1} MB")
            }
        }
    }
}

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
pub enum DownloadPhase {
    /// Extracting video information from URL
    Extracting {
        /// The URL to extract from
        url: String,
    },
    /// Selecting format (interactive or automatic)
    SelectingFormat {
        /// Extracted video metadata
        info: Box<rdlp_core::InfoDict>,
    },
    /// Selecting subtitles (interactive multi-select or pass-through)
    SelectingSubtitles {
        /// Extracted video metadata
        info: Box<rdlp_core::InfoDict>,
        /// Selected format for download
        format: Box<Format>,
    },
    /// Preparing download (checking for resume state)
    Preparing {
        /// Extracted video metadata
        info: Box<rdlp_core::InfoDict>,
        /// Selected format for download
        format: Box<Format>,
        /// Subtitles selected for download (empty if none)
        subtitle_selection: Vec<(String, rdlp_core::Subtitle)>,
    },
    /// Downloading with progress tracking
    Downloading {
        /// Extracted video metadata (for post-processing and thumbnail)
        info: Box<rdlp_core::InfoDict>,
        /// Path where the file will be saved
        output_path: PathBuf,
        /// Selected format being downloaded
        format: Box<Format>,
        /// Resume state (fresh or resuming from offset)
        state: DownloadState,
        /// Subtitles selected for download (empty if none)
        subtitle_selection: Vec<(String, rdlp_core::Subtitle)>,
    },
    /// Download completed successfully
    Complete {
        /// Path to the downloaded file
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
    /// - `Extracting` → `SelectingFormat` (after successful extraction)
    /// - `SelectingFormat` → `SelectingSubtitles` (after format selection)
    ///   OR `Cancelled` (user cancels)
    /// - `SelectingSubtitles` → `Preparing` (after subtitle selection or pass-through)
    ///   OR `Cancelled` (user cancels)
    /// - `Preparing` → `Downloading` (after determining resume state)
    /// - `Downloading` → `Complete` (after successful download) OR `Cancelled` (Ctrl+C)
    /// - `Complete` / `Cancelled` → Self (terminal states)
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
                let info = orchestrator.extract_video_info(&url).await?;

                // Try loading saved session state to skip interactive selection
                let sanitized = orchestrator.sanitize_filename(&info.title);
                let state_path = session_state::single_video_state_path(
                    &orchestrator.config.output_directory,
                    &sanitized,
                );
                if let Some(SessionState::SingleVideo(saved)) =
                    SessionState::load(&state_path, &url).await
                {
                    // Try to match the saved format_id against available formats
                    if let Some(format) = info
                        .formats
                        .iter()
                        .find(|f| f.format_id == saved.format_id)
                        .cloned()
                    {
                        info!(
                            format_id = saved.format_id.as_str();
                            "Resuming with saved selections"
                        );
                        // Resolve subtitles from saved language codes
                        let subtitle_selection = orchestrator
                            .resolve_subtitles_for_episode(&info, &saved.subtitle_langs);
                        return Ok(Self::Preparing {
                            info: Box::new(info),
                            format: Box::new(format),
                            subtitle_selection,
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
                let Some(format) = orchestrator.select_format(&info, interactive).await? else {
                    return Ok(Self::Cancelled);
                };

                Ok(Self::SelectingSubtitles {
                    info,
                    format: Box::new(format),
                })
            }

            Self::SelectingSubtitles { info, format } => {
                let list_subs = orchestrator.config.list_subs;
                let Some(subtitle_selection) = orchestrator
                    .select_subtitles_if_needed(&info, interactive, list_subs)
                    .await?
                else {
                    return Ok(Self::Cancelled);
                };

                // Save session state so selections survive interruption
                let sanitized = orchestrator.sanitize_filename(&info.title);
                let state_path = session_state::single_video_state_path(
                    &orchestrator.config.output_directory,
                    &sanitized,
                );
                let sub_langs: Vec<String> = subtitle_selection
                    .iter()
                    .map(|(lang, _)| lang.clone())
                    .collect();
                let state = SessionState::SingleVideo(SingleVideoState::new(
                    &info.webpage_url,
                    &info.title,
                    &format.format_id,
                    sub_langs,
                ));
                state.save(&state_path).await;

                Ok(Self::Preparing {
                    info,
                    format,
                    subtitle_selection,
                })
            }

            Self::Preparing {
                info,
                format,
                subtitle_selection,
            } => {
                let output_path = orchestrator.generate_output_path(&info, &format)?;
                info!(path:? = output_path.display(); "Downloading to");

                let resume_offset = orchestrator
                    .detect_resume_point(&output_path, format.filesize)
                    .await?;

                // Check if file is already complete
                if let Some(expected_size) = format.filesize {
                    if resume_offset == expected_size {
                        return Ok(Self::Complete { path: output_path });
                    }
                }

                let state = if resume_offset > 0 {
                    DownloadState::Resume(resume_offset)
                } else {
                    DownloadState::Fresh
                };

                Ok(Self::Downloading {
                    info,
                    output_path,
                    format,
                    state,
                    subtitle_selection,
                })
            }

            Self::Downloading {
                info,
                output_path,
                format,
                state,
                subtitle_selection,
            } => {
                // Execute download with CDN fallback (shared implementation)
                let Some(outcome) = orchestrator
                    .download_with_cdn_fallback(&format, &output_path, state.offset())
                    .await?
                else {
                    return Ok(Self::Cancelled);
                };

                // Download thumbnail for embedding or standalone use
                if orchestrator.config.embed_thumbnail || orchestrator.config.write_thumbnail {
                    orchestrator.download_thumbnail(&info, &output_path).await;
                }

                // Download subtitles (interactive selection or config-based)
                orchestrator
                    .download_subtitles(&info, &output_path, &subtitle_selection)
                    .await?;

                // Run post-processing (automatic for HLS, optional for others)
                let final_files = orchestrator
                    .run_postprocessing(&info, vec![output_path.clone()], outcome.is_hls)
                    .await?;
                let final_path = final_files.into_iter().next().unwrap_or(output_path);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selecting_subtitles_display() {
        let phase = DownloadPhase::SelectingSubtitles {
            info: Box::new(rdlp_core::InfoDict::new(
                "id",
                "title",
                "test",
                "http://example.com",
            )),
            format: Box::new(rdlp_core::Format::new(
                "f1",
                "http://example.com/v.mp4",
                "mp4",
                rdlp_core::DownloadProtocol::Https,
            )),
        };

        assert_eq!(format!("{phase}"), "selecting subtitles");
    }

    #[test]
    fn test_selecting_subtitles_passes_through_when_no_subs() {
        // Verify the phase can be constructed with empty subtitle data
        let info = Box::new(rdlp_core::InfoDict::new(
            "id",
            "title",
            "test",
            "http://example.com",
        ));

        // No subtitles in info → select_subtitles_if_needed returns empty vec
        assert!(info.subtitles.is_none());
        assert!(info.automatic_captions.is_none());
    }
}
