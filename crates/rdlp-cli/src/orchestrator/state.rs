//! State machine types for the download workflow

use super::{errors::*, Orchestrator};
use rdlp_core::Format;
use std::path::PathBuf;

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
    pub fn offset(&self) -> u64 {
        match self {
            Self::Fresh => 0,
            Self::Resume(offset) => *offset,
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
    Extracting { url: String },
    /// Selecting format (interactive or automatic)
    SelectingFormat { info: Box<rdlp_core::InfoDict> },
    /// Preparing download (checking for resume state)
    Preparing {
        info: Box<rdlp_core::InfoDict>,
        format: Box<Format>,
    },
    /// Downloading with progress tracking
    Downloading {
        output_path: PathBuf,
        format: Box<Format>,
        state: DownloadState,
    },
    /// Download completed successfully
    Complete { path: PathBuf },
    /// User cancelled the operation
    Cancelled,
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
                let format = match orchestrator.select_format(&info.formats, interactive)? {
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
                println!("💾 Downloading to: {}", output_path.display());

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
                    output_path,
                    format,
                    state,
                })
            }

            Self::Downloading {
                output_path,
                format,
                state,
            } => {
                let resume_from = state.offset();

                // Create progress bar
                let progress_bar =
                    orchestrator.create_progress_bar(format.filesize, resume_from)?;

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
                        &progress_bar,
                    )
                    .await?
                {
                    Some(stats) => stats,
                    None => return Ok(Self::Cancelled),
                };

                // Report success
                println!("\n✅ Downloaded successfully!");
                println!("   File: {}", output_path.display());
                println!("   Size: {}", stats.bytes_string());
                println!("   Speed: {}", stats.speed_string());
                println!("   Time: {:.2}s", stats.duration.as_secs_f64());

                Ok(Self::Complete { path: output_path })
            }

            Self::Complete { .. } | Self::Cancelled => {
                // Already in terminal state, no further transitions
                Ok(self)
            }
        }
    }
}
