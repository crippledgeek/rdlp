//! Playlist download functionality
//!
//! Provides batch download support with progress tracking, resume capability,
//! and graceful degradation for failed videos.

use super::{Orchestrator, OrchestratorError, Result};
use log::{error, info};
use std::collections::HashMap;
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

        self.download_playlist_internal(infos, interactive).await
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
        infos: Vec<rdlp_core::InfoDict>,
        interactive: bool,
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
        let (existing_files, partial_count) =
            self.detect_existing_playlist_files(&playlist_dir, &infos);
        let already_downloaded = existing_files.len();
        let remaining = total - already_downloaded;

        println!("\n{}", "=".repeat(60));
        info!(title:? = playlist_title; "Playlist");
        info!(path:? = playlist_dir.display(); "Folder");
        info!(total; "Total videos");

        if already_downloaded > 0 || partial_count > 0 {
            if already_downloaded > 0 {
                info!(already_downloaded; "Already downloaded");
            }
            if partial_count > 0 {
                info!(partial_count; "Leftover segments (will be cleaned up)");
            }
            info!(remaining; "Remaining");
        }

        println!("{}", "=".repeat(60));
        println!();

        // If all videos are already downloaded, return early
        if remaining == 0 {
            info!("All videos already downloaded");
            let paths: Vec<PathBuf> = existing_files.into_values().collect();
            return Ok(Some(paths));
        }

        // Confirm before downloading (unless non-interactive)
        if interactive && remaining > 0 {
            use dialoguer::Confirm;

            let prompt = if already_downloaded > 0 {
                format!("Resume downloading {remaining} remaining videos?")
            } else {
                format!("Download {total} videos to '{playlist_folder_name}'?")
            };

            let proceed = Confirm::new()
                .with_prompt(prompt)
                .default(true)
                .interact()
                .unwrap_or(false);

            if !proceed {
                println!("Cancelled by user");
                return Ok(None);
            }
            println!();
        }

        // Create playlist directory if it doesn't exist
        if !playlist_dir.exists() {
            std::fs::create_dir_all(&playlist_dir).map_err(|e| {
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

            println!("\n{}", "─".repeat(60));
            info!(position, total, title:? = info.title; "Downloading");
            println!("{}", "─".repeat(60));

            // Race download against Ctrl+C signal
            tokio::select! {
                // Download single video to playlist folder (non-interactive)
                result = self.download_from_info_to_dir(info, false, &playlist_dir) => {
                    match result {
                        Ok(Some(path)) => {
                            info!(position, total, path:? = path.display(); "Saved");
                            downloaded.push(path);
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

        info!("");
        info!("{}", "=".repeat(60));
        info!("Playlist Download Summary");
        info!("{}", "=".repeat(60));
        info!(path:? = playlist_dir.display(); "Folder");
        info!(downloaded = downloaded.len(), total; "Total downloaded");

        if already_downloaded > 0 {
            println!("   (previously: {already_downloaded}, this session: {newly_downloaded})");
        }

        if !failed.is_empty() {
            error!(count = failed.len(); "Failed");
            error!("Failed videos:");
            for (pos, title, err) in &failed {
                error!(position = pos, title:?; "Failed video");
                error!(err:?; "Error");
            }
        }

        if interrupted {
            let remaining_after = total - downloaded.len();
            info!(remaining = remaining_after; "Interrupted with videos remaining");
            info!("Run the same command again to resume");
        }

        println!("{}", "=".repeat(60));

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
    pub(super) fn detect_existing_playlist_files(
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
        let dir_entries = match std::fs::read_dir(playlist_dir) {
            Ok(entries) => entries,
            Err(_) => return (completed, partial_count),
        };

        let files: Vec<PathBuf> = dir_entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();

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
        self.download_from_info_to_dir(info, interactive, &self.config.output_directory)
            .await
    }

    /// Download from pre-extracted InfoDict to a specific directory
    ///
    /// This method is used by playlist downloads to save files to the playlist folder.
    pub(super) async fn download_from_info_to_dir(
        &self,
        info: &rdlp_core::InfoDict,
        interactive: bool,
        output_dir: &std::path::Path,
    ) -> Result<Option<PathBuf>> {
        // Select format
        let format = match self.select_format(&info.formats, interactive)? {
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

        let resume_from = resume_offset;

        // Create progress bar with best available size estimate
        // For HLS streams, don't use filesize_approx - it's unreliable since the
        // actual bitrate of the selected variant often differs from the estimate
        let is_hls = format.url.contains(".m3u8") || format.ext == "hls";
        let estimated_size = if is_hls {
            format.filesize // Only use exact size if available (rare for HLS)
        } else {
            format.filesize.or(format.filesize_approx)
        };
        let progress_bar = self.create_progress_bar(estimated_size, resume_from)?;

        // Find downloader
        let downloader = self
            .downloader_registry
            .find_downloader(&format.url)
            .ok_or_else(|| OrchestratorError::NoDownloader {
                url: format.url.clone(),
            })?;

        // Execute download
        let stats = match self
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
            None => return Ok(None),
        };

        // Report success
        info!("Downloaded successfully!");
        info!(path:? = output_path.display(); "   File");
        info!(stats:?; "   Stats");

        // Run post-processing if configured (or automatic for HLS)
        let final_files = self
            .run_postprocessing(info, vec![output_path.clone()], is_hls)
            .await?;
        let final_path = final_files.into_iter().next().unwrap_or(output_path);

        Ok(Some(final_path))
    }
}
