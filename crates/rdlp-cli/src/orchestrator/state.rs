//! State machine types for the download workflow

use super::{Orchestrator, errors::*};
use log::info;
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
    /// Preparing download (checking for resume state)
    Preparing {
        /// Extracted video metadata
        info: Box<rdlp_core::InfoDict>,
        /// Selected format for download
        format: Box<Format>,
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
    /// - `SelectingFormat` → `Preparing` (after format selection) OR `Cancelled` (user cancels)
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
                Ok(Self::SelectingFormat {
                    info: Box::new(info),
                })
            }

            Self::SelectingFormat { info } => {
                let format = match orchestrator.select_format(&info, interactive).await? {
                    Some(format) => format,
                    None => return Ok(Self::Cancelled),
                };

                Ok(Self::Preparing {
                    info,
                    format: Box::new(format),
                })
            }

            Self::Preparing { info, format } => {
                let output_path = orchestrator.generate_output_path(&info, &format)?;
                info!(path:? = output_path.display(); "Downloading to");

                let resume_offset = orchestrator
                    .detect_resume_point(&output_path, format.filesize)
                    .await?;

                // Check if file is already complete
                if let Some(expected_size) = format.filesize {
                    if resume_offset == expected_size {
                        // File is already fully downloaded, skip to Complete
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
                })
            }

            Self::Downloading {
                info,
                output_path,
                format,
                state,
            } => {
                // Create progress bar with best available size estimate
                // For HLS streams, use segment-based progress bar (not byte-based)
                let is_hls = format.url.contains(".m3u8") || format.ext == "hls";

                // HLS downloads use segment-based resume (tracked in .hls_state.json), not byte offsets
                // The HLS downloader handles its own resume logic internally
                let resume_from = if is_hls {
                    0 // HLS downloader manages its own state file for segment-based resume
                } else {
                    state.offset()
                };

                let estimated_size = if is_hls {
                    None // HLS uses segment-based progress, not byte-based
                } else {
                    format.filesize.or(format.filesize_approx)
                };
                let progress_bar = if is_hls {
                    orchestrator.create_hls_progress_bar()?
                } else {
                    orchestrator.create_progress_bar(estimated_size, resume_from)?
                };

                // Find downloader
                let downloader = orchestrator
                    .downloader_registry
                    .find_downloader(&format.url)
                    .ok_or_else(|| OrchestratorError::NoDownloader {
                        url: format.url.clone(),
                    })?;

                // Execute download
                let stats = match orchestrator
                    .execute_download(
                        &downloader,
                        &format.url,
                        &output_path,
                        resume_from,
                        progress_bar.as_ref(),
                        estimated_size,
                    )
                    .await?
                {
                    Some(stats) => stats,
                    None => return Ok(Self::Cancelled),
                };

                // Report success
                info!("Downloaded successfully!");
                info!("   File: {}", output_path.display());
                info!("   Stats: {stats:?}");

                // Download thumbnail for embedding or standalone use
                // Skipped only when --no-thumbnail is set (embed_thumbnail=false && write_thumbnail=false)
                if orchestrator.config.embed_thumbnail || orchestrator.config.write_thumbnail {
                    orchestrator.download_thumbnail(&info, &output_path).await;
                }

                // Run post-processing (automatic for HLS, optional for others)
                let final_path = orchestrator
                    .run_postprocessing_for_state_machine(&info, &output_path, is_hls)
                    .await?;

                Ok(Self::Complete { path: final_path })
            }

            Self::Complete { .. } | Self::Cancelled => {
                // Already in terminal state, no further transitions
                Ok(self)
            }
        }
    }
}
