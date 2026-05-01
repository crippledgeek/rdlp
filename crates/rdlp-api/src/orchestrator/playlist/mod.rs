//! Playlist download functionality
//!
//! Provides batch download support with progress tracking, resume capability,
//! and graceful degradation for failed videos.

// `Duration::from_mins` (lint's suggested replacement) needs Rust 1.95; MSRV is 1.85.
#![allow(clippy::duration_suboptimal_units)]

mod episode;
mod execution;
mod helpers;
mod resume;

#[cfg(test)]
mod tests;

use super::errors::is_reextractable_error;
use super::session_state::{self, FailedEpisode, PlaylistState, SessionState};
use super::{DownloadPlan, Orchestrator, OrchestratorError, Result, archive};
use anyhow::Context;
use futures_util::StreamExt;
use helpers::{detect_audio_types, filter_formats_by_language};
use log::{debug, error, info, warn};
use rdlp_types::{Format, SubtitleFormat};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Download result: (downloaded file paths, `is_hls`, merge formats if applicable).
type DownloadResult = Option<(Vec<PathBuf>, bool, Option<Vec<Format>>)>;

/// Maximum age of batch-resolved tokens before proactive re-extraction.
/// Megacloud/netmagcdn tokens last ~30-60 min; we re-extract at 25 min
/// to stay well within the window.
const TOKEN_MAX_AGE: Duration = Duration::from_secs(25 * 60);

/// All subtitle formats checked during resume detection.
const RESUME_SUB_FORMATS: &[SubtitleFormat] = &[
    SubtitleFormat::Srt,
    SubtitleFormat::Vtt,
    SubtitleFormat::Ass,
    SubtitleFormat::Ssa,
    SubtitleFormat::Lrc,
];

/// Resume detection result for a playlist folder.
#[derive(Debug)]
pub(super) struct ResumeDetection {
    /// Completed episodes: `sanitized_title` -> video file path
    pub completed: HashMap<String, PathBuf>,
    /// Count of episodes with leftover segment files
    pub partial_count: usize,
    /// Episodes with video complete but subtitle files missing:
    /// `sanitized_title` -> video file path
    pub missing_subs: HashMap<String, PathBuf>,
}

// Re-export for sibling modules that use extract_cdn_host
pub(super) use helpers::extract_cdn_host;

impl Orchestrator {
    /// Internal playlist download logic
    ///
    /// Separated from public method to allow reuse by auto-detection in `download()`
    ///
    /// # Resume Support
    ///
    /// Automatically detects already-downloaded videos in the playlist folder
    /// and skips them, allowing interrupted downloads to be resumed.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn download_playlist_internal(
        &self,
        mut infos: Vec<rdlp_types::InfoDict>,
        interactive: bool,
        archive: Option<HashSet<String>>,
    ) -> Result<Option<Vec<PathBuf>>> {
        let total = infos.len();
        #[allow(clippy::indexing_slicing)] // infos is non-empty (callers assert this)
        let playlist_title = infos[0]
            .playlist_title
            .clone()
            .unwrap_or_else(|| "Unnamed Playlist".to_string());

        // Create playlist folder
        let playlist_folder_name = Self::sanitize_filename(&playlist_title);
        let playlist_dir = self.config.output_directory.join(&playlist_folder_name);

        // Detect available audio/language types across all episodes
        let audio_types = detect_audio_types(&infos);

        // Load saved session state early so resume detection can use subtitle
        // selections to verify VTT files exist alongside video files.
        let state_path = session_state::playlist_state_path(&playlist_dir);
        #[allow(clippy::indexing_slicing)] // infos non-empty; asserted at call sites
        let source_url = infos
            .iter()
            .find_map(|i| i.playlist_id.clone())
            .unwrap_or_else(|| infos[0].webpage_url.clone());
        let saved_state = SessionState::load(&state_path, &source_url).await;
        let resuming = matches!(&saved_state, Some(SessionState::Playlist(_)));

        // Extract subtitle langs and strict_subs from saved state.
        // When no saved state exists (first run or previous run completed cleanly),
        // fall back to the current CLI config so resume detection can still find
        // missing subtitle files.
        let (saved_sub_langs, saved_strict_subs) = match &saved_state {
            Some(SessionState::Playlist(s)) => (s.selected_sub_langs.clone(), s.strict_subs),
            _ => (self.config.subtitle_langs.clone(), self.config.strict_subs),
        };

        // Warn if saved strict_subs differs from current CLI flag
        if resuming && saved_strict_subs != self.config.strict_subs {
            warn!(
                saved = saved_strict_subs,
                current = self.config.strict_subs;
                "strict_subs changed since last run — using saved value for resume"
            );
        }

        // Check for existing files (resume detection)
        // Pass subtitle langs and strict_subs so detection can verify VTT files exist.
        let resume = self
            .detect_existing_playlist_files(
                &playlist_dir,
                &infos,
                &saved_sub_langs,
                saved_strict_subs,
            )
            .await;
        let existing_files = resume.completed;
        let partial_count = resume.partial_count;
        let missing_subs = resume.missing_subs;
        let already_downloaded = existing_files.len();
        let remaining = total - already_downloaded;

        info!("{}", "=".repeat(60));
        info!(title:? = playlist_title; "Playlist");
        info!(path:? = playlist_dir.display(); "Folder");
        info!(total; "Total videos");

        if audio_types.len() > 1 {
            info!(types:% = audio_types.join(", "); "Audio types available");
        }

        if already_downloaded > 0 || partial_count > 0 {
            if already_downloaded > 0 {
                info!(already_downloaded; "Already downloaded");
            }
            if partial_count > 0 {
                info!(partial_count; "Leftover segments (will be cleaned up)");
            }
            info!(remaining; "Remaining");
        }

        if resuming {
            info!("Resuming with saved selections");
            if let Some(SessionState::Playlist(ref saved)) = saved_state
                && !saved.failed_episodes.is_empty()
            {
                warn!(
                    count = saved.failed_episodes.len();
                    "Previously failed episodes will be retried"
                );
                for ep in &saved.failed_episodes {
                    warn!("  [{}] {}: {}", ep.position, ep.title, ep.last_error);
                }
            }
        }

        info!("{}", "=".repeat(60));

        // If all videos are already downloaded, handle subtitle retry or return
        if remaining == 0 {
            return self
                .handle_all_downloaded(
                    &state_path,
                    &existing_files,
                    &missing_subs,
                    &saved_sub_langs,
                    &infos,
                    total,
                )
                .await;
        }

        // Restore or prompt for selections
        let mut selected_audio: Option<String>;
        let selected_sub_langs: Vec<String>;

        if let Some(SessionState::Playlist(ref saved)) = saved_state {
            // Restore selections from saved state
            selected_audio = saved.selected_audio.clone();
            selected_sub_langs = saved.selected_sub_langs.clone();

            // Apply audio filter from saved state
            if let Some(ref lang) = selected_audio {
                debug!(audio:% = lang; "Restoring audio type from saved state");
                filter_formats_by_language(&mut infos, lang);
            }
            if !selected_sub_langs.is_empty() {
                debug!(
                    subs:% = selected_sub_langs.join(", ");
                    "Restoring subtitle selection from saved state"
                );
            }
        } else {
            // Audio type selection for playlists with multiple language tracks
            selected_audio = None;
            if interactive
                && audio_types.len() > 1
                && remaining > 0
                && let Some(ref callback) = self.interactive
            {
                let options: Vec<String> = audio_types.clone();
                let type_count = options.len();

                let selection = callback.select_audio_type(&options).await;

                if let Some(idx) = selection
                    && idx < type_count
                {
                    #[allow(clippy::indexing_slicing)] // guarded by idx < type_count above
                    let selected = &audio_types[idx];
                    debug!(audio:% = selected; "Filtering to selected audio type");
                    selected_audio = Some(selected.clone());
                    filter_formats_by_language(&mut infos, selected);
                }
            }

            // Interactive subtitle selection (once for the whole playlist)
            let list_subs = self.config.list_subs;
            let sub_representative = infos.iter().find(|i| {
                i.subtitles.as_ref().is_some_and(|s| !s.is_empty())
                    || i.automatic_captions.as_ref().is_some_and(|a| !a.is_empty())
            });
            debug!(
                has_subs = sub_representative.is_some(), interactive, list_subs;
                "Playlist subtitle selection check"
            );

            selected_sub_langs =
                if let Some(rep) = sub_representative.filter(|_| interactive || list_subs) {
                    match self
                        .select_subtitles_if_needed(rep, interactive, list_subs)
                        .await?
                    {
                        Some(sel) => sel.into_iter().map(|(lang, _)| lang).collect(),
                        None => return Ok(None),
                    }
                } else {
                    Vec::new()
                };

            // Confirm before downloading (unless non-interactive)
            if interactive
                && remaining > 0
                && let Some(ref callback) = self.interactive
            {
                let prompt = if already_downloaded > 0 {
                    format!("Resume downloading {remaining} remaining videos?")
                } else {
                    format!("Download {total} videos to '{playlist_folder_name}'?")
                };

                if !callback.confirm(&prompt).await {
                    debug!("Cancelled by user");
                    return Ok(None);
                }
            }

            // Save session state after user confirms selections
            let state = SessionState::Playlist(PlaylistState::new(
                &source_url,
                &playlist_title,
                selected_audio.clone(),
                selected_sub_langs.clone(),
                self.config.strict_subs,
            ));
            state.save(&state_path).await;
        }

        // Batch-resolve all episode URLs upfront with bounded parallelism.
        let batch_resolved_at = self
            .batch_resolve_episodes(&mut infos, &existing_files, selected_audio.as_deref())
            .await;

        // Create playlist directory if it doesn't exist
        if !playlist_dir.exists() {
            tokio::fs::create_dir_all(&playlist_dir)
                .await
                .with_context(|| {
                    format!(
                        "failed to create playlist folder '{}'",
                        playlist_dir.display()
                    )
                })
                .map_err(OrchestratorError::Other)?;
            debug!(path:? = playlist_dir.display(); "Created folder");
        }

        // Download episodes with bounded parallelism (2 concurrent).
        let (mut downloaded, mut failed, mut interrupted) = self
            .download_episodes(
                &infos,
                &existing_files,
                &archive,
                &playlist_dir,
                &selected_sub_langs,
                &selected_audio,
                batch_resolved_at,
                total,
            )
            .await;

        // Retry waves for failed episodes
        let retried_ok = self
            .run_retry_waves(
                &mut failed,
                &mut downloaded,
                &mut interrupted,
                &infos,
                &playlist_dir,
                &selected_sub_langs,
                &selected_audio,
                total,
            )
            .await;

        // Subtitle retry for completed episodes missing subs
        let sub_retried_ok = if !interrupted
            && self.config.retry_subs
            && !selected_sub_langs.is_empty()
            && !missing_subs.is_empty()
        {
            self.retry_missing_subtitles(&missing_subs, &infos, &selected_sub_langs)
                .await
        } else {
            0
        };

        // Summary report
        self.print_playlist_summary(
            &playlist_dir,
            &downloaded,
            &failed,
            &missing_subs,
            &selected_sub_langs,
            total,
            already_downloaded,
            retried_ok,
            sub_retried_ok,
            interrupted,
        );

        // Persist or delete session state
        let dl_count = downloaded.len();
        if !interrupted && failed.is_empty() && dl_count == total {
            SessionState::delete(&state_path).await;
        } else {
            let failed_eps: Vec<FailedEpisode> = failed
                .iter()
                .map(|(pos, title, err)| FailedEpisode {
                    position: *pos,
                    title: title.clone(),
                    last_error: err.clone(),
                })
                .collect();
            let mut state = PlaylistState::new(
                &source_url,
                &playlist_title,
                selected_audio.clone(),
                selected_sub_langs.clone(),
                self.config.strict_subs,
            );
            state.failed_episodes = failed_eps;
            SessionState::Playlist(state).save(&state_path).await;
        }

        if downloaded.is_empty() {
            Err(OrchestratorError::ExtractionFailed(
                rdlp_core::RdlpError::Extraction {
                    message: "All playlist videos failed to download".to_string(),
                    url: None,
                },
            ))
        } else {
            Ok(Some(downloaded))
        }
    }
}
