use dialoguer::{theme::ColorfulTheme, Select};
use indicatif::{ProgressBar, ProgressStyle};
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::{
    Config, DownloadProgress, DownloadStats, ExtractionContext, Format, FormatSelector, ProgressCallback,
};
use rdlp_downloader::DownloaderRegistry;
use rdlp_extractor::ExtractorRegistry;
use rdlp_jsinterp::SimpleJsEngine;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// Errors that can occur during orchestration
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// No extractor found for the given URL
    #[error("No extractor found for URL: {url}")]
    NoExtractor {
        url: String,
    },

    /// Video extraction failed
    #[error("Failed to extract video information: {0}")]
    ExtractionFailed(#[source] anyhow::Error),

    /// User cancelled the operation
    #[error("Operation cancelled by user")]
    UserCancelled,

    /// No suitable format found
    #[error("No suitable format found matching criteria")]
    NoFormat,

    /// Format selector parsing failed
    #[error("Invalid format selector: {0}")]
    InvalidFormatSelector(#[source] anyhow::Error),

    /// No downloader found for the URL
    #[error("No downloader found for URL: {url}")]
    NoDownloader {
        url: String,
    },

    /// Download failed
    #[error("Download failed: {0}")]
    DownloadFailed(#[source] anyhow::Error),

    /// Resume detection failed
    #[error("Failed to detect resume point: {0}")]
    ResumeDetectionFailed(#[source] anyhow::Error),

    /// Missing chunk file during merge
    #[error("Missing chunk file: {path}")]
    MissingChunk {
        path: PathBuf,
    },

    /// Chunk merge failed
    #[error("Failed to merge chunk files: {0}")]
    ChunkMergeFailed(#[source] std::io::Error),

    /// Failed to generate output path
    #[error("Failed to generate output path: {0}")]
    PathGenerationFailed(String),

    /// Progress bar creation failed
    #[error("Failed to create progress bar: {0}")]
    ProgressBarFailed(#[source] anyhow::Error),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for orchestrator operations
pub type Result<T> = std::result::Result<T, OrchestratorError>;

/// Merge interrupted parallel download chunks into a single file
///
/// Returns the total size of the merged file
async fn merge_chunk_files(output_path: &std::path::Path, chunk_count: usize) -> Result<u64> {
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufWriter};

    let base_name = output_path.file_name().unwrap().to_string_lossy();
    let parent_dir = output_path.parent().unwrap_or_else(|| std::path::Path::new("."));

    // Create output file
    let file = File::create(output_path).await
        .map_err(OrchestratorError::ChunkMergeFailed)?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file); // 2 MB buffer

    let mut total_size = 0u64;

    // Merge each chunk in order
    for i in 0..chunk_count {
        let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));

        if !chunk_path.exists() {
            return Err(OrchestratorError::MissingChunk {
                path: chunk_path,
            });
        }

        let mut chunk_file = File::open(&chunk_path).await
            .map_err(OrchestratorError::ChunkMergeFailed)?;

        let bytes_copied = tokio::io::copy(&mut chunk_file, &mut writer).await
            .map_err(OrchestratorError::ChunkMergeFailed)?;

        total_size += bytes_copied;

        // Delete chunk file after successful merge
        tokio::fs::remove_file(&chunk_path).await
            .map_err(OrchestratorError::ChunkMergeFailed)?;
    }

    writer.flush().await
        .map_err(OrchestratorError::ChunkMergeFailed)?;

    Ok(total_size)
}

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
    Extracting {
        url: String,
    },
    /// Selecting format (interactive or automatic)
    SelectingFormat {
        info: Box<rdlp_core::InfoDict>,
    },
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
    Complete {
        path: PathBuf,
    },
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
    async fn advance(
        self,
        orchestrator: &Orchestrator,
        interactive: bool,
    ) -> Result<Self> {
        match self {
            Self::Extracting { url } => {
                let info = orchestrator.extract_video_info(&url).await?;
                Ok(Self::SelectingFormat { info: Box::new(info) })
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

                let resume_offset = orchestrator.detect_resume_point(&output_path, format.filesize).await?;

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

            Self::Downloading { output_path, format, state } => {
                let resume_from = state.offset();

                // Create progress bar
                let progress_bar = orchestrator.create_progress_bar(format.filesize, resume_from)?;

                // Find downloader
                let downloader = orchestrator.downloader_registry
                    .find_downloader(&format.url)
                    .ok_or_else(|| OrchestratorError::NoDownloader {
                        url: format.url.clone(),
                    })?;

                // Execute download
                let stats = match orchestrator.execute_download(
                    &downloader,
                    &format.url,
                    &output_path,
                    resume_from,
                    &progress_bar,
                ).await? {
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

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    extractor_registry: ExtractorRegistry,
    downloader_registry: DownloaderRegistry,
    extraction_context: Arc<ExtractionContext>,
    config: Arc<Config>,
}

impl Orchestrator {
    /// Create a new orchestrator with default registries
    pub fn new(config: Config) -> Self {
        let http_client = Arc::new(reqwest::Client::new());
        let js_engine = Arc::new(SimpleJsEngine::new());
        let cookie_jar = Arc::new(SimpleCookieJar::new());

        // Wrap config in Arc once and share it
        let config = Arc::new(config);

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::clone(&config),  // Cheap Arc clone instead of deep clone
        ));

        Self {
            extractor_registry: ExtractorRegistry::new(),
            downloader_registry: DownloaderRegistry::new(),
            extraction_context,
            config,
        }
    }

    /// Extract video information from URL using appropriate extractor
    ///
    /// # Errors
    /// Returns an error if:
    /// - No suitable extractor is found for the URL
    /// - Extraction fails (network error, parsing error, etc.)
    async fn extract_video_info(&self, url: &str) -> Result<rdlp_core::InfoDict> {
        println!("🔍 Finding extractor for URL...");

        let extractor = self
            .extractor_registry
            .find_extractor(url)
            .ok_or_else(|| OrchestratorError::NoExtractor {
                url: url.to_string(),
            })?;

        println!("✓ Using {} extractor", extractor.name());
        println!("📊 Extracting video information...");

        let info = extractor
            .extract(url, &self.extraction_context)
            .await
            .map_err(|e| OrchestratorError::ExtractionFailed(e.into()))?;

        println!("✓ Title: {}", info.title);
        println!("✓ Found {} formats", info.formats.len());

        Ok(info)
    }

    /// Select format from available formats
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (interactive mode only)
    ///
    /// # Errors
    /// Returns an error if:
    /// - Format selector string is invalid
    /// - No suitable format is found (automatic mode)
    fn select_format(&self, formats: &[Format], interactive: bool) -> Result<Option<Format>> {
        let format = if interactive {
            match self.select_format_interactive(formats)? {
                Some(format) => format,
                None => {
                    println!("\n❌ Selection cancelled by user");
                    return Ok(None);
                }
            }
        } else {
            let format_selector = FormatSelector::parse(&self.config.format)
                .map_err(|e| OrchestratorError::InvalidFormatSelector(e.into()))?;

            let selected_formats = format_selector.select(formats);
            if selected_formats.is_empty() {
                return Err(OrchestratorError::NoFormat);
            }

            selected_formats[0].clone()
        };

        println!("✓ Selected format: {} ({})", format.format_id, format.format_note.as_deref().unwrap_or("unknown"));
        Ok(Some(format))
    }

    /// Detect resume point for a download
    ///
    /// Checks for:
    /// 1. Existing partial download file
    /// 2. Interrupted parallel download chunks (.part0, .part1, etc.)
    ///
    /// If chunks are found, they are merged into the main file.
    ///
    /// Returns the number of bytes already downloaded (0 for fresh download)
    async fn detect_resume_point(&self, output_path: &std::path::Path, expected_size: Option<u64>) -> Result<u64> {
        if output_path.exists() {
            if let Ok(metadata) = tokio::fs::metadata(output_path).await {
                let size = metadata.len();
                if size > 0 {
                    // Check if file is already complete
                    if let Some(expected) = expected_size {
                        if size == expected {
                            println!("✓ File already downloaded ({:.1} MB), skipping...", size as f64 / (1024.0 * 1024.0));
                            return Ok(size);
                        } else if size > expected {
                            println!("⚠️  Partial file is larger than expected ({:.1} MB > {:.1} MB), starting fresh...",
                                size as f64 / (1024.0 * 1024.0), expected as f64 / (1024.0 * 1024.0));
                            tokio::fs::remove_file(output_path).await.ok();
                            return Ok(0);
                        }
                    }
                    println!("📋 Found partial download ({:.1} MB), resuming...", size as f64 / (1024.0 * 1024.0));
                    return Ok(size);
                }
            }
        }

        // Check for interrupted parallel download chunks (.part0, .part1, etc.)
        let base_name = output_path.file_name().unwrap().to_string_lossy();
        let parent_dir = output_path.parent().unwrap_or_else(|| std::path::Path::new("."));

        // Look for .part files
        let mut total_chunk_size = 0u64;
        let mut chunk_count = 0;

        for i in 0..10 {  // Check up to 10 chunks (concurrent_fragments is capped at 10)
            let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));
            if chunk_path.exists() {
                if let Ok(metadata) = tokio::fs::metadata(&chunk_path).await {
                    total_chunk_size += metadata.len();
                    chunk_count += 1;
                }
            }
        }

        if chunk_count > 0 {
            println!("📋 Found {} interrupted chunk files ({:.1} MB), merging and resuming...",
                chunk_count, total_chunk_size as f64 / (1024.0 * 1024.0));

            // Merge chunks into the main file
            match merge_chunk_files(output_path, chunk_count).await {
                Ok(size) => {
                    println!("✓ Merged {} chunks into main file ({:.1} MB)", chunk_count, size as f64 / (1024.0 * 1024.0));
                    Ok(size)
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to merge chunks: {e}. Starting fresh.");
                    // Clean up partial chunks
                    for i in 0..chunk_count {
                        let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));
                        let _ = tokio::fs::remove_file(&chunk_path).await;
                    }
                    Ok(0)
                }
            }
        } else {
            Ok(0)
        }
    }

    /// Create a progress bar for download tracking
    ///
    /// # Errors
    /// Returns an error if progress bar template is invalid
    fn create_progress_bar(&self, filesize: Option<u64>, resume_from: u64) -> Result<Option<ProgressBar>> {
        if !self.config.progress {
            return Ok(None);
        }

        let pb = ProgressBar::new(filesize.unwrap_or(0));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
                )
                .map_err(|e| OrchestratorError::ProgressBarFailed(e.into()))?
                .progress_chars("#>-"),
        );

        if resume_from > 0 {
            pb.set_position(resume_from);
        }

        Ok(Some(pb))
    }

    /// Execute download with Ctrl+C signal handling
    ///
    /// Returns Ok(Some(stats)) on success, Ok(None) if user cancelled
    ///
    /// # Errors
    /// Returns an error if download fails
    async fn execute_download(
        &self,
        downloader: &Arc<dyn rdlp_core::Downloader>,
        url: &str,
        output_path: &std::path::Path,
        resume_from: u64,
        progress_bar: &Option<ProgressBar>,
    ) -> Result<Option<DownloadStats>> {
        let progress_callback: Option<Box<dyn ProgressCallback>> = progress_bar.as_ref().map(|pb| {
            Box::new(ProgressBarCallback::new(pb.clone())) as Box<dyn ProgressCallback>
        });

        println!("⚠️  Press Ctrl+C to pause and save progress");

        let download_future = if resume_from > 0 {
            downloader.download_with_resume(url, output_path, resume_from, progress_callback)
        } else {
            downloader.download_to_file(url, output_path, progress_callback)
        };

        // Race between download and Ctrl+C signal
        let stats = tokio::select! {
            result = download_future => {
                result.map_err(|e| OrchestratorError::DownloadFailed(e.into()))?
            }
            _ = tokio::signal::ctrl_c() => {
                if let Some(pb) = progress_bar {
                    pb.finish_with_message("⏸️  Download paused");
                }
                println!("\n⏸️  Download interrupted by user");
                println!("💾 Progress saved. Run the same command again to resume.");
                return Ok(None);
            }
        };

        if let Some(pb) = progress_bar {
            pb.finish_with_message("✓ Download complete");
        }

        Ok(Some(stats))
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
    /// # Returns
    ///
    /// - `Ok(Some(path))` - Download completed successfully
    /// - `Ok(None)` - User cancelled operation
    /// - `Err` - Error occurred during any phase
    pub async fn download(&self, url: &str, interactive: bool) -> Result<Option<PathBuf>> {
        let mut phase = DownloadPhase::Extracting {
            url: url.to_string(),
        };

        loop {
            phase = phase.advance(self, interactive).await?;

            match phase {
                DownloadPhase::Complete { path } => return Ok(Some(path)),
                DownloadPhase::Cancelled => return Ok(None),
                _ => continue,  // Keep advancing through phases
            }
        }
    }

    /// Generate output file path
    fn generate_output_path(&self, info: &rdlp_core::InfoDict, format: &rdlp_core::Format) -> Result<PathBuf> {
        // Simple template parsing (full implementation in Phase 5)
        let filename = format!(
            "{}.{}",
            self.sanitize_filename(&info.title),
            format.ext
        );

        let mut path = self.config.output_directory.clone();
        path.push(filename);

        Ok(path)
    }

    /// Sanitize filename by removing invalid characters
    fn sanitize_filename(&self, name: &str) -> String {
        name.chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect()
    }

    /// List all available extractors
    pub fn list_extractors(&self) -> Vec<String> {
        self.extractor_registry.list_extractors()
    }

    /// Interactive format selection menu
    /// Returns Ok(Some(format)) if user selects, Ok(None) if cancelled
    fn select_format_interactive(&self, formats: &[Format]) -> Result<Option<Format>> {
        if formats.is_empty() {
            return Err(OrchestratorError::NoFormat);
        }

        // Build menu items with format details
        let items: Vec<String> = formats
            .iter()
            .map(|f| {
                let quality = f.format_note.as_deref().unwrap_or("unknown");
                let resolution = if let (Some(w), Some(h)) = (f.width, f.height) {
                    format!("{w}x{h}")
                } else {
                    "N/A".to_string()
                };

                let size = if let Some(filesize) = f.filesize {
                    format!("{:.1} MB", filesize as f64 / (1024.0 * 1024.0))
                } else {
                    "Unknown".to_string()
                };

                let codecs = match (&f.vcodec, &f.acodec) {
                    (Some(v), Some(a)) => format!("{v}/{a}"),
                    (Some(v), None) => format!("{v} (video only)"),
                    (None, Some(a)) => format!("{a} (audio only)"),
                    (None, None) => "Unknown".to_string(),
                };

                format!(
                    "{quality:<12} | {resolution:<10} | {size:<12} | {codecs}"
                )
            })
            .collect();

        println!("\n📋 Available formats:");
        println!("{:<12} | {:<10} | {:<12} | Codecs", "Quality", "Resolution", "Size");
        println!("{}", "-".repeat(70));

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a format to download (ESC to cancel)")
            .items(&items)
            .default(0)
            .interact_opt()
            .map_err(|e| OrchestratorError::Io(e.into()))?;

        match selection {
            Some(index) => Ok(Some(formats[index].clone())),
            None => Ok(None),
        }
    }
}

/// Progress callback that updates a progress bar
struct ProgressBarCallback {
    progress_bar: ProgressBar,
}

impl ProgressBarCallback {
    fn new(progress_bar: ProgressBar) -> Self {
        Self { progress_bar }
    }
}

impl ProgressCallback for ProgressBarCallback {
    fn on_progress(&self, progress: &DownloadProgress) {
        if let Some(total) = progress.total_bytes {
            self.progress_bar.set_length(total);
        }
        self.progress_bar.set_position(progress.bytes_downloaded);
    }

    fn on_complete(&self, _stats: &DownloadStats) {
        // Progress bar will be finished by caller
    }

    fn on_error(&self, error: &str) {
        self.progress_bar.abandon_with_message(format!("❌ Error: {error}"));
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
            "https".to_string()
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

        assert_eq!(orchestrator.sanitize_filename("valid_filename"), "valid_filename");
        assert_eq!(orchestrator.sanitize_filename("file/with/slashes"), "file_with_slashes");
        assert_eq!(orchestrator.sanitize_filename("file\\with\\backslashes"), "file_with_backslashes");
        assert_eq!(orchestrator.sanitize_filename("file:with:colons"), "file_with_colons");
        assert_eq!(orchestrator.sanitize_filename("file*with?special<chars>"), "file_with_special_chars_");
        assert_eq!(orchestrator.sanitize_filename("file|with\"pipes"), "file_with_pipes");
    }

    #[test]
    fn test_generate_output_path() {
        let orchestrator = create_test_orchestrator();
        let mut info = rdlp_core::InfoDict::new(
            "test123".to_string(),
            "Test Video".to_string(),
            "test".to_string(),
            "https://example.com/test".to_string()
        );
        info.formats = vec![];
        let format = create_test_format("720p", "720p", Some(1000000));

        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Test Video.mp4");
    }

    #[test]
    fn test_generate_output_path_sanitizes_invalid_chars() {
        let orchestrator = create_test_orchestrator();
        let mut info = rdlp_core::InfoDict::new(
            "test123".to_string(),
            "Invalid/Characters\\In:Title*?.mp4".to_string(),
            "test".to_string(),
            "https://example.com/test".to_string()
        );
        info.formats = vec![];
        let format = create_test_format("720p", "720p", Some(1000000));

        let path = orchestrator.generate_output_path(&info, &format).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Invalid_Characters_In_Title__.mp4.mp4");
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
        let mut config = Config::default();
        config.progress = false;
        let orchestrator = Orchestrator::new(config);

        let result = orchestrator.create_progress_bar(Some(1000000), 0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_create_progress_bar_enabled() {
        let mut config = Config::default();
        config.progress = true;
        let orchestrator = Orchestrator::new(config);

        let result = orchestrator.create_progress_bar(Some(1000000), 0);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_create_progress_bar_with_resume() {
        let mut config = Config::default();
        config.progress = true;
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

        let resume_from = orchestrator.detect_resume_point(path, None).await.unwrap();
        assert_eq!(resume_from, 0);
    }

    #[test]
    fn test_list_extractors() {
        let orchestrator = create_test_orchestrator();
        let extractors = orchestrator.list_extractors();

        // Should have at least the TNAFlix network extractors
        assert!(!extractors.is_empty());
        assert!(extractors.contains(&"TNAFlix".to_string()) ||
                extractors.contains(&"EMPFlix".to_string()) ||
                extractors.contains(&"MovieFap".to_string()),
                "Expected to find at least one TNAFlix network extractor, found: {:?}", extractors);
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
        let debug_str = format!("{:?}", phase);
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
            DownloadPhase::Cancelled => {}, // Expected
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

    #[test]
    fn test_sanitize_filename_empty_string() {
        let orchestrator = create_test_orchestrator();

        assert_eq!(orchestrator.sanitize_filename(""), "");
    }

    #[tokio::test]
    async fn test_merge_chunk_files_success() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create 3 chunk files with distinct content
        let chunk0 = vec![1u8; 512];
        let chunk1 = vec![2u8; 512];
        let chunk2 = vec![3u8; 512];

        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &chunk0).await.unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &chunk1).await.unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &chunk2).await.unwrap();

        // Merge chunks
        let total_size = merge_chunk_files(&output_path, 3).await.unwrap();

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
        assert!(!temp_dir.path().join("video.mp4.part0").exists());
        assert!(!temp_dir.path().join("video.mp4.part1").exists());
        assert!(!temp_dir.path().join("video.mp4.part2").exists());
    }

    #[tokio::test]
    async fn test_merge_chunk_files_missing_chunk() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create only 2 of 3 chunks
        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 512]).await.unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[2u8; 512]).await.unwrap();
        // part2 is missing

        // Merge should fail
        let result = merge_chunk_files(&output_path, 3).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            OrchestratorError::MissingChunk { path } => {
                assert!(path.to_string_lossy().contains("video.mp4.part2"));
            }
            _ => panic!("Expected MissingChunk error, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_merge_chunk_files_empty_chunks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create empty chunk files
        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[]).await.unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[]).await.unwrap();

        let total_size = merge_chunk_files(&output_path, 2).await.unwrap();

        assert_eq!(total_size, 0);
        assert!(output_path.exists());

        // Verify chunk files were deleted
        assert!(!temp_dir.path().join("video.mp4.part0").exists());
        assert!(!temp_dir.path().join("video.mp4.part1").exists());
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
        }

        #[test]
        fn test_sanitize_filename_preserves_length_roughly(
            filename in "[a-zA-Z0-9 ]{1,100}"
        ) {
            let orchestrator = create_test_orchestrator();
            let sanitized = orchestrator.sanitize_filename(&filename);

            // Length should be the same (no invalid chars to replace in this input)
            prop_assert_eq!(sanitized.len(), filename.len());
        }
    }
}
