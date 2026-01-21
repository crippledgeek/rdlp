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
use rdlp_core::{Config, ExtractionContext};
use rdlp_downloader::{DownloaderRegistry, DownloaderRegistryTrait};
use rdlp_extractor::{ExtractorRegistry, ExtractorRegistryTrait};
use rdlp_jsinterp::SimpleJsEngine;
use std::path::PathBuf;
use std::sync::Arc;

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    pub(super) extractor_registry: Arc<dyn ExtractorRegistryTrait>,
    pub(super) downloader_registry: Arc<dyn DownloaderRegistryTrait>,
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

        Self {
            extractor_registry: Arc::new(ExtractorRegistry::new()),
            downloader_registry: Arc::new(DownloaderRegistry::new()),
            extraction_context,
            config,
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

        Self {
            extractor_registry,
            downloader_registry,
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
                _ => continue, // Keep advancing through phases
            }
        }
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
                // Try to detect from URL (e.g., .../segment.ts or .../segment.m4s)
                if format.url.contains(".ts") {
                    "ts".to_string()  // MPEG-TS segments
                } else if format.url.contains(".m4s") || format.url.contains(".mp4") {
                    "mp4".to_string()  // fMP4 segments
                } else {
                    "mp4".to_string()  // Default to MP4 for HLS
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

    /// Sanitize filename by removing invalid characters
    pub(super) fn sanitize_filename(&self, name: &str) -> String {
        name.chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect()
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
            "Expected to find at least one TNAFlix network extractor, found: {:?}",
            extractors
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

    #[test]
    fn test_sanitize_filename_empty_string() {
        let orchestrator = create_test_orchestrator();

        assert_eq!(orchestrator.sanitize_filename(""), "");
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
            _ => panic!("Expected MissingChunk error, got {:?}", err),
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
                    .join(format!("video.mp4.0.part{}", i))
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
                    temp_dir.path().join(format!("video.mp4.0.part{}", i)),
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
                    .join(format!("video.mp4.0.part{}", i))
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
                    .join(format!("video.mp4.2.part{}", i))
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
                    temp_dir.path().join(format!("video.mp4.0.part{}", i)),
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
                    .join(format!("video.mp4.0.part{}", i))
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
