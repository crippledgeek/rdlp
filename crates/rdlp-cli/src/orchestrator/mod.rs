//! Orchestrator module for coordinating extraction, download, and post-processing

mod errors;
mod state;
mod extraction;
mod selection;
mod resume;
mod execution;

// Public re-exports
pub use errors::{OrchestratorError, Result};
pub use state::{DownloadPhase, DownloadState};

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

impl Orchestrator {
    /// Create a new orchestrator with default registries
    pub fn new(config: Config) -> Self {
        let http_client = Arc::new(
            reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(std::time::Duration::from_secs(60))        // Total request timeout
                .connect_timeout(std::time::Duration::from_secs(10)) // Connection timeout
                .build()
                .expect("Failed to build HTTP client")
        );
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
                // Always print FFmpeg status for debugging
                eprintln!("[PostProcess] FFmpeg initialized successfully");
                Some(Arc::new(registry))
            }
            Err(e) => {
                // Always warn about FFmpeg not being found (needed for HLS fixup)
                eprintln!("[PostProcess] FFmpeg NOT found: {e}");
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
        let http_client = Arc::new(
            reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(std::time::Duration::from_secs(60))        // Total request timeout
                .connect_timeout(std::time::Duration::from_secs(10)) // Connection timeout
                .build()
                .expect("Failed to build HTTP client")
        );
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
        println!("📋 Playlist: {playlist_title}");
        println!("📁 Folder: {}", playlist_dir.display());
        println!("📊 Total videos: {total}");

        if already_downloaded > 0 || partial_count > 0 {
            if already_downloaded > 0 {
                println!("✅ Already downloaded: {already_downloaded}");
            }
            if partial_count > 0 {
                println!("🧹 Leftover segments: {partial_count} (will be cleaned up)");
            }
            println!("📥 Remaining: {remaining}");
        }

        println!("{}", "=".repeat(60));
        println!();

        // If all videos are already downloaded, return early
        if remaining == 0 {
            println!("✅ All videos already downloaded!");
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
            println!("📁 Created folder: {}", playlist_dir.display());
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
                println!("⏭️  [{position}/{total}] Already downloaded: {}", info.title);
                continue;
            }

            println!("\n{}", "─".repeat(60));
            println!("📥 [{}/{}] {}", position, total, info.title);
            println!("{}", "─".repeat(60));

            // Race download against Ctrl+C signal
            tokio::select! {
                // Download single video to playlist folder (non-interactive)
                result = self.download_from_info_to_dir(info, false, &playlist_dir) => {
                    match result {
                        Ok(Some(path)) => {
                            println!("✅ [{}/{}] Saved: {}", position, total, path.display());
                            downloaded.push(path);
                        }
                        Ok(None) => {
                            println!("⏭️  [{position}/{total}] Skipped by user");
                        }
                        Err(e) => {
                            eprintln!("❌ [{position}/{total}] Failed: {e}");
                            failed.push((position, info.title.clone(), e.to_string()));
                        }
                    }
                }
                // Catch Ctrl+C immediately during download
                _ = tokio::signal::ctrl_c() => {
                    println!("\n⏸️  Playlist download interrupted by user");
                    println!("💾 Run the same command again to resume");
                    interrupted = true;
                }
            }

            if interrupted {
                break;
            }
        }

        // Summary report
        let newly_downloaded = downloaded.len() - already_downloaded;

        println!("\n{}", "=".repeat(60));
        println!("📋 Playlist Download Summary");
        println!("{}", "=".repeat(60));
        println!("📁 Folder: {}", playlist_dir.display());
        println!("✅ Total downloaded: {}/{}", downloaded.len(), total);

        if already_downloaded > 0 {
            println!("   (previously: {already_downloaded}, this session: {newly_downloaded})");
        }

        if !failed.is_empty() {
            println!("❌ Failed: {}", failed.len());
            println!("\nFailed videos:");
            for (pos, title, error) in &failed {
                println!("   [{pos}] {title}");
                println!("       Error: {error}");
            }
        }

        if interrupted {
            let remaining_after = total - downloaded.len();
            println!("\n⏸️  Interrupted with {remaining_after} videos remaining");
            println!("💡 Run the same command again to resume");
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

        println!("💾 Downloading to: {}", output_path.display());

        // Detect resume point
        let resume_offset = self
            .detect_resume_point(&output_path, format.filesize)
            .await?;

        // Check if file is already complete
        if let Some(expected_size) = format.filesize {
            if resume_offset == expected_size {
                println!("✓ File already complete, skipping");
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
        println!("\n✅ Downloaded successfully!");
        println!("   File: {}", output_path.display());
        println!("   Stats: {stats}");

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
        // Debug: ALWAYS print to confirm function is called
        eprintln!("[PostProcess] Called: is_hls={is_hls}, registry={}", self.postprocessor_registry.is_some());

        let registry = match &self.postprocessor_registry {
            Some(r) => r,
            None => {
                // No post-processor available - return files unchanged
                if self.needs_postprocessing() || is_hls {
                    eprintln!("⚠️  Post-processing unavailable (FFmpeg not found)");
                    if is_hls {
                        eprintln!("   HLS downloads may have container issues without FFmpeg remux");
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

        if is_hls {
            println!("🔧 Running FFmpeg fixup for HLS download...");
        } else {
            println!("🔧 Running post-processing pipeline...");
        }

        if self.config.verbose {
            let processors = registry.list_processors();
            println!("   Available processors: {}", processors.join(", "));
        }

        // For HLS, run a simple FFmpeg remux to fix the container
        let result_files = if is_hls && !self.config.extract_audio {
            self.ffmpeg_remux_hls(&files).await.unwrap_or(files.clone())
        } else {
            files.clone()
        };

        // Run the full post-processing pipeline
        match registry.process(info, result_files.clone(), &pp_config).await {
            Ok(result) => {
                if result.files != files {
                    println!("✓ Post-processing complete");
                    if self.config.verbose {
                        for file in &result.files {
                            println!("   Output: {}", file.display());
                        }
                    }
                }
                Ok(result.files)
            }
            Err(e) => {
                eprintln!("⚠️  Post-processing failed: {e}");
                // Return remuxed files on failure (or original if remux failed)
                Ok(result_files)
            }
        }
    }

    /// Run FFmpeg remux on HLS downloaded files to fix container format
    ///
    /// This performs a stream copy (no re-encoding) to:
    /// - Move moov atom to beginning of file (faststart)
    /// - Fix timestamps
    /// - Ensure proper MP4 container structure
    async fn ffmpeg_remux_hls(&self, files: &[PathBuf]) -> Option<Vec<PathBuf>> {
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
                        eprintln!("   Warning: Could not remove original file: {e}");
                    }
                    if let Err(e) = tokio::fs::rename(&temp_path, file).await {
                        eprintln!("   Warning: Could not rename fixed file: {e}");
                        output_files.push(temp_path);
                    } else {
                        println!("   ✓ Container fixed: {}", file.display());
                        output_files.push(file.clone());
                    }
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if self.config.verbose {
                        eprintln!("   FFmpeg remux failed: {stderr}");
                    }
                    // Clean up temp file
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    output_files.push(file.clone());
                }
                Err(e) => {
                    if self.config.verbose {
                        eprintln!("   FFmpeg not available: {e}");
                    }
                    output_files.push(file.clone());
                }
            }
        }

        Some(output_files)
    }

    /// Run post-processing for state machine downloads (single videos)
    ///
    /// Simplified version that doesn't require InfoDict - only runs HLS fixup
    /// and user-requested post-processing.
    pub(super) async fn run_postprocessing_for_state_machine(
        &self,
        output_path: &Path,
        is_hls: bool,
    ) -> Result<PathBuf> {
        // For HLS downloads, run FFmpeg remux to fix container
        if is_hls {
            if let Some(fixed_files) = self.ffmpeg_remux_hls(&[output_path.to_path_buf()]).await {
                if let Some(fixed_path) = fixed_files.into_iter().next() {
                    return Ok(fixed_path);
                }
            }
        }

        // For non-HLS or if HLS fixup failed, check if user requested any post-processing
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
            eprintln!("🧹 Cleaned up {deleted} leftover segment files");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_core::{Config, Format};
    use std::path::Path;

    /// Helper function to create a test orchestrator
    pub(crate) fn create_test_orchestrator() -> Orchestrator {
        let config = Config::default();
        Orchestrator::new(config)
    }

    /// Helper function to create a test format
    fn create_test_format(format_id: &str, quality: &str, filesize: Option<u64>) -> Format {
        let mut format = Format::new(
            format_id.to_string(),
            "https://example.com/video.mp4".to_string(),
            "mp4".to_string(),
            "https".to_string(),
        );
        format.format_note = Some(quality.to_string());
        format.filesize = filesize;
        format.width = Some(1920);
        format.height = Some(1080);
        format.vcodec = Some("h264".to_string());
        format.acodec = Some("aac".to_string());
        format.tbr = Some(2000.0);
        format
    }

    #[test]
    fn test_sanitize_filename() {
        let orchestrator = create_test_orchestrator();

        assert_eq!(
            orchestrator.sanitize_filename("valid_filename"),
            "valid_filename"
        );
        assert_eq!(
            orchestrator.sanitize_filename("file/with/slashes"),
            "file_with_slashes"
        );
        assert_eq!(
            orchestrator.sanitize_filename("file\\with\\backslashes"),
            "file_with_backslashes"
        );
        assert_eq!(
            orchestrator.sanitize_filename("file:with:colons"),
            "file_with_colons"
        );
        assert_eq!(
            orchestrator.sanitize_filename("file*with?special<chars>"),
            "file_with_special_chars_"
        );
        assert_eq!(
            orchestrator.sanitize_filename("file|with\"pipes"),
            "file_with_pipes"
        );
    }

    #[test]
    fn test_sanitize_filename_null_bytes() {
        let orchestrator = create_test_orchestrator();
        // Null bytes should be removed
        assert_eq!(
            orchestrator.sanitize_filename("file\0name"),
            "filename"
        );
        assert_eq!(
            orchestrator.sanitize_filename("before\0\0after"),
            "beforeafter"
        );
    }

    #[test]
    fn test_sanitize_filename_control_characters() {
        let orchestrator = create_test_orchestrator();
        // Control characters should be removed (except space)
        assert_eq!(
            orchestrator.sanitize_filename("file\x01\x02name"),
            "filename"
        );
        // Tab and newline are control chars
        assert_eq!(
            orchestrator.sanitize_filename("file\tname"),
            "filename"
        );
        assert_eq!(
            orchestrator.sanitize_filename("file\nname"),
            "filename"
        );
        // Space is preserved
        assert_eq!(
            orchestrator.sanitize_filename("file name"),
            "file name"
        );
    }

    #[test]
    fn test_sanitize_filename_windows_reserved() {
        let orchestrator = create_test_orchestrator();
        // Windows reserved names get prefixed with underscore
        assert_eq!(orchestrator.sanitize_filename("CON"), "_CON");
        assert_eq!(orchestrator.sanitize_filename("con"), "_con");
        assert_eq!(orchestrator.sanitize_filename("PRN.txt"), "_PRN.txt");
        assert_eq!(orchestrator.sanitize_filename("AUX"), "_AUX");
        assert_eq!(orchestrator.sanitize_filename("NUL"), "_NUL");
        assert_eq!(orchestrator.sanitize_filename("COM1"), "_COM1");
        assert_eq!(orchestrator.sanitize_filename("LPT9"), "_LPT9");
        // Not reserved (has suffix that's not extension)
        assert_eq!(orchestrator.sanitize_filename("CONX"), "CONX");
        assert_eq!(orchestrator.sanitize_filename("CONSOLE"), "CONSOLE");
    }

    #[test]
    fn test_sanitize_filename_leading_trailing_dots_spaces() {
        let orchestrator = create_test_orchestrator();
        // Leading/trailing dots and spaces are trimmed
        assert_eq!(orchestrator.sanitize_filename("  filename  "), "filename");
        assert_eq!(orchestrator.sanitize_filename("..filename.."), "filename");
        assert_eq!(orchestrator.sanitize_filename(". .filename. ."), "filename");
        // Mixed
        assert_eq!(orchestrator.sanitize_filename(" . filename . "), "filename");
    }

    #[test]
    fn test_sanitize_filename_empty_string() {
        let orchestrator = create_test_orchestrator();
        assert_eq!(orchestrator.sanitize_filename(""), "unnamed");
        assert_eq!(orchestrator.sanitize_filename("   "), "unnamed");
        assert_eq!(orchestrator.sanitize_filename("..."), "unnamed");
        assert_eq!(orchestrator.sanitize_filename("\0\0"), "unnamed");
    }

    #[test]
    fn test_sanitize_filename_length_truncation() {
        let orchestrator = create_test_orchestrator();
        // Create a very long filename (300 chars)
        let long_name = "a".repeat(300);
        let result = orchestrator.sanitize_filename(&long_name);
        assert!(result.len() <= 200, "Filename should be truncated to 200 chars");

        // With extension - extension should be preserved
        let long_with_ext = format!("{}.mp4", "b".repeat(300));
        let result = orchestrator.sanitize_filename(&long_with_ext);
        assert!(result.len() <= 200);
        assert!(result.ends_with(".mp4"), "Extension should be preserved");
    }

    #[test]
    fn test_generate_output_path() {
        let orchestrator = create_test_orchestrator();
        let mut info = rdlp_core::InfoDict::new(
            "test123".to_string(),
            "Test Video".to_string(),
            "test".to_string(),
            "https://example.com/test".to_string(),
        );
        info.formats = vec![];
        let format = create_test_format("720p", "720p", Some(1000000));

        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "Test Video.mp4"
        );
    }

    #[test]
    fn test_generate_output_path_sanitizes_invalid_chars() {
        let orchestrator = create_test_orchestrator();
        let mut info = rdlp_core::InfoDict::new(
            "test123".to_string(),
            "Invalid/Characters\\In:Title*?.mp4".to_string(),
            "test".to_string(),
            "https://example.com/test".to_string(),
        );
        info.formats = vec![];
        let format = create_test_format("720p", "720p", Some(1000000));

        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "Invalid_Characters_In_Title__.mp4.mp4"
        );
    }

    #[test]
    fn test_generate_output_path_hls_extension() {
        let orchestrator = create_test_orchestrator();
        let mut info = rdlp_core::InfoDict::new(
            "test123".to_string(),
            "HLS Test Video".to_string(),
            "test".to_string(),
            "https://example.com/test".to_string(),
        );
        info.formats = vec![];

        // HLS with fMP4 segments (default)
        let mut format = create_test_format("720p", "720p", Some(1000000));
        format.ext = "hls".to_string();
        format.url = "https://example.com/playlist.m3u8".to_string();

        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "HLS Test Video.mp4"
        );

        // HLS with MPEG-TS segments (detected from URL)
        format.url = "https://example.com/segment0.ts".to_string();
        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "HLS Test Video.ts"
        );

        // HLS with explicit container field
        format.url = "https://example.com/playlist.m3u8".to_string();
        format.container = Some("mp4".to_string());
        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "HLS Test Video.mp4"
        );

        // HLS with container suffix (e.g., "mp4_dash")
        format.container = Some("mp4_dash".to_string());
        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "HLS Test Video.mp4"
        );
    }

    #[test]
    fn test_select_format_automatic_mode() {
        let orchestrator = create_test_orchestrator();
        let formats = vec![
            create_test_format("360p", "360p", Some(500000)),
            create_test_format("720p", "720p", Some(1000000)),
            create_test_format("1080p", "1080p", Some(2000000)),
        ];

        // Non-interactive mode should use format selector
        let result = orchestrator.select_format(&formats, false);
        assert!(result.is_ok());
        let selected = result.unwrap();
        assert!(selected.is_some());
        // Default selector should pick best quality
        let format = selected.unwrap();
        assert_eq!(format.format_id, "1080p");
    }

    #[test]
    fn test_select_format_empty_formats() {
        let orchestrator = create_test_orchestrator();
        let formats = vec![];

        // Should fail with empty formats in automatic mode
        let result = orchestrator.select_format(&formats, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_progress_bar_disabled() {
        let config = Config {
            progress: false,
            ..Default::default()
        };
        let orchestrator = Orchestrator::new(config);

        let result = orchestrator.create_progress_bar(Some(1000000), 0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_create_progress_bar_enabled() {
        let config = Config {
            progress: true,
            ..Default::default()
        };
        let orchestrator = Orchestrator::new(config);

        let result = orchestrator.create_progress_bar(Some(1000000), 0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_create_progress_bar_with_resume() {
        let config = Config {
            progress: true,
            ..Default::default()
        };
        let orchestrator = Orchestrator::new(config);

        let result = orchestrator.create_progress_bar(Some(1000000), 500000);
        assert!(result.is_ok());
        let pb = result.unwrap();
        assert!(pb.is_some());
        // Progress bar should be positioned at resume point
        assert_eq!(pb.unwrap().position(), 500000);
    }

    #[tokio::test]
    async fn test_detect_resume_point_no_file() {
        let orchestrator = create_test_orchestrator();
        let path = Path::new("nonexistent_file.mp4");

        let resume_from = orchestrator
            .detect_resume_point(path, None)
            .await
            .unwrap();
        assert_eq!(resume_from, 0);
    }

    #[test]
    fn test_list_extractors() {
        let orchestrator = create_test_orchestrator();
        let extractors = orchestrator.list_extractors();

        // Should have at least the TNAFlix network extractors
        assert!(!extractors.is_empty());
        assert!(
            extractors.contains(&"TNAFlix".to_string())
                || extractors.contains(&"EMPFlix".to_string())
                || extractors.contains(&"MovieFap".to_string()),
            "Expected to find at least one TNAFlix network extractor, found: {extractors:?}"
        );
    }

    // === State Machine Tests ===

    #[test]
    fn test_download_state_fresh() {
        let state = DownloadState::Fresh;
        assert_eq!(state.offset(), 0);
    }

    #[test]
    fn test_download_state_resume() {
        let state = DownloadState::Resume(1024);
        assert_eq!(state.offset(), 1024);
    }

    #[test]
    fn test_download_state_equality() {
        assert_eq!(DownloadState::Fresh, DownloadState::Fresh);
        assert_eq!(DownloadState::Resume(500), DownloadState::Resume(500));
        assert_ne!(DownloadState::Fresh, DownloadState::Resume(0));
        assert_ne!(DownloadState::Resume(100), DownloadState::Resume(200));
    }

    #[test]
    fn test_download_phase_debug() {
        let phase = DownloadPhase::Extracting {
            url: "https://example.com/video".to_string(),
        };
        let debug_str = format!("{phase:?}");
        assert!(debug_str.contains("Extracting"));
        assert!(debug_str.contains("https://example.com/video"));
    }

    #[test]
    fn test_download_phase_complete() {
        let phase = DownloadPhase::Complete {
            path: PathBuf::from("/path/to/video.mp4"),
        };
        match phase {
            DownloadPhase::Complete { path } => {
                assert_eq!(path, PathBuf::from("/path/to/video.mp4"));
            }
            _ => panic!("Expected Complete phase"),
        }
    }

    #[test]
    fn test_download_phase_cancelled() {
        let phase = DownloadPhase::Cancelled;
        match phase {
            DownloadPhase::Cancelled => {} // Expected
            _ => panic!("Expected Cancelled phase"),
        }
    }

    #[test]
    fn test_sanitize_filename_preserves_valid_chars() {
        let orchestrator = create_test_orchestrator();

        assert_eq!(
            orchestrator.sanitize_filename("My-Video_2024.mp4"),
            "My-Video_2024.mp4"
        );
        assert_eq!(
            orchestrator.sanitize_filename("test-file_name.mkv"),
            "test-file_name.mkv"
        );
    }

    #[test]
    fn test_sanitize_filename_handles_unicode() {
        let orchestrator = create_test_orchestrator();

        // Unicode should be preserved
        let result = orchestrator.sanitize_filename("日本語タイトル.mp4");
        assert!(result.contains("日本語タイトル"));
        assert!(result.ends_with(".mp4"));

        let result = orchestrator.sanitize_filename("Видео на русском.mp4");
        assert!(result.contains("Видео на русском"));
    }

    #[tokio::test]
    async fn test_merge_chunk_files_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create 3 chunk files with distinct content (old-style)
        let chunk0 = vec![1u8; 512];
        let chunk1 = vec![2u8; 512];
        let chunk2 = vec![3u8; 512];

        let chunk0_path = temp_dir.path().join("video.mp4.part0");
        let chunk1_path = temp_dir.path().join("video.mp4.part1");
        let chunk2_path = temp_dir.path().join("video.mp4.part2");

        tokio::fs::write(&chunk0_path, &chunk0).await.unwrap();
        tokio::fs::write(&chunk1_path, &chunk1).await.unwrap();
        tokio::fs::write(&chunk2_path, &chunk2).await.unwrap();

        // Create ChunkInfo for old-style chunks
        let chunk_info = resume::ChunkInfo {
            download_id: None,
            chunk_paths: vec![chunk0_path.clone(), chunk1_path.clone(), chunk2_path.clone()],
            total_size: 1536,
        };

        // Merge chunks
        let total_size = resume::merge_chunk_files(&output_path, &chunk_info).await.unwrap();

        // Verify total size
        assert_eq!(total_size, 1536);

        // Verify merged file exists
        assert!(output_path.exists());

        // Verify merged content
        let content = tokio::fs::read(&output_path).await.unwrap();
        assert_eq!(content.len(), 1536);
        assert_eq!(&content[0..512], chunk0.as_slice());
        assert_eq!(&content[512..1024], chunk1.as_slice());
        assert_eq!(&content[1024..1536], chunk2.as_slice());

        // Verify chunk files were deleted
        assert!(!chunk0_path.exists());
        assert!(!chunk1_path.exists());
        assert!(!chunk2_path.exists());
    }

    #[tokio::test]
    async fn test_merge_chunk_files_missing_chunk() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create only 2 of 3 chunks
        let chunk0_path = temp_dir.path().join("video.mp4.part0");
        let chunk1_path = temp_dir.path().join("video.mp4.part1");
        let chunk2_path = temp_dir.path().join("video.mp4.part2");

        tokio::fs::write(&chunk0_path, &[1u8; 512]).await.unwrap();
        tokio::fs::write(&chunk1_path, &[2u8; 512]).await.unwrap();
        // part2 is missing

        // Create ChunkInfo expecting 3 chunks but part2 doesn't exist
        let chunk_info = resume::ChunkInfo {
            download_id: None,
            chunk_paths: vec![chunk0_path, chunk1_path, chunk2_path.clone()],
            total_size: 1536,
        };

        // Merge should fail
        let result = resume::merge_chunk_files(&output_path, &chunk_info).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            OrchestratorError::MissingChunk { path } => {
                assert!(path.to_string_lossy().contains("video.mp4.part2"));
            }
            _ => panic!("Expected MissingChunk error, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_merge_chunk_files_empty_chunks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create empty chunk files
        let chunk0_path = temp_dir.path().join("video.mp4.part0");
        let chunk1_path = temp_dir.path().join("video.mp4.part1");

        tokio::fs::write(&chunk0_path, &[]).await.unwrap();
        tokio::fs::write(&chunk1_path, &[]).await.unwrap();

        // Create ChunkInfo
        let chunk_info = resume::ChunkInfo {
            download_id: None,
            chunk_paths: vec![chunk0_path.clone(), chunk1_path.clone()],
            total_size: 0,
        };

        let total_size = resume::merge_chunk_files(&output_path, &chunk_info).await.unwrap();

        assert_eq!(total_size, 0);
        assert!(output_path.exists());

        // Verify chunk files were deleted
        assert!(!chunk0_path.exists());
        assert!(!chunk1_path.exists());
    }

    /// Tests for Phase 3: Resume Compatibility
    mod resume_compatibility_tests {
        use super::*;

        #[tokio::test]
        async fn test_detect_old_style_chunks() {
            // Test detecting old-style chunks: video.mp4.part0, video.mp4.part1, ...
            let temp_dir = tempfile::tempdir().unwrap();
            let output_path = temp_dir.path().join("video.mp4");

            // Create old-style chunk files
            tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 512])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[2u8; 512])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &[3u8; 512])
                .await
                .unwrap();

            let orchestrator = create_test_orchestrator();
            let resume_offset = orchestrator
                .detect_resume_point(&output_path, None)
                .await
                .unwrap();

            // Should have merged the 3 chunks
            assert_eq!(resume_offset, 1536);
            assert!(output_path.exists());

            // Verify merged content
            let content = tokio::fs::read(&output_path).await.unwrap();
            assert_eq!(content.len(), 1536);

            // Verify chunk files were deleted
            assert!(!temp_dir.path().join("video.mp4.part0").exists());
            assert!(!temp_dir.path().join("video.mp4.part1").exists());
            assert!(!temp_dir.path().join("video.mp4.part2").exists());
        }

        #[tokio::test]
        async fn test_detect_new_style_chunks() {
            // Test detecting new-style chunks: video.mp4.0.part0, video.mp4.0.part1, ...
            let temp_dir = tempfile::tempdir().unwrap();
            let output_path = temp_dir.path().join("video.mp4");

            // Create new-style chunk files with download ID 0
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 256])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part1"), &[2u8; 256])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part2"), &[3u8; 256])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part3"), &[4u8; 256])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part4"), &[5u8; 256])
                .await
                .unwrap();

            let orchestrator = create_test_orchestrator();
            let resume_offset = orchestrator
                .detect_resume_point(&output_path, None)
                .await
                .unwrap();

            // Should have merged the 5 chunks
            assert_eq!(resume_offset, 1280);
            assert!(output_path.exists());

            // Verify merged content
            let content = tokio::fs::read(&output_path).await.unwrap();
            assert_eq!(content.len(), 1280);

            // Verify chunk files were deleted
            for i in 0..5 {
                assert!(!temp_dir
                    .path()
                    .join(format!("video.mp4.0.part{i}"))
                    .exists());
            }
        }

        #[tokio::test]
        async fn test_prioritize_new_style_over_old_style() {
            // Test that new-style chunks are prioritized over old-style when both exist
            let temp_dir = tempfile::tempdir().unwrap();
            let output_path = temp_dir.path().join("video.mp4");

            // Create old-style chunks (3 chunks, 1536 bytes total)
            tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 512])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[2u8; 512])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &[3u8; 512])
                .await
                .unwrap();

            // Create new-style chunks (5 chunks, 1280 bytes total, more recent)
            for i in 0..5 {
                tokio::fs::write(
                    temp_dir.path().join(format!("video.mp4.0.part{i}")),
                    &[((i + 10) as u8); 256],
                )
                .await
                .unwrap();
            }

            let orchestrator = create_test_orchestrator();
            let resume_offset = orchestrator
                .detect_resume_point(&output_path, None)
                .await
                .unwrap();

            // Should have merged the new-style chunks (1280 bytes), not old-style
            assert_eq!(resume_offset, 1280);
            assert!(output_path.exists());

            // Verify content came from new-style chunks
            let content = tokio::fs::read(&output_path).await.unwrap();
            assert_eq!(content.len(), 1280);
            assert_eq!(&content[0..256], &[10u8; 256]); // First new-style chunk

            // Verify new-style chunk files were deleted
            for i in 0..5 {
                assert!(!temp_dir
                    .path()
                    .join(format!("video.mp4.0.part{i}"))
                    .exists());
            }

            // Verify old-style chunk files were also cleaned up
            assert!(!temp_dir.path().join("video.mp4.part0").exists());
            assert!(!temp_dir.path().join("video.mp4.part1").exists());
            assert!(!temp_dir.path().join("video.mp4.part2").exists());
        }

        #[tokio::test]
        async fn test_prioritize_higher_download_id() {
            // Test that higher download IDs are prioritized
            let temp_dir = tempfile::tempdir().unwrap();
            let output_path = temp_dir.path().join("video.mp4");

            // Create chunks with download ID 0 (older)
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 256])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.0.part1"), &[2u8; 256])
                .await
                .unwrap();

            // Create chunks with download ID 2 (newer, should be preferred)
            tokio::fs::write(temp_dir.path().join("video.mp4.2.part0"), &[10u8; 512])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.2.part1"), &[20u8; 512])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.2.part2"), &[30u8; 512])
                .await
                .unwrap();

            let orchestrator = create_test_orchestrator();
            let resume_offset = orchestrator
                .detect_resume_point(&output_path, None)
                .await
                .unwrap();

            // Should have merged the download ID 2 chunks (3 × 512 = 1536 bytes)
            assert_eq!(resume_offset, 1536);

            // Verify content came from download ID 2
            let content = tokio::fs::read(&output_path).await.unwrap();
            assert_eq!(content.len(), 1536);
            assert_eq!(&content[0..512], &[10u8; 512]); // First chunk from ID 2

            // Verify download ID 2 chunks were deleted
            for i in 0..3 {
                assert!(!temp_dir
                    .path()
                    .join(format!("video.mp4.2.part{i}"))
                    .exists());
            }

            // Verify download ID 0 chunks still exist (not cleaned up since not used)
            // Actually, they should exist since we didn't use them
            assert!(temp_dir.path().join("video.mp4.0.part0").exists());
            assert!(temp_dir.path().join("video.mp4.0.part1").exists());
        }

        #[tokio::test]
        async fn test_cleanup_orphaned_chunks_when_file_complete() {
            // Test that orphaned chunks are cleaned up when main file is already complete
            let temp_dir = tempfile::tempdir().unwrap();
            let output_path = temp_dir.path().join("video.mp4");

            // Create complete file
            let complete_data = vec![42u8; 2048];
            tokio::fs::write(&output_path, &complete_data)
                .await
                .unwrap();

            // Create orphaned old-style chunks
            tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 256])
                .await
                .unwrap();
            tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[2u8; 256])
                .await
                .unwrap();

            let orchestrator = create_test_orchestrator();
            let resume_offset = orchestrator
                .detect_resume_point(&output_path, Some(2048))
                .await
                .unwrap();

            // Should detect file is complete
            assert_eq!(resume_offset, 2048);

            // Verify orphaned chunks were cleaned up
            assert!(!temp_dir.path().join("video.mp4.part0").exists());
            assert!(!temp_dir.path().join("video.mp4.part1").exists());
        }

        #[tokio::test]
        async fn test_many_new_style_chunks() {
            // Test handling many small chunks (simulating power-of-two chunking)
            let temp_dir = tempfile::tempdir().unwrap();
            let output_path = temp_dir.path().join("video.mp4");

            // Create 100 small chunks (simulating 1 MB chunks for a 100 MB file)
            for i in 0..100 {
                tokio::fs::write(
                    temp_dir.path().join(format!("video.mp4.0.part{i}")),
                    &[(i % 256) as u8; 128],
                )
                .await
                .unwrap();
            }

            let orchestrator = create_test_orchestrator();
            let resume_offset = orchestrator
                .detect_resume_point(&output_path, None)
                .await
                .unwrap();

            // Should have merged all 100 chunks (100 × 128 = 12800 bytes)
            assert_eq!(resume_offset, 12800);
            assert!(output_path.exists());

            // Verify all chunk files were deleted
            for i in 0..100 {
                assert!(!temp_dir
                    .path()
                    .join(format!("video.mp4.0.part{i}"))
                    .exists());
            }
        }
    }
}

#[cfg(test)]
mod property_tests {
    use super::tests::create_test_orchestrator;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_sanitize_filename_never_produces_invalid_chars(
            filename in "[a-zA-Z0-9 /\\\\:*?\"<>|.-]{0,100}"
        ) {
            let orchestrator = create_test_orchestrator();
            let sanitized = orchestrator.sanitize_filename(&filename);

            // No invalid filesystem characters should remain
            prop_assert!(!sanitized.contains('/'));
            prop_assert!(!sanitized.contains('\\'));
            prop_assert!(!sanitized.contains(':'));
            prop_assert!(!sanitized.contains('*'));
            prop_assert!(!sanitized.contains('?'));
            prop_assert!(!sanitized.contains('"'));
            prop_assert!(!sanitized.contains('<'));
            prop_assert!(!sanitized.contains('>'));
            prop_assert!(!sanitized.contains('|'));
            prop_assert!(!sanitized.contains('\0'));

            // No leading/trailing dots or spaces
            if !sanitized.is_empty() && sanitized != "unnamed" {
                let first = sanitized.chars().next().unwrap();
                let last = sanitized.chars().last().unwrap();
                prop_assert!(first != '.' && first != ' ', "Should not start with dot or space");
                prop_assert!(last != '.' && last != ' ', "Should not end with dot or space");
            }

            // Length should be within limit
            prop_assert!(sanitized.len() <= 200, "Filename should not exceed 200 chars");
        }

        #[test]
        fn test_sanitize_filename_never_empty(
            filename in ".{0,50}"  // Any string up to 50 chars
        ) {
            let orchestrator = create_test_orchestrator();
            let sanitized = orchestrator.sanitize_filename(&filename);

            // Result should never be empty
            prop_assert!(!sanitized.is_empty(), "Sanitized filename should never be empty");
        }

        #[test]
        fn test_sanitize_filename_preserves_alphanumeric_content(
            // Generate filenames with only alphanumeric chars and underscore (no edge cases)
            filename in "[a-zA-Z][a-zA-Z0-9_]{0,50}"
        ) {
            let orchestrator = create_test_orchestrator();
            let sanitized = orchestrator.sanitize_filename(&filename);

            // For simple alphanumeric filenames, content should be preserved
            prop_assert_eq!(sanitized, filename, "Simple alphanumeric filenames should be unchanged");
        }
    }
}
