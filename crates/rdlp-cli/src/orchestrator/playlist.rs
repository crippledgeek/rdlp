//! Playlist download functionality
//!
//! Provides batch download support with progress tracking, resume capability,
//! and graceful degradation for failed videos.

use super::{Orchestrator, OrchestratorError, Result, archive};
use log::{debug, error, info};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

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
            .as_deref()
            .unwrap_or("Unnamed Playlist");

        // Create playlist folder
        let playlist_folder_name = self.sanitize_filename(playlist_title);
        let playlist_dir = self.config.output_directory.join(&playlist_folder_name);

        // Check for existing files (resume detection)
        let (existing_files, partial_count) = self
            .detect_existing_playlist_files(&playlist_dir, &infos)
            .await;
        let already_downloaded = existing_files.len();
        let remaining = total - already_downloaded;

        // Detect available audio/language types across all episodes
        let audio_types = detect_audio_types(&infos);

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

        info!("{}", "=".repeat(60));

        // If all videos are already downloaded, return early
        if remaining == 0 {
            info!("All videos already downloaded");
            let paths: Vec<PathBuf> = existing_files.into_values().collect();
            return Ok(Some(paths));
        }

        // Audio type selection for playlists with multiple language tracks
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
                filter_formats_by_language(&mut infos, selected);
            }
        }

        // Interactive subtitle selection (once for the whole playlist, before confirm)
        let list_subs = self.config.list_subs;
        let has_subs = infos[0]
            .subtitles
            .as_ref()
            .is_some_and(|s| !s.is_empty());
        let has_auto = infos[0]
            .automatic_captions
            .as_ref()
            .is_some_and(|a| !a.is_empty());
        debug!(
            has_subs, has_auto, interactive, list_subs;
            "Playlist subtitle selection check"
        );

        let subtitle_selection = if interactive || list_subs {
            // Use the first episode's subtitle data as representative
            match self
                .select_subtitles_if_needed(&infos[0], interactive, list_subs)
                .await?
            {
                Some(sel) => sel,
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

        // Download each video with progress tracking
        // Use tokio::select! to properly catch Ctrl+C during downloads
        let mut downloaded: Vec<PathBuf> = existing_files.into_values().collect();
        let mut failed = Vec::new();
        let mut interrupted = false;

        for (index, info) in infos.iter().enumerate() {
            let position = index + 1;

            // Check if this video is already downloaded
            let sanitized_title = self.sanitize_filename(&info.title);
            let already_exists = downloaded.iter().any(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|name| name == sanitized_title)
            });

            if already_exists {
                info!(position, total, title:? = info.title; "Already downloaded");
                continue;
            }

            // Check download archive
            if let Some(ref archive_set) = archive {
                if archive::is_in_archive(archive_set, &info.extractor, &info.id) {
                    info!(position, total, title:? = info.title; "Already in archive, skipping");
                    continue;
                }
            }

            info!("{}", "─".repeat(60));
            info!(position, total, title:? = info.title; "Downloading");
            info!("{}", "─".repeat(60));

            // Race download against Ctrl+C signal
            tokio::select! {
                // Download single video to playlist folder (non-interactive)
                result = self.download_from_info_to_dir(info, false, &playlist_dir, &subtitle_selection) => {
                    match result {
                        Ok(Some(path)) => {
                            info!(position, total, path:? = path.display(); "Saved");
                            downloaded.push(path);
                            self.record_in_archive(&info.extractor, &info.id);
                        }
                        Ok(None) => {
                            info!(position, total; "Skipped by user");
                        }
                        Err(e) => {
                            error!(position, total; "Failed: {e}");
                            failed.push((position, info.title.clone(), e.to_string()));
                        }
                    }
                }
                // Catch Ctrl+C immediately during download
                _ = tokio::signal::ctrl_c() => {
                    info!("Playlist download interrupted by user");
                    info!("Run the same command again to resume");
                    interrupted = true;
                }
            }

            if interrupted {
                break;
            }
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

        if !subtitle_selection.is_empty() {
            let langs: Vec<&str> = subtitle_selection.iter().map(|(l, _)| l.as_str()).collect();
            info!("Subtitles: {}", langs.join(", "));
        }

        if !failed.is_empty() {
            error!("Failed: {}", failed.len());
            for (pos, title, err) in &failed {
                error!("  [{pos}/{total}] {title}: {err}");
            }
        }

        if interrupted {
            let remaining_after = total - dl_count;
            info!("Interrupted — {remaining_after} videos remaining");
            info!("Run the same command again to resume");
        }

        info!("{}", "=".repeat(60));

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
    /// Returns a tuple of:
    /// - HashMap of sanitized title -> file path for completed downloads
    /// - Count of videos with leftover segment files (will be cleaned up)
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
    ) -> (HashMap<String, PathBuf>, usize) {
        let mut completed = HashMap::new();
        let mut partial_count = 0;

        if !playlist_dir.exists() {
            return (completed, partial_count);
        }

        // Get all files in the playlist directory
        let mut dir_entries = match tokio::fs::read_dir(playlist_dir).await {
            Ok(entries) => entries,
            Err(_) => return (completed, partial_count),
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
                        // Skip .part files
                        if file_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.contains("part"))
                        {
                            found_partial = true;
                            continue;
                        }

                        // Check if file has reasonable size (> 1MB to avoid empty/corrupted files)
                        if let Ok(metadata) = file_path.metadata() {
                            if metadata.len() > 1_000_000 {
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

        (completed, partial_count)
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
        self.download_from_info_to_dir(info, interactive, &self.config.output_directory, &[])
            .await
    }

    /// Download from pre-extracted InfoDict to a specific directory
    ///
    /// This method is used by playlist downloads to save files to the playlist folder.
    /// Uses the shared CDN fallback loop via `download_with_cdn_fallback()`.
    ///
    /// # Arguments
    /// * `info` - Pre-extracted video metadata
    /// * `interactive` - Whether interactive mode is active
    /// * `output_dir` - Directory to save the file in
    /// * `subtitle_selection` - Pre-selected subtitles from interactive menu (empty = config-based)
    pub(super) async fn download_from_info_to_dir(
        &self,
        info: &rdlp_core::InfoDict,
        interactive: bool,
        output_dir: &std::path::Path,
        subtitle_selection: &[(String, rdlp_core::Subtitle)],
    ) -> Result<Option<PathBuf>> {
        // Select format
        let format = match self.select_format(info, interactive).await? {
            Some(format) => format,
            None => return Ok(None),
        };

        // Generate output path in the specified directory
        let file_ext = self.determine_file_extension(&format);
        let sanitized_title = self.sanitize_filename(&info.title);
        let filename = format!("{sanitized_title}.{file_ext}");
        let output_path = output_dir.join(&filename);

        // Clean up any leftover HLS segment files from interrupted downloads
        self.cleanup_leftover_segments(output_dir, &sanitized_title)
            .await;

        info!(path:? = output_path.display(); "Downloading to");

        // Detect resume point
        let resume_offset = self
            .detect_resume_point(&output_path, format.filesize)
            .await?;

        // Check if file is already complete
        if let Some(expected_size) = format.filesize {
            if resume_offset == expected_size {
                info!("File already complete, skipping");
                return Ok(Some(output_path));
            }
        }

        // Execute download with CDN fallback (shared implementation)
        let outcome = match self
            .download_with_cdn_fallback(&format, &output_path, resume_offset)
            .await?
        {
            Some(outcome) => outcome,
            None => return Ok(None),
        };

        // Download thumbnail if needed (before post-processing so embed can find it)
        if self.config.embed_thumbnail || self.config.write_thumbnail {
            self.download_thumbnail(info, &output_path).await;
        }

        // Download subtitles (interactive selection or config-based)
        self.download_subtitles(info, &output_path, subtitle_selection)
            .await?;

        // Run post-processing if configured (or automatic for HLS)
        let final_files = self
            .run_postprocessing(info, vec![output_path.clone()], outcome.is_hls)
            .await?;
        let final_path = final_files.into_iter().next().unwrap_or(output_path);

        Ok(Some(final_path))
    }
}

/// Detect distinct audio/language types across all playlist episodes.
///
/// Scans format `language` fields and returns sorted unique values.
/// Used to offer SUB/DUB selection before batch download.
fn detect_audio_types(infos: &[rdlp_core::InfoDict]) -> Vec<String> {
    let mut types = HashSet::new();
    for info in infos {
        for format in &info.formats {
            if let Some(lang) = &format.language {
                if !lang.is_empty() {
                    types.insert(lang.clone());
                }
            }
        }
    }
    let mut result: Vec<String> = types.into_iter().collect();
    result.sort();
    result
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
}
