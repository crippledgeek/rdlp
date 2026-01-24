//! Orchestrator module for coordinating extraction, download, and post-processing

mod errors;
mod execution;
mod extraction;
mod resume;
mod selection;
mod state;

#[cfg(test)]
mod tests;

// Public re-exports
pub use errors::{OrchestratorError, Result};
pub use state::{DownloadPhase, DownloadState};

use log::{debug, error, info, warn};
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::{Config, ExtractionContext, PostProcessConfig};
use rdlp_downloader::{DownloaderRegistry, DownloaderRegistryTrait};
use rdlp_extractor::{ExtractorRegistry, ExtractorRegistryTrait};
use rdlp_jsinterp::SimpleJsEngine;
use rdlp_postprocess::{PostProcessorRegistry, PostProcessorRegistryTrait};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    pub(super) extractor_registry: Arc<dyn ExtractorRegistryTrait>,
    pub(super) downloader_registry: Arc<dyn DownloaderRegistryTrait>,
    pub(super) postprocessor_registry: Option<Arc<dyn PostProcessorRegistryTrait>>,
    pub(super) extraction_context: Arc<ExtractionContext>,
    pub(super) config: Arc<Config>,
}

/// Default user agent for HTTP requests
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Create a configured HTTP client for extraction
fn create_http_client() -> Arc<reqwest::Client> {
    Arc::new(
        reqwest::Client::builder()
            .user_agent(DEFAULT_USER_AGENT)
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client"),
    )
}

impl Orchestrator {
    /// Create a new orchestrator with default registries
    pub fn new(config: Config) -> Self {
        let http_client = create_http_client();
        let js_engine = Arc::new(SimpleJsEngine::new());
        let cookie_jar = Arc::new(SimpleCookieJar::new());

        // Wrap config in Arc once and share it
        let config = Arc::new(config);

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config), // Cheap Arc clone instead of deep clone
        ));

        // Initialize post-processor registry (optional - graceful degradation if FFmpeg not found)
        let postprocessor_registry = Self::create_postprocessor_registry(&config);

        Self {
            extractor_registry: Arc::new(ExtractorRegistry::new()),
            downloader_registry: Arc::new(DownloaderRegistry::new()),
            postprocessor_registry,
            extraction_context,
            config,
        }
    }

    /// Create post-processor registry with optional FFmpeg location
    ///
    /// Returns None if FFmpeg is not found (graceful degradation)
    fn create_postprocessor_registry(config: &Config) -> Option<Arc<dyn PostProcessorRegistryTrait>> {
        let registry_result = if let Some(ref ffmpeg_path) = config.ffmpeg_location {
            PostProcessorRegistry::with_ffmpeg_location(Some(ffmpeg_path.as_path()))
        } else {
            PostProcessorRegistry::new()
        };

        match registry_result {
            Ok(registry) => {
                debug!("[PostProcess] FFmpeg initialized successfully");
                Some(Arc::new(registry))
            }
            Err(e) => {
                // Warn about FFmpeg not being found (needed for HLS fixup)
                warn!("[PostProcess] FFmpeg NOT found: {e}");
                None
            }
        }
    }

    /// Create a new orchestrator with custom registries (for testing)
    ///
    /// This method is primarily used for integration tests to inject mock registries.
    /// It's public to allow integration tests to use it, but should not be used in
    /// production code.
    #[doc(hidden)]
    pub fn with_registries(
        config: Config,
        extractor_registry: Arc<dyn ExtractorRegistryTrait>,
        downloader_registry: Arc<dyn DownloaderRegistryTrait>,
    ) -> Self {
        let http_client = create_http_client();
        let js_engine = Arc::new(SimpleJsEngine::new());
        let cookie_jar = Arc::new(SimpleCookieJar::new());

        let config = Arc::new(config);

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config),
        ));

        // Initialize post-processor registry (optional)
        let postprocessor_registry = Self::create_postprocessor_registry(&config);

        Self {
            extractor_registry,
            downloader_registry,
            postprocessor_registry,
            extraction_context,
            config,
        }
    }

    /// Download a video from URL using state machine pattern
    ///
    /// # State Machine Workflow
    ///
    /// This method implements an explicit state machine for the download workflow:
    /// 1. `Extracting` - Find extractor and extract video metadata
    /// 2. `SelectingFormat` - Choose format (interactive or automatic)
    /// 3. `Preparing` - Detect resume point and prepare for download
    /// 4. `Downloading` - Execute download with progress tracking
    /// 5. `Complete` - Return downloaded file path
    ///
    /// At any point, user can cancel (Ctrl+C or ESC) → `Cancelled` state
    ///
    /// **Note**: This method now auto-detects playlists! If the URL is a playlist,
    /// it will automatically delegate to `download_playlist()` and return the first
    /// video path (or None if playlist was empty/cancelled).
    ///
    /// # Returns
    ///
    /// - `Ok(Some(path))` - Download completed successfully
    /// - `Ok(None)` - User cancelled operation
    /// - `Err` - Error occurred during any phase
    pub async fn download(&self, url: &str, interactive: bool) -> Result<Option<PathBuf>> {
        // Try playlist extraction first to check if this is a playlist
        let infos = self.extract_playlist_info(url).await?;

        // If multiple videos found, this is a playlist
        if infos.len() > 1 {
            return self
                .download_playlist_internal(infos, interactive)
                .await
                .map(|opt| opt.and_then(|paths| paths.into_iter().next()));
        }

        // Single video - use existing state machine
        let mut phase = DownloadPhase::Extracting {
            url: url.to_string(),
        };

        loop {
            phase = phase.advance(self, interactive).await?;

            match phase {
                DownloadPhase::Complete { path } => return Ok(Some(path)),
                DownloadPhase::Cancelled => return Ok(None),
                _ => continue, // Keep advancing through phases
            }
        }
    }

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
    pub async fn download_playlist(&self, url: &str, interactive: bool) -> Result<Option<Vec<PathBuf>>> {
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
    async fn download_playlist_internal(
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
        let (existing_files, partial_count) = self.detect_existing_playlist_files(&playlist_dir, &infos);
        let already_downloaded = existing_files.len();
        let remaining = total - already_downloaded;

        println!("\n{}", "=".repeat(60));
        info!("Playlist: {playlist_title}");
        info!("Folder: {}", playlist_dir.display());
        info!("Total videos: {total}");

        if already_downloaded > 0 || partial_count > 0 {
            if already_downloaded > 0 {
                info!("Already downloaded: {already_downloaded}");
            }
            if partial_count > 0 {
                info!("Leftover segments: {partial_count} (will be cleaned up)");
            }
            info!("Remaining: {remaining}");
        }

        println!("{}", "=".repeat(60));
        println!();

        // If all videos are already downloaded, return early
        if remaining == 0 {
            info!("All videos already downloaded!");
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
            info!("Created folder: {}", playlist_dir.display());
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
                info!("[{position}/{total}] Already downloaded: {}", info.title);
                continue;
            }

            println!("\n{}", "─".repeat(60));
            info!("[{}/{}] Downloading: {}", position, total, info.title);
            println!("{}", "─".repeat(60));

            // Race download against Ctrl+C signal
            tokio::select! {
                // Download single video to playlist folder (non-interactive)
                result = self.download_from_info_to_dir(info, false, &playlist_dir) => {
                    match result {
                        Ok(Some(path)) => {
                            info!("[{}/{}] Saved: {}", position, total, path.display());
                            downloaded.push(path);
                        }
                        Ok(None) => {
                            info!("[{position}/{total}] Skipped by user");
                        }
                        Err(e) => {
                            error!("[{position}/{total}] Failed: {e}");
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
        info!("Folder: {}", playlist_dir.display());
        info!("Total downloaded: {}/{}", downloaded.len(), total);

        if already_downloaded > 0 {
            println!("   (previously: {already_downloaded}, this session: {newly_downloaded})");
        }

        if !failed.is_empty() {
            error!("Failed: {}", failed.len());
            error!("Failed videos:");
            for (pos, title, err) in &failed {
                error!("   [{pos}] {title}");
                error!("       Error: {err}");
            }
        }

        if interrupted {
            let remaining_after = total - downloaded.len();
            info!("Interrupted with {remaining_after} videos remaining");
            info!("Run the same command again to resume");
        }

        println!("{}", "=".repeat(60));

        if downloaded.is_empty() {
            Err(OrchestratorError::ExtractionFailed(
                rdlp_core::RdlpError::Extraction("All playlist videos failed to download".to_string()),
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
    fn detect_existing_playlist_files(
        &self,
        playlist_dir: &std::path::Path,
        infos: &[rdlp_core::InfoDict],
    ) -> (std::collections::HashMap<String, PathBuf>, usize) {
        let mut completed = std::collections::HashMap::new();
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
                        if file_path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.contains("part")) {
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
    async fn download_from_info(
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
    async fn download_from_info_to_dir(
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
        self.cleanup_leftover_segments(output_dir, &sanitized_title).await;

        info!("Downloading to: {}", output_path.display());

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
            .execute_download(&downloader, &format.url, &output_path, resume_from, progress_bar.as_ref(), estimated_size)
            .await?
        {
            Some(stats) => stats,
            None => return Ok(None),
        };

        // Report success
        info!("Downloaded successfully!");
        info!("   File: {}", output_path.display());
        info!("   Stats: {stats}");

        // Run post-processing if configured (or automatic for HLS)
        let final_files = self.run_postprocessing(info, vec![output_path.clone()], is_hls).await?;
        let final_path = final_files.into_iter().next().unwrap_or(output_path);

        Ok(Some(final_path))
    }

    /// List all available extractors
    pub fn list_extractors(&self) -> Vec<String> {
        self.extractor_registry.list_extractors()
    }

    /// List all available download protocols
    pub fn list_downloaders(&self) -> Vec<String> {
        self.downloader_registry.list_downloaders()
    }

    /// Generate output file path
    pub(super) fn generate_output_path(
        &self,
        info: &rdlp_core::InfoDict,
        format: &rdlp_core::Format,
    ) -> Result<PathBuf> {
        // Determine the actual file extension
        let file_ext = self.determine_file_extension(format);

        let filename = format!("{}.{}", self.sanitize_filename(&info.title), file_ext);

        let mut path = self.config.output_directory.clone();
        path.push(filename);

        Ok(path)
    }

    /// Determine the actual file extension for a format
    ///
    /// For streaming protocols (HLS, DASH), detects the actual container format
    /// from the format metadata or segment URLs.
    fn determine_file_extension(&self, format: &rdlp_core::Format) -> String {
        // Priority 1: Use container field if explicitly set
        if let Some(ref container) = format.container {
            // Clean up container names (e.g., "mp4_dash" -> "mp4")
            return container
                .split('_')
                .next()
                .unwrap_or(container)
                .to_string();
        }

        // Priority 2: For HLS/DASH, detect from URL or default to mp4
        match format.ext.as_str() {
            "hls" | "m3u8" => {
                // Try to detect from URL (e.g., .../segment.ts)
                if format.url.contains(".ts") {
                    "ts".to_string()  // MPEG-TS segments
                } else {
                    "mp4".to_string()  // fMP4 segments (.m4s/.mp4) or default
                }
            }
            "dash" | "mpd" => {
                // DASH typically uses fMP4
                if format.url.contains(".webm") {
                    "webm".to_string()
                } else {
                    "mp4".to_string()  // Default to MP4 for DASH
                }
            }
            ext => ext.to_string(),  // Use extension as-is for direct formats
        }
    }

    /// Maximum filename length (conservative limit for cross-platform compatibility)
    const MAX_FILENAME_LENGTH: usize = 200;

    /// Windows reserved filenames (case-insensitive)
    const WINDOWS_RESERVED_NAMES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    /// Sanitize filename for safe filesystem usage
    ///
    /// This function provides comprehensive protection against:
    /// - Path traversal attacks (removes `/`, `\`, `:`)
    /// - Invalid filesystem characters (`*`, `?`, `"`, `<`, `>`, `|`)
    /// - Null bytes and control characters
    /// - Windows reserved filenames (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
    /// - Leading/trailing dots and spaces
    /// - Excessive filename length (truncated to 200 chars)
    ///
    /// # Security
    ///
    /// This function is critical for security. Never use unsanitized filenames
    /// directly from external sources (video titles, URLs, etc.).
    pub(super) fn sanitize_filename(&self, name: &str) -> String {
        // Step 1: Replace invalid filesystem characters and filter control characters
        let sanitized: String = name
            .chars()
            .filter_map(|c| {
                // Filter out null bytes and control characters (except space)
                if c == '\0' || (c.is_control() && c != ' ') {
                    return None;
                }
                // Replace invalid filesystem characters with underscore
                Some(match c {
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    _ => c,
                })
            })
            .collect();

        // Step 2: Trim leading/trailing dots and spaces (problematic on Windows)
        let trimmed = sanitized.trim_matches(|c| c == '.' || c == ' ');

        // Step 3: Check for Windows reserved names
        let base_name = if let Some(dot_pos) = trimmed.rfind('.') {
            &trimmed[..dot_pos]
        } else {
            trimmed
        };

        let result = if Self::WINDOWS_RESERVED_NAMES
            .iter()
            .any(|&reserved| base_name.eq_ignore_ascii_case(reserved))
        {
            // Prefix with underscore to avoid reserved name collision
            format!("_{trimmed}")
        } else {
            trimmed.to_string()
        };

        // Step 4: Handle empty result
        let result = if result.is_empty() {
            "unnamed".to_string()
        } else {
            result
        };

        // Step 5: Truncate to maximum length (preserving extension if possible)
        if result.len() > Self::MAX_FILENAME_LENGTH {
            Self::truncate_filename(&result, Self::MAX_FILENAME_LENGTH)
        } else {
            result
        }
    }

    /// Truncate filename while preserving extension
    fn truncate_filename(name: &str, max_len: usize) -> String {
        if let Some(dot_pos) = name.rfind('.') {
            let ext = &name[dot_pos..];
            // Only preserve extension if it's reasonable length (< 10 chars)
            if ext.len() < 10 && dot_pos > 0 {
                let base_max = max_len.saturating_sub(ext.len());
                if base_max > 0 {
                    let base = &name[..dot_pos];
                    // Truncate at char boundary
                    let truncated_base: String = base.chars().take(base_max).collect();
                    return format!("{truncated_base}{ext}");
                }
            }
        }
        // Fallback: simple truncation at char boundary
        name.chars().take(max_len).collect()
    }

    /// Convert Config to PostProcessConfig
    fn to_postprocess_config(&self) -> PostProcessConfig {
        PostProcessConfig {
            extract_audio: self.config.extract_audio,
            audio_format: self.config.audio_format.clone(),
            audio_quality: self.config.audio_quality.clone(),
            recode_video: self.config.recode_video.clone(),
            merge_output_format: self.config.merge_output_format.clone(),
            embed_thumbnail: self.config.embed_thumbnail,
            embed_metadata: self.config.embed_metadata,
            embed_subtitles: self.config.embed_subtitles,
            keep_video: self.config.keep_video,
            ffmpeg_location: self.config.ffmpeg_location.clone(),
            ffmpeg_args: self.config.ffmpeg_args.clone(),
        }
    }

    /// Check if post-processing is needed based on configuration
    fn needs_postprocessing(&self) -> bool {
        self.config.extract_audio
            || self.config.embed_metadata
            || self.config.embed_thumbnail
            || self.config.recode_video.is_some()
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
    async fn run_postprocessing(
        &self,
        info: &rdlp_core::InfoDict,
        files: Vec<PathBuf>,
        is_hls: bool,
    ) -> Result<Vec<PathBuf>> {
        debug!("[PostProcess] Called: is_hls={is_hls}, registry={}", self.postprocessor_registry.is_some());

        let registry = match &self.postprocessor_registry {
            Some(r) => r,
            None => {
                // No post-processor available - return files unchanged
                if self.needs_postprocessing() || is_hls {
                    warn!("Post-processing unavailable (FFmpeg not found)");
                    if is_hls {
                        warn!("HLS downloads may have container issues without FFmpeg remux");
                    }
                }
                return Ok(files);
            }
        };

        // For HLS downloads, always run FFmpeg remux to fix container
        // For other downloads, only run if explicitly configured
        let needs_processing = self.needs_postprocessing() || is_hls;
        if !needs_processing {
            return Ok(files);
        }

        // Build config - for HLS, enable remux even if not explicitly requested
        let mut pp_config = self.to_postprocess_config();
        if is_hls && pp_config.recode_video.is_none() && !pp_config.extract_audio {
            // Set merge_output_format to trigger FFmpeg remux for container fixup
            // This ensures proper moov atom placement and timestamp fixing
            pp_config.merge_output_format = Some("mp4".to_string());
        }

        info!("Running post-processing pipeline...");

        if self.config.verbose {
            let processors = registry.list_processors();
            println!("   Available processors: {}", processors.join(", "));
        }

        // Run FFmpeg remux to fix container (faststart, timestamps)
        // Applied to both HLS and HTTP downloads for consistent output
        let result_files = if !self.config.extract_audio {
            self.ffmpeg_remux(&files).await.unwrap_or(files.clone())
        } else {
            files.clone()
        };

        // Run the full post-processing pipeline
        match registry.process(info, result_files.clone(), &pp_config).await {
            Ok(result) => {
                if result.files != files {
                    info!("Post-processing complete");
                    if self.config.verbose {
                        for file in &result.files {
                            debug!("Output: {}", file.display());
                        }
                    }
                }
                Ok(result.files)
            }
            Err(e) => {
                warn!("Post-processing failed: {e}");
                // Return remuxed files on failure (or original if remux failed)
                Ok(result_files)
            }
        }
    }

    /// Run FFmpeg remux on downloaded files to fix container format
    ///
    /// This performs a stream copy (no re-encoding) to:
    /// - Move moov atom to beginning of file (faststart)
    /// - Fix timestamps
    /// - Ensure proper MP4 container structure
    ///
    /// Applied to both HLS and HTTP downloads for consistent output quality.
    async fn ffmpeg_remux(&self, files: &[PathBuf]) -> Option<Vec<PathBuf>> {
        let registry = self.postprocessor_registry.as_ref()?;
        let ffmpeg = registry.list_processors(); // Just to verify FFmpeg is available
        if ffmpeg.is_empty() {
            return None;
        }

        let mut output_files = Vec::new();

        for file in files {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
            let temp_path = file.with_extension(format!("fixed.{ext}"));

            // Run FFmpeg remux: -c copy -movflags +faststart
            let result = tokio::process::Command::new("ffmpeg")
                .args([
                    "-y",                       // Overwrite output
                    "-i", &file.to_string_lossy(),
                    "-c", "copy",               // Stream copy (no re-encoding)
                    "-movflags", "+faststart",  // Move moov atom to start
                    "-f", "mp4",                // Force MP4 format
                    &temp_path.to_string_lossy(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => {
                    // Replace original with fixed file
                    if let Err(e) = tokio::fs::remove_file(file).await {
                        warn!("Could not remove original file: {e}");
                    }
                    if let Err(e) = tokio::fs::rename(&temp_path, file).await {
                        warn!("Could not rename fixed file: {e}");
                        output_files.push(temp_path);
                    } else {
                        info!("Post-processed: faststart enabled, container fixed");
                        output_files.push(file.clone());
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    debug!("FFmpeg remux failed: {stderr}");
                    // Clean up temp file
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    output_files.push(file.clone());
                }
                Err(e) => {
                    debug!("FFmpeg not available: {e}");
                    output_files.push(file.clone());
                }
            }
        }

        Some(output_files)
    }

    /// Run post-processing for state machine downloads (single videos)
    ///
    /// Runs FFmpeg remux on all downloads for consistent output quality:
    /// - Faststart (moov atom at beginning)
    /// - Fixed timestamps
    /// - Proper MP4 container structure
    ///
    /// Also runs user-requested post-processing (extract audio, embed metadata, etc.)
    pub(super) async fn run_postprocessing_for_state_machine(
        &self,
        output_path: &Path,
        is_hls: bool,
    ) -> Result<PathBuf> {
        // Run FFmpeg remux on all downloads (both HLS and HTTP) for consistent output
        // This moves moov atom to start (faststart), fixes timestamps, ensures proper container
        if let Some(fixed_files) = self.ffmpeg_remux(&[output_path.to_path_buf()]).await {
            if let Some(fixed_path) = fixed_files.into_iter().next() {
                // If user also requested additional post-processing, run it
                if self.needs_postprocessing() {
                    let info = rdlp_core::InfoDict::new(
                        "unknown".to_string(),
                        fixed_path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        "unknown".to_string(),
                        "unknown".to_string(),
                    );

                    let result = self.run_postprocessing(&info, vec![fixed_path.clone()], is_hls).await?;
                    if let Some(path) = result.into_iter().next() {
                        return Ok(path);
                    }
                }
                return Ok(fixed_path);
            }
        }

        // If FFmpeg remux failed, check if user requested any post-processing
        if self.needs_postprocessing() {
            // Create a minimal InfoDict for post-processing
            let info = rdlp_core::InfoDict::new(
                "unknown".to_string(),
                output_path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                "unknown".to_string(),
                "unknown".to_string(),
            );

            let result = self.run_postprocessing(&info, vec![output_path.to_path_buf()], is_hls).await?;
            if let Some(path) = result.into_iter().next() {
                return Ok(path);
            }
        }

        Ok(output_path.to_path_buf())
    }

    /// Clean up leftover HLS segment files from interrupted downloads
    ///
    /// When HLS downloads are interrupted via Ctrl+C, segment files like
    /// `filename.part0`, `filename.part1`, etc. may be left behind.
    /// This function removes them before starting a new download.
    async fn cleanup_leftover_segments(&self, dir: &std::path::Path, base_name: &str) {
        if !dir.exists() {
            return;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        let mut deleted = 0;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                // Match pattern: base_name.part{number}
                if filename.starts_with(base_name) && filename.contains(".part") {
                    // Verify it's a segment file (has numeric suffix after .part)
                    if let Some(part_idx) = filename.rfind(".part") {
                        let suffix = &filename[part_idx + 5..];
                        if suffix.chars().all(|c| c.is_ascii_digit())
                            && std::fs::remove_file(&path).is_ok()
                        {
                            deleted += 1;
                        }
                    }
                }
            }
        }

        if deleted > 0 {
            info!("Cleaned up {deleted} leftover segment files");
        }
    }
}
