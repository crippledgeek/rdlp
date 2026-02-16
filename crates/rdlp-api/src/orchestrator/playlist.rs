//! Playlist download functionality
//!
//! Provides batch download support with progress tracking, resume capability,
//! and graceful degradation for failed videos.

use super::errors::is_reextractable_error;
use super::session_state::{self, FailedEpisode, PlaylistState, SessionState};
use super::{Orchestrator, OrchestratorError, Result, archive};
use futures_util::StreamExt;
use log::{debug, error, info, warn};
use rdlp_core::SubtitleFormat;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
pub(super) struct ResumeDetection {
    /// Completed episodes: sanitized_title -> video file path
    pub completed: HashMap<String, PathBuf>,
    /// Count of episodes with leftover segment files
    pub partial_count: usize,
    /// Episodes with video complete but subtitle files missing:
    /// sanitized_title -> video file path
    pub missing_subs: HashMap<String, PathBuf>,
}

/// Check if any subtitle file exists for the given language (exact or fuzzy).
///
/// Checks `{stem}.{lang}.{ext}` for all subtitle formats. If no exact match,
/// scans `{stem}.*.{ext}` patterns for fuzzy prefix matches (e.g., `"en"` finds
/// `title.English.vtt`).
fn has_subtitle_file(dir: &std::path::Path, stem: &str, lang: &str) -> bool {
    // Exact match: {stem}.{lang}.{ext}
    for fmt in RESUME_SUB_FORMATS {
        if dir.join(format!("{stem}.{lang}.{}", fmt.as_ext())).exists() {
            return true;
        }
    }

    // Fuzzy match: scan for {stem}.*.{ext} and prefix-match the middle segment
    if lang.len() <= 3 {
        let lang_lower = lang.to_ascii_lowercase();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Must start with "{stem}." and have at least one more dot
                if let Some(rest) = name_str
                    .strip_prefix(stem)
                    .and_then(|r| r.strip_prefix('.'))
                {
                    // rest = "English.vtt" → split into ("English", "vtt")
                    if let Some((mid, ext)) = rest.rsplit_once('.') {
                        let is_sub_ext = RESUME_SUB_FORMATS
                            .iter()
                            .any(|f| f.as_ext().eq_ignore_ascii_case(ext));
                        if is_sub_ext && mid.to_ascii_lowercase().starts_with(&lang_lower) {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

impl Orchestrator {
    /// Download all videos from a playlist
    ///
    /// This method provides explicit playlist download functionality with:
    /// - User confirmation prompt (interactive mode)
    /// - Progress tracking per video
    /// - Graceful degradation (skip failed videos)
    /// - Summary report at end
    ///
    /// # Returns
    ///
    /// - `Ok(Some(paths))` - All or some downloads completed (graceful degradation)
    /// - `Ok(None)` - User cancelled operation
    /// - `Err` - Fatal error (no videos downloaded)
    pub async fn download_playlist(
        &self,
        url: &str,
        interactive: bool,
    ) -> Result<Option<Vec<PathBuf>>> {
        // Extract playlist
        let infos = self.extract_playlist_info(url).await?;

        // If single video, delegate to single download
        if infos.len() == 1 {
            return self
                .download(url, interactive)
                .await
                .map(|opt| opt.map(|path| vec![path]));
        }

        let archive = self.load_archive_if_configured();
        self.download_playlist_internal(infos, interactive, archive)
            .await
    }

    /// Internal playlist download logic
    ///
    /// Separated from public method to allow reuse by auto-detection in `download()`
    ///
    /// # Resume Support
    ///
    /// Automatically detects already-downloaded videos in the playlist folder
    /// and skips them, allowing interrupted downloads to be resumed.
    pub(super) async fn download_playlist_internal(
        &self,
        mut infos: Vec<rdlp_core::InfoDict>,
        interactive: bool,
        archive: Option<HashSet<String>>,
    ) -> Result<Option<Vec<PathBuf>>> {
        let total = infos.len();
        let playlist_title = infos[0]
            .playlist_title
            .clone()
            .unwrap_or_else(|| "Unnamed Playlist".to_string());

        // Create playlist folder
        let playlist_folder_name = self.sanitize_filename(&playlist_title);
        let playlist_dir = self.config.output_directory.join(&playlist_folder_name);

        // Detect available audio/language types across all episodes
        let audio_types = detect_audio_types(&infos);

        // Load saved session state early so resume detection can use subtitle
        // selections to verify VTT files exist alongside video files.
        let state_path = session_state::playlist_state_path(&playlist_dir);
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
            if let Some(SessionState::Playlist(ref saved)) = saved_state {
                if !saved.failed_episodes.is_empty() {
                    warn!(
                        count = saved.failed_episodes.len();
                        "Previously failed episodes will be retried"
                    );
                    for ep in &saved.failed_episodes {
                        warn!("  [{}] {}: {}", ep.position, ep.title, ep.last_error);
                    }
                }
            }
        }

        info!("{}", "=".repeat(60));

        // If all videos are already downloaded, handle subtitle retry or return
        if remaining == 0 {
            let need_sub_retry =
                self.config.retry_subs && !saved_sub_langs.is_empty() && !missing_subs.is_empty();
            if !need_sub_retry {
                if !missing_subs.is_empty() && !saved_sub_langs.is_empty() {
                    // Report missing subs even when not retrying
                    warn!("Missing subtitles for {} episode(s)", missing_subs.len());
                    self.log_missing_subs(&missing_subs, &infos, total);
                    if !self.config.retry_subs {
                        info!("Hint: re-run with --retry-subs to attempt subtitle download");
                    }
                }
                info!("All videos already downloaded");
                SessionState::delete(&state_path).await;
                let paths: Vec<PathBuf> = existing_files.into_values().collect();
                return Ok(Some(paths));
            }
            // Fall through to subtitle retry pass below
            info!("All videos already downloaded — retrying missing subtitles");
            let sub_recovered = self
                .retry_missing_subtitles(&missing_subs, &infos, &saved_sub_langs)
                .await;
            if sub_recovered > 0 {
                info!("Recovered subtitles for {sub_recovered} episode(s)");
            }
            let still_missing = missing_subs.len() - sub_recovered;
            if still_missing > 0 {
                warn!("Still missing subtitles for {still_missing} episode(s)");
            }
            SessionState::delete(&state_path).await;
            let paths: Vec<PathBuf> = existing_files.into_values().collect();
            return Ok(Some(paths));
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
                info!(audio:% = lang; "Restoring audio type from saved state");
                filter_formats_by_language(&mut infos, lang);
            }
            if !selected_sub_langs.is_empty() {
                info!(
                    subs:% = selected_sub_langs.join(", ");
                    "Restoring subtitle selection from saved state"
                );
            }
        } else {
            // Audio type selection for playlists with multiple language tracks
            selected_audio = None;
            if interactive && audio_types.len() > 1 && remaining > 0 {
                use dialoguer::Select;

                let mut options: Vec<String> = audio_types.clone();
                options.push("All (keep both)".to_string());
                let type_count = audio_types.len();

                let selection = tokio::task::spawn_blocking(move || {
                    Select::new()
                        .with_prompt("Select audio type")
                        .items(&options)
                        .default(0)
                        .interact()
                        .unwrap_or(options.len() - 1)
                })
                .await
                .unwrap_or(type_count);

                if selection < type_count {
                    let selected = &audio_types[selection];
                    info!(audio:% = selected; "Filtering to selected audio type");
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
            if interactive && remaining > 0 {
                use dialoguer::Confirm;

                let prompt = if already_downloaded > 0 {
                    format!("Resume downloading {remaining} remaining videos?")
                } else {
                    format!("Download {total} videos to '{playlist_folder_name}'?")
                };

                let proceed = tokio::task::spawn_blocking(move || {
                    Confirm::new()
                        .with_prompt(prompt)
                        .default(true)
                        .interact()
                        .unwrap_or(false)
                })
                .await
                .unwrap_or(false);

                if !proceed {
                    info!("Cancelled by user");
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
        // Resolving 3 episodes concurrently keeps CDN tokens fresh while
        // avoiding rate-limits. Layer 3 retry handles any that expire later.
        const RESOLVE_CONCURRENCY: usize = 4;

        // Collect work items: (index, url, title) for episodes needing resolution
        let to_resolve: Vec<(usize, String, String)> = infos
            .iter()
            .enumerate()
            .filter_map(|(i, ep)| {
                let sanitized = self.sanitize_filename(&ep.title);
                if existing_files.contains_key(&sanitized) || ep.webpage_url.is_empty() {
                    None
                } else {
                    Some((i, ep.webpage_url.clone(), ep.title.clone()))
                }
            })
            .collect();

        let resolve_count = to_resolve.len();
        info!(count = resolve_count, concurrency = RESOLVE_CONCURRENCY; "Resolving episode URLs");

        // Resolve with buffer_unordered: starts the next future as soon as
        // any one completes, keeping the pipeline always full at N concurrent.
        let futs = to_resolve.iter().map(|(i, url, title)| {
            let i = *i;
            let url = url.clone();
            let title = title.clone();
            async move {
                info!(position = i + 1, title:? = title; "Resolving episode");
                (i, self.extract_lazy_formats(&url).await)
            }
        });

        let results: Vec<_> = futures_util::stream::iter(futs)
            .buffer_unordered(RESOLVE_CONCURRENCY)
            .collect()
            .await;

        for (i, result) in results {
            match result {
                Ok(resolved) => {
                    let mut formats = resolved.formats;

                    // Apply audio type filter
                    if let Some(ref lang) = selected_audio {
                        formats.retain(|f| f.language.as_deref() == Some(lang.as_str()));
                    }

                    infos[i].formats = formats;

                    // Carry over subtitles from lazy resolution
                    if infos[i].subtitles.is_none() {
                        infos[i].subtitles = resolved.subtitles;
                    }
                }
                Err(e) => {
                    warn!(
                        title:? = infos[i].title;
                        "Failed to resolve: {e} — will retry at download time"
                    );
                    infos[i].formats.clear();
                }
            }
        }

        // Clear formats for already-downloaded episodes
        for ep in infos.iter_mut() {
            let sanitized = self.sanitize_filename(&ep.title);
            if existing_files.contains_key(&sanitized) {
                ep.formats.clear();
            }
        }

        // Track when batch resolution completed for token freshness guard.
        // Episodes downloaded >25 min after this point will re-extract fresh URLs.
        let batch_resolved_at = Instant::now();
        info!("Batch resolution complete ({resolve_count} episodes)");

        // Create playlist directory if it doesn't exist
        if !playlist_dir.exists() {
            tokio::fs::create_dir_all(&playlist_dir)
                .await
                .map_err(|e| {
                    OrchestratorError::IoError(format!(
                        "Failed to create playlist folder '{}': {e}",
                        playlist_dir.display()
                    ))
                })?;
            info!(path:? = playlist_dir.display(); "Created folder");
        }

        // Download episodes with bounded parallelism (2 concurrent).
        // Pre-filter to skip already-downloaded and archived episodes.
        const DOWNLOAD_CONCURRENCY: usize = 2;
        let mut downloaded: Vec<PathBuf> = existing_files.into_values().collect();
        let mut failed = Vec::new();
        let mut interrupted = false;

        // Collect episodes that still need downloading
        let to_download: Vec<(usize, &rdlp_core::InfoDict)> = infos
            .iter()
            .enumerate()
            .filter(|(_, ep)| {
                let sanitized = self.sanitize_filename(&ep.title);
                let exists = downloaded.iter().any(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|name| name == sanitized)
                });
                if exists {
                    return false;
                }
                if let Some(ref archive_set) = archive {
                    if archive::is_in_archive(archive_set, &ep.extractor, &ep.id) {
                        return false;
                    }
                }
                true
            })
            .collect();

        let download_count = to_download.len();
        if download_count > 0 {
            info!(
                count = download_count,
                concurrency = DOWNLOAD_CONCURRENCY;
                "Downloading episodes"
            );
        }

        // Build concurrent download stream.
        // Each future clones the data it needs so lifetimes are straightforward.
        let batch_ts = Some(batch_resolved_at);
        let futs = to_download.into_iter().map(|(index, info)| {
            let position = index + 1;
            let info_owned = info.clone();
            let dir = playlist_dir.clone();
            let sub_langs = selected_sub_langs.clone();
            let audio = selected_audio.clone();
            async move {
                info!("{}", "─".repeat(60));
                info!(position, total, title:? = info_owned.title; "Downloading");
                let result = self
                    .download_from_info_to_dir(
                        &info_owned,
                        false,
                        &dir,
                        &sub_langs,
                        audio.as_deref(),
                        batch_ts,
                    )
                    .await;
                (position, info_owned, result)
            }
        });

        let mut stream = futures_util::stream::iter(futs).buffer_unordered(DOWNLOAD_CONCURRENCY);

        // Consume download stream with Ctrl+C support
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        loop {
            tokio::select! {
                next = stream.next() => {
                    match next {
                        Some((position, info_owned, result)) => {
                            match result {
                                Ok(Some(path)) => {
                                    info!(position, total, path:? = path.display(); "Saved");
                                    downloaded.push(path);
                                    self.record_in_archive(
                                        &info_owned.extractor,
                                        &info_owned.id,
                                    );
                                }
                                Ok(None) => {
                                    info!(position, total; "Skipped by user");
                                }
                                Err(e) => {
                                    error!(position, total; "Failed: {e}");
                                    failed.push((
                                        position,
                                        info_owned.title,
                                        e.to_string(),
                                    ));
                                }
                            }
                        }
                        None => break, // All downloads complete
                    }
                }
                _ = &mut ctrl_c => {
                    info!("Playlist download interrupted by user");
                    info!("Run the same command again to resume");
                    interrupted = true;
                    break;
                }
            }
        }

        // ── Retry waves for failed episodes ──────────────────────────
        // After the initial pass, retry failed episodes up to 3 more
        // times with increasing delays. Each wave re-extracts fresh CDN
        // URLs since old tokens are likely expired.
        const MAX_RETRY_WAVES: usize = 3;
        const RETRY_WAVE_DELAYS: [u64; MAX_RETRY_WAVES] = [60, 120, 180];
        let mut retried_ok = 0usize;

        if !interrupted && !failed.is_empty() {
            info!("");
            info!("{} episode(s) failed — starting retry waves", failed.len());

            for (wave, &delay_secs) in RETRY_WAVE_DELAYS.iter().enumerate() {
                if failed.is_empty() || interrupted {
                    break;
                }

                let fail_count = failed.len();
                info!(
                    wave = wave + 1,
                    max_waves = MAX_RETRY_WAVES,
                    failed = fail_count,
                    delay_secs;
                    "Retry wave — waiting before retry"
                );

                // Sleep with Ctrl+C race so user can abort during wait
                let sleep = tokio::time::sleep(Duration::from_secs(delay_secs));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => {}
                    _ = &mut ctrl_c => {
                        info!("Retry interrupted by user");
                        interrupted = true;
                        break;
                    }
                }

                // Take current failures; will re-collect any that fail again
                let retry_batch: Vec<(usize, String, String)> = std::mem::take(&mut failed);

                let retry_futs = retry_batch.into_iter().map(|(position, title, _err)| {
                    let dir = playlist_dir.clone();
                    let sub_langs = selected_sub_langs.clone();
                    let audio = selected_audio.clone();
                    let index = position - 1;
                    // Clone the original info and clear formats to force
                    // lazy re-extraction of fresh CDN URLs.
                    let mut info_stub = infos[index].clone();
                    info_stub.formats.clear();
                    async move {
                        info!(
                            wave = wave + 1,
                            position,
                            total,
                            title:? = title;
                            "Retrying episode"
                        );
                        let result = self
                            .download_from_info_to_dir(
                                &info_stub,
                                false,
                                &dir,
                                &sub_langs,
                                audio.as_deref(),
                                None,
                            )
                            .await;
                        (position, title, result)
                    }
                });

                let mut retry_stream =
                    futures_util::stream::iter(retry_futs).buffer_unordered(DOWNLOAD_CONCURRENCY);

                loop {
                    tokio::select! {
                        next = retry_stream.next() => {
                            match next {
                                Some((pos, title, result)) => {
                                    match result {
                                        Ok(Some(path)) => {
                                            info!(
                                                wave = wave + 1,
                                                pos, total,
                                                path:? = path.display();
                                                "Retry succeeded"
                                            );
                                            downloaded.push(path);
                                            let idx = pos - 1;
                                            self.record_in_archive(
                                                &infos[idx].extractor,
                                                &infos[idx].id,
                                            );
                                            retried_ok += 1;
                                        }
                                        Ok(None) => {
                                            info!(
                                                pos, total;
                                                "Skipped on retry"
                                            );
                                        }
                                        Err(e) => {
                                            error!(
                                                wave = wave + 1,
                                                pos, total;
                                                "Retry failed: {e}"
                                            );
                                            failed.push((
                                                pos,
                                                title,
                                                e.to_string(),
                                            ));
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                        _ = &mut ctrl_c => {
                            info!("Retry wave interrupted by user");
                            interrupted = true;
                            break;
                        }
                    }
                }

                if failed.is_empty() {
                    info!("All previously-failed episodes recovered!");
                    break;
                }
            }
        }

        // ── Subtitle retry for completed episodes missing subs ──
        let mut sub_retried_ok = 0usize;
        if !interrupted
            && self.config.retry_subs
            && !selected_sub_langs.is_empty()
            && !missing_subs.is_empty()
        {
            sub_retried_ok = self
                .retry_missing_subtitles(&missing_subs, &infos, &selected_sub_langs)
                .await;
        }

        // Summary report
        let newly_downloaded = downloaded.len() - already_downloaded;
        let dl_count = downloaded.len();

        info!("");
        info!("{}", "=".repeat(60));
        info!("Playlist Download Summary");
        info!("{}", "=".repeat(60));
        info!("Folder: {}", playlist_dir.display());
        info!("Downloaded: {dl_count}/{total}");

        if already_downloaded > 0 {
            info!("  (previously: {already_downloaded}, this session: {newly_downloaded})");
        }

        if retried_ok > 0 {
            info!("  (recovered {retried_ok} via retry waves)");
        }

        if !selected_sub_langs.is_empty() {
            info!("Subtitles: {}", selected_sub_langs.join(", "));
        }

        if !failed.is_empty() {
            error!("Failed: {}", failed.len());
            for (pos, title, err) in &failed {
                error!("  [{pos}/{total}] {title}: {err}");
            }
        }

        // Report missing subtitles
        if !missing_subs.is_empty() && !selected_sub_langs.is_empty() {
            let still_missing = missing_subs.len() - sub_retried_ok;
            if sub_retried_ok > 0 {
                info!(
                    "Subtitles recovered: {sub_retried_ok}/{}",
                    missing_subs.len()
                );
            }
            if still_missing > 0 {
                warn!("Missing subtitles: {still_missing}");
                self.log_missing_subs(&missing_subs, &infos, total);
                if !self.config.retry_subs {
                    info!("Hint: re-run with --retry-subs to attempt subtitle download");
                }
            }
        }

        if interrupted {
            let remaining_after = total - dl_count;
            info!("Interrupted — {remaining_after} videos remaining");
            info!("Run the same command again to resume");
        }

        info!("{}", "=".repeat(60));

        // Delete session state only when fully complete (no failures, no interruption).
        // Otherwise update with current failed episodes so the next run knows what to retry.
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
                rdlp_core::RdlpError::Extraction(
                    "All playlist videos failed to download".to_string(),
                ),
            ))
        } else {
            Ok(Some(downloaded))
        }
    }

    /// Detect existing files in playlist folder that match video titles
    ///
    /// Returns a [`ResumeDetection`] containing:
    /// - `completed`: sanitized title -> file path for completed downloads
    /// - `partial_count`: videos with leftover segment files
    /// - `missing_subs`: completed videos missing expected subtitle files
    ///
    /// # Note on .part files
    ///
    /// - HTTP chunks: `filename.mp4.part0` - few large files, resumable
    /// - HLS segments: `filename.part0` - many small files, will be cleaned up
    ///
    /// Since HLS segments are cleaned up before each download, we just report
    /// the count for user information.
    pub(super) async fn detect_existing_playlist_files(
        &self,
        playlist_dir: &std::path::Path,
        infos: &[rdlp_core::InfoDict],
        subtitle_langs: &[String],
        strict_subs: bool,
    ) -> ResumeDetection {
        let mut completed = HashMap::new();
        let mut partial_count = 0;
        let mut missing_subs = HashMap::new();

        if !playlist_dir.exists() {
            return ResumeDetection {
                completed,
                partial_count,
                missing_subs,
            };
        }

        // Get all files in the playlist directory
        let mut dir_entries = match tokio::fs::read_dir(playlist_dir).await {
            Ok(entries) => entries,
            Err(_) => {
                return ResumeDetection {
                    completed,
                    partial_count,
                    missing_subs,
                };
            }
        };

        let mut files: Vec<PathBuf> = Vec::new();
        while let Ok(Some(entry)) = dir_entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }

        // Check each video in the playlist
        for info in infos {
            let sanitized_title = self.sanitize_filename(&info.title);

            // Look for completed file matching this title
            let mut found_complete = false;
            let mut found_partial = false;

            for file_path in &files {
                let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");

                // Check for .part files (HLS segments or HTTP chunks)
                // HLS segments: filename.part{n} (no extension before .part)
                // HTTP chunks: filename.mp4.part{n} (extension before .part)
                if filename.starts_with(&sanitized_title) && filename.contains(".part") {
                    found_partial = true;
                    continue;
                }

                // Check for completed file
                if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == sanitized_title {
                        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

                        // Skip .part files
                        if ext.contains("part") {
                            found_partial = true;
                            continue;
                        }

                        // Reject .ts files — HLS remux didn't complete
                        if ext.eq_ignore_ascii_case("ts") {
                            debug!(
                                file:% = filename;
                                "Skipping .ts file (remux incomplete)"
                            );
                            found_partial = true;
                            continue;
                        }

                        // Check if file has reasonable size (> 1MB to avoid empty/corrupted files)
                        if let Ok(metadata) = file_path.metadata() {
                            if metadata.len() > 1_000_000 {
                                // Check subtitle file existence once for both
                                // strict and lenient paths. Uses fuzzy matching
                                // so "en" finds "title.English.vtt".
                                let subs_missing = if !subtitle_langs.is_empty() {
                                    let dir = file_path.parent().unwrap_or(playlist_dir);
                                    !subtitle_langs
                                        .iter()
                                        .any(|lang| has_subtitle_file(dir, &sanitized_title, lang))
                                } else {
                                    false
                                };

                                // Strict mode: missing subs invalidate the video
                                if strict_subs && subs_missing {
                                    debug!(
                                        file:% = filename;
                                        "Skipping: missing subtitle file (strict mode)"
                                    );
                                    found_partial = true;
                                    continue;
                                }

                                // Lenient mode: track for reporting / --retry-subs
                                if subs_missing {
                                    missing_subs.insert(sanitized_title.clone(), file_path.clone());
                                }

                                completed.insert(sanitized_title.clone(), file_path.clone());
                                found_complete = true;
                                break;
                            } else {
                                // File exists but is too small - treat as partial
                                found_partial = true;
                            }
                        }
                    }
                }
            }

            if !found_complete && found_partial {
                partial_count += 1;
            }
        }

        ResumeDetection {
            completed,
            partial_count,
            missing_subs,
        }
    }

    /// Retry subtitle downloads for completed videos that are missing subs.
    ///
    /// Re-extracts fresh metadata for each episode and downloads subtitles
    /// using the structured pipeline. Returns the count of episodes that
    /// had subtitles successfully recovered.
    async fn retry_missing_subtitles(
        &self,
        missing_subs: &HashMap<String, PathBuf>,
        infos: &[rdlp_core::InfoDict],
        sub_langs: &[String],
    ) -> usize {
        let count = missing_subs.len();
        info!("Retrying subtitle download for {count} episode(s) with missing subs");

        let mut recovered = 0usize;
        for (sanitized_title, video_path) in missing_subs {
            // Find the matching InfoDict by sanitized title
            let Some(ep_info) = infos
                .iter()
                .find(|i| self.sanitize_filename(&i.title) == *sanitized_title)
            else {
                warn!(
                    title:% = sanitized_title;
                    "Could not find episode metadata for subtitle retry"
                );
                continue;
            };

            // Re-extract fresh metadata to get fresh subtitle URLs
            let fresh_info = match self.extract_lazy_formats(&ep_info.webpage_url).await {
                Ok(mut resolved) => {
                    // Preserve episode metadata from original stub
                    resolved.id = ep_info.id.clone();
                    resolved.title = ep_info.title.clone();
                    resolved.playlist = ep_info.playlist.clone();
                    resolved.playlist_title = ep_info.playlist_title.clone();
                    resolved.playlist_index = ep_info.playlist_index;
                    resolved.playlist_count = ep_info.playlist_count;
                    resolved
                }
                Err(e) => {
                    warn!(
                        title:? = ep_info.title;
                        "Failed to re-extract for subtitle retry: {e}"
                    );
                    continue;
                }
            };

            // Resolve per-episode subtitle URLs from language names
            let episode_subs = self.resolve_subtitles_for_episode(&fresh_info, sub_langs);

            let downloaded_subs = match self
                .download_subtitles(&fresh_info, video_path, &episode_subs)
                .await
            {
                Ok(subs) => subs,
                Err(e) => {
                    warn!(
                        title:? = ep_info.title;
                        "Subtitle retry download failed: {e}"
                    );
                    continue;
                }
            };

            if !downloaded_subs.is_empty() {
                info!(
                    title:? = ep_info.title,
                    count = downloaded_subs.len();
                    "Subtitle retry succeeded"
                );
                recovered += 1;
            } else {
                warn!(
                    title:? = ep_info.title;
                    "Subtitle retry: no subtitles downloaded"
                );
            }
        }

        recovered
    }

    /// Log per-episode missing subtitle details.
    ///
    /// Uses `sanitize_filename` to match infos against `missing_subs` keys.
    fn log_missing_subs(
        &self,
        missing_subs: &HashMap<String, PathBuf>,
        infos: &[rdlp_core::InfoDict],
        total: usize,
    ) {
        for info in infos {
            let sanitized = self.sanitize_filename(&info.title);
            if missing_subs.contains_key(&sanitized) {
                let pos = info.playlist_index.unwrap_or(0);
                warn!("  [{pos}/{total}] {}: no subtitle files found", info.title);
            }
        }
    }

    /// Download from pre-extracted InfoDict (internal helper)
    ///
    /// This method skips the extraction phase and starts from format selection.
    /// Used by playlist downloads to avoid re-extracting already-fetched metadata.
    #[allow(dead_code)]
    pub(super) async fn download_from_info(
        &self,
        info: &rdlp_core::InfoDict,
        interactive: bool,
    ) -> Result<Option<PathBuf>> {
        self.download_from_info_to_dir(
            info,
            interactive,
            &self.config.output_directory,
            &[],
            None,
            None,
        )
        .await
    }

    /// Download from pre-extracted InfoDict to a specific directory
    ///
    /// This method is used by playlist downloads to save files to the playlist folder.
    /// Uses the shared CDN fallback loop via `download_with_cdn_fallback()`.
    ///
    /// Supports lazy format resolution: if `info.formats` is empty and
    /// `info.webpage_url` is set, formats are resolved on-the-fly via
    /// `extract_video_info()`.
    ///
    /// On CDN failure (invalid M3U8, 403/503), re-extracts fresh URLs up to
    /// `MAX_EXTRACT_RETRIES` times with a delay between attempts. This mirrors
    /// yt-dlp's `--extractor-retries` behavior.
    ///
    /// # Arguments
    /// * `info` - Pre-extracted video metadata
    /// * `interactive` - Whether interactive mode is active
    /// * `output_dir` - Directory to save the file in
    /// * `subtitle_langs` - Pre-selected subtitle language names (empty = config-based)
    /// * `audio_type_filter` - If set, filter lazily-resolved formats to this language
    pub(super) async fn download_from_info_to_dir(
        &self,
        info: &rdlp_core::InfoDict,
        interactive: bool,
        output_dir: &std::path::Path,
        subtitle_langs: &[String],
        audio_type_filter: Option<&str>,
        batch_resolved_at: Option<Instant>,
    ) -> Result<Option<PathBuf>> {
        const MAX_EXTRACT_RETRIES: usize = 3;
        /// Exponential backoff delays: 5s, 15s, 30s — gives Cloudflare time
        /// to clear rate-limits between re-extraction attempts.
        const RETRY_DELAYS: [u64; MAX_EXTRACT_RETRIES] = [5, 15, 30];

        let mut output_path: Option<PathBuf> = None;
        let mut download_outcome = None;
        let mut last_info_ref: Option<rdlp_core::InfoDict> = None;

        for attempt in 0..=MAX_EXTRACT_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(RETRY_DELAYS[attempt - 1]);
                warn!(
                    attempt = attempt + 1,
                    max = MAX_EXTRACT_RETRIES + 1,
                    delay_secs = delay.as_secs();
                    "Re-extracting fresh CDN URLs after backoff"
                );
                tokio::time::sleep(delay).await;
            }

            // Lazy resolve: always re-extract on retry, only if empty on first
            let resolved_info;
            // Re-extract if: retry attempt, no formats, or batch tokens expired
            let tokens_stale =
                attempt == 0 && batch_resolved_at.is_some_and(|t| t.elapsed() > TOKEN_MAX_AGE);
            if tokens_stale {
                info!(
                    "Batch-resolved tokens stale (>{} min), forcing re-extraction",
                    TOKEN_MAX_AGE.as_secs() / 60
                );
            }
            let info_ref = if (attempt > 0 || info.formats.is_empty() || tokens_stale)
                && !info.webpage_url.is_empty()
            {
                info!(title:? = info.title; "Lazily resolving episode formats");
                let mut resolved = match self.extract_lazy_formats(&info.webpage_url).await {
                    Ok(r) => r,
                    Err(e) if attempt < MAX_EXTRACT_RETRIES => {
                        warn!(
                            attempt = attempt + 1;
                            "Extraction failed: {e}, will retry"
                        );
                        continue;
                    }
                    Err(e) => return Err(e),
                };

                // Preserve all episode metadata from the original stub
                resolved.id = info.id.clone();
                resolved.title = info.title.clone();
                resolved.thumbnail = info.thumbnail.clone();
                resolved.description = info.description.clone();
                resolved.playlist = info.playlist.clone();
                resolved.playlist_title = info.playlist_title.clone();
                resolved.playlist_index = info.playlist_index;
                resolved.playlist_count = info.playlist_count;

                // Apply audio type filter to lazily-resolved formats
                if let Some(lang) = audio_type_filter {
                    resolved
                        .formats
                        .retain(|f| f.language.as_deref() == Some(lang));
                }

                resolved_info = resolved;
                &resolved_info
            } else {
                info
            };

            // Select format (non-interactive on retry to avoid re-prompting)
            let Some(mut format) = self
                .select_format(info_ref, interactive && attempt == 0)
                .await?
            else {
                return Ok(None);
            };

            // Add alternative CDN server URLs as fallbacks.
            // Sort by CDN host stickiness: same host as primary URL first,
            // since a working CDN server is more likely to stay healthy.
            let mut alt_urls: Vec<String> = info_ref
                .formats
                .iter()
                .filter(|f| f.url != format.url)
                .map(|f| f.url.clone())
                .collect();
            if !alt_urls.is_empty() {
                let primary_host = extract_cdn_host(&format.url);
                alt_urls.sort_by_key(|url| u8::from(extract_cdn_host(url) != primary_host));
                format
                    .fallback_urls
                    .get_or_insert_with(Vec::new)
                    .extend(alt_urls);
            }

            // Compute output path on first attempt only
            if output_path.is_none() {
                let file_ext = self.determine_file_extension(&format);
                let sanitized_title = self.sanitize_filename(&info_ref.title);
                let filename = format!("{sanitized_title}.{file_ext}");
                let path = output_dir.join(&filename);

                self.cleanup_leftover_segments(output_dir, &sanitized_title)
                    .await;

                info!(path:? = path.display(); "Downloading to");
                output_path = Some(path);
            }

            let path = output_path.as_ref().unwrap();

            // Detect resume point (recalculate on retry for partial HLS)
            let resume_offset = self.detect_resume_point(path, format.filesize).await?;

            // Check if file is already complete
            if let Some(expected_size) = format.filesize {
                if resume_offset == expected_size {
                    info!("File already complete, skipping");
                    return Ok(Some(path.clone()));
                }
            }

            // Save info for post-download use (thumbnail, subtitles)
            last_info_ref = Some(info_ref.clone());

            // Download with CDN fallback
            match self
                .download_with_cdn_fallback(&format, path, resume_offset)
                .await
            {
                Ok(Some(outcome)) => {
                    download_outcome = Some(outcome);
                    break;
                }
                Ok(None) => return Ok(None), // User cancelled
                Err(e) if attempt < MAX_EXTRACT_RETRIES && is_reextractable_error(&e) => {
                    warn!("All CDN URLs failed: {e}");
                    warn!("Will re-extract fresh URLs after delay");
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        let outcome = download_outcome.ok_or_else(|| {
            OrchestratorError::DownloadFailed(rdlp_core::RdlpError::Extraction(
                "All extraction retry attempts exhausted".to_string(),
            ))
        })?;

        let output_path = output_path.unwrap();
        let final_info = last_info_ref.as_ref().unwrap_or(info);

        // Download thumbnail if needed (before post-processing so embed can find it)
        if self.config.embed_thumbnail || self.config.write_thumbnail {
            self.download_thumbnail(final_info, &output_path).await;
        }

        // Download subtitles: resolve per-episode URLs from language names.
        // If subtitles were selected but none downloaded, fail the episode
        // so the retry system can re-attempt with fresh URLs.
        let episode_subs = if !subtitle_langs.is_empty() {
            self.resolve_subtitles_for_episode(final_info, subtitle_langs)
        } else {
            Vec::new()
        };
        let downloaded_subs = self
            .download_subtitles(final_info, &output_path, &episode_subs)
            .await?;

        if !subtitle_langs.is_empty() && downloaded_subs.is_empty() {
            if self.config.strict_subs {
                return Err(OrchestratorError::DownloadFailed(
                    rdlp_core::RdlpError::Extraction(format!(
                        "Strict subtitle mode: no subtitles downloaded for '{}' \
                         (expected [{}])",
                        final_info.title,
                        subtitle_langs.join(", ")
                    )),
                ));
            }
            warn!(
                title:? = final_info.title,
                expected:% = subtitle_langs.join(", ");
                "Subtitles unavailable for this episode, continuing without"
            );
        }

        // Run post-processing if configured (or automatic for HLS)
        let final_files = self
            .run_postprocessing(final_info, vec![output_path.clone()], outcome.is_hls)
            .await?;
        let final_path = final_files.into_iter().next().unwrap_or(output_path);

        // HLS downloads produce .ts files that should be remuxed to a proper
        // container. If post-processing left the file as .ts, fail the episode
        // so the retry system can re-attempt (FFmpeg may have been busy/failed).
        if outcome.is_hls {
            let ext = final_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if ext.eq_ignore_ascii_case("ts") {
                return Err(OrchestratorError::DownloadFailed(
                    rdlp_core::RdlpError::Extraction(format!(
                        "Post-processing failed for '{}': \
                         HLS container still .ts (FFmpeg remux did not complete)",
                        final_info.title,
                    )),
                ));
            }
        }

        Ok(Some(final_path))
    }
}

/// Detect distinct audio/language types across all playlist episodes.
///
/// Scans format `language` fields and returns sorted unique values.
/// Used to offer SUB/DUB selection before batch download.
fn detect_audio_types(infos: &[rdlp_core::InfoDict]) -> Vec<String> {
    let types: HashSet<&str> = infos
        .iter()
        .flat_map(|info| &info.formats)
        .filter_map(|f| f.language.as_deref())
        .filter(|lang| !lang.is_empty())
        .collect();

    let mut result: Vec<String> = types.into_iter().map(String::from).collect();
    result.sort();
    result
}

/// Extract the hostname from a URL for CDN server stickiness.
///
/// Returns the host portion (e.g., "s2.netmagcdn.com") so fallback
/// URLs on the same CDN server can be prioritized.
fn extract_cdn_host(url: &str) -> Option<&str> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    after_scheme.split('/').next()
}

/// Filter episode formats to only include the selected audio/language type.
///
/// Removes all formats whose `language` field does not match the given value.
fn filter_formats_by_language(infos: &mut [rdlp_core::InfoDict], language: &str) {
    for info in infos.iter_mut() {
        info.formats
            .retain(|f| f.language.as_deref() == Some(language));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_audio_types_empty() {
        let infos: Vec<rdlp_core::InfoDict> = Vec::new();
        assert!(detect_audio_types(&infos).is_empty());
    }

    #[test]
    fn test_detect_audio_types_single() {
        let mut info = rdlp_core::InfoDict::new("id", "title", "test", "http://example.com");
        let mut fmt = rdlp_core::Format::new(
            "f1",
            "http://example.com/v.m3u8",
            "mp4",
            rdlp_core::DownloadProtocol::M3u8,
        );
        fmt.language = Some("SUB".to_string());
        info.formats = vec![fmt];
        assert_eq!(detect_audio_types(&[info]), vec!["SUB"]);
    }

    #[test]
    fn test_detect_audio_types_multiple() {
        let mut info = rdlp_core::InfoDict::new("id", "title", "test", "http://example.com");
        let mut sub = rdlp_core::Format::new(
            "f1",
            "http://example.com/v.m3u8",
            "mp4",
            rdlp_core::DownloadProtocol::M3u8,
        );
        sub.language = Some("SUB".to_string());
        let mut dub = rdlp_core::Format::new(
            "f2",
            "http://example.com/v2.m3u8",
            "mp4",
            rdlp_core::DownloadProtocol::M3u8,
        );
        dub.language = Some("DUB".to_string());
        info.formats = vec![dub, sub];
        // Should be sorted alphabetically
        assert_eq!(detect_audio_types(&[info]), vec!["DUB", "SUB"]);
    }

    #[test]
    fn test_detect_audio_types_no_language() {
        let mut info = rdlp_core::InfoDict::new("id", "title", "test", "http://example.com");
        let fmt = rdlp_core::Format::new(
            "f1",
            "http://example.com/v.m3u8",
            "mp4",
            rdlp_core::DownloadProtocol::M3u8,
        );
        info.formats = vec![fmt];
        assert!(detect_audio_types(&[info]).is_empty());
    }

    #[test]
    fn test_filter_formats_by_language() {
        let mut info = rdlp_core::InfoDict::new("id", "title", "test", "http://example.com");
        let mut sub = rdlp_core::Format::new(
            "f1",
            "http://example.com/v.m3u8",
            "mp4",
            rdlp_core::DownloadProtocol::M3u8,
        );
        sub.language = Some("SUB".to_string());
        let mut dub = rdlp_core::Format::new(
            "f2",
            "http://example.com/v2.m3u8",
            "mp4",
            rdlp_core::DownloadProtocol::M3u8,
        );
        dub.language = Some("DUB".to_string());
        info.formats = vec![sub, dub];

        let mut infos = vec![info];
        filter_formats_by_language(&mut infos, "SUB");

        assert_eq!(infos[0].formats.len(), 1);
        assert_eq!(infos[0].formats[0].language.as_deref(), Some("SUB"));
    }

    #[test]
    fn test_extract_cdn_host() {
        assert_eq!(
            extract_cdn_host("https://s2.netmagcdn.com/hls/master.m3u8"),
            Some("s2.netmagcdn.com")
        );
        assert_eq!(
            extract_cdn_host("http://cdn.example.com/video.mp4"),
            Some("cdn.example.com")
        );
        assert_eq!(extract_cdn_host("not-a-url"), None);
    }

    #[test]
    fn test_extract_cdn_host_stickiness_sort() {
        let primary = "https://s2.netmagcdn.com/hls/ep1/master.m3u8";
        let primary_host = extract_cdn_host(primary);

        let mut urls = vec![
            "https://s1.netmagcdn.com/hls/ep1/alt.m3u8".to_string(),
            "https://s2.netmagcdn.com/hls/ep1/alt2.m3u8".to_string(),
            "https://s3.netmagcdn.com/hls/ep1/alt3.m3u8".to_string(),
        ];

        urls.sort_by_key(|url| u8::from(extract_cdn_host(url) != primary_host));

        // s2 (same host) should sort first
        assert!(urls[0].contains("s2.netmagcdn.com"));
    }
}
