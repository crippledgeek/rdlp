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
        let http_client = Arc::new(reqwest::Client::new());
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
        let http_client = Arc::new(reqwest::Client::new());
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

    /// Generate output file path
    pub(super) fn generate_output_path(
        &self,
        info: &rdlp_core::InfoDict,
        format: &rdlp_core::Format,
    ) -> Result<PathBuf> {
        // Simple template parsing (full implementation in Phase 5)
        let filename = format!("{}.{}", self.sanitize_filename(&info.title), format.ext);

        let mut path = self.config.output_directory.clone();
        path.push(filename);

        Ok(path)
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

        // Create 3 chunk files with distinct content
        let chunk0 = vec![1u8; 512];
        let chunk1 = vec![2u8; 512];
        let chunk2 = vec![3u8; 512];

        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &chunk0)
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &chunk1)
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &chunk2)
            .await
            .unwrap();

        // Merge chunks
        let total_size = resume::merge_chunk_files(&output_path, 3).await.unwrap();

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
        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 512])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[2u8; 512])
            .await
            .unwrap();
        // part2 is missing

        // Merge should fail
        let result = resume::merge_chunk_files(&output_path, 3).await;
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
        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[])
            .await
            .unwrap();

        let total_size = resume::merge_chunk_files(&output_path, 2).await.unwrap();

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
