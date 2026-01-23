//! Integration tests for the Orchestrator
//!
//! These tests verify the full orchestrator workflow with mocked components.

mod test_utils;

use rdlp_cli::Orchestrator;
use rdlp_core::Config;
use tempfile::TempDir;
use test_utils::{
    create_test_info_dict, MockDownloader, MockDownloaderRegistry, MockExtractor,
    MockExtractorRegistry,
};
use std::sync::Arc;

/// Helper function to create a test config with a temporary directory
fn create_test_config(temp_dir: &TempDir) -> Config {
    Config {
        output_directory: temp_dir.path().to_path_buf(),
        progress: false, // Disable progress bars in tests
        ..Default::default()
    }
}

#[test]
fn test_orchestrator_creation() {
    // Create a temporary directory for test outputs
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    // Create orchestrator
    let orch = Orchestrator::new(config);

    // Verify orchestrator was created successfully
    // The orchestrator should have registered extractors
    let extractors = orch.list_extractors();
    assert!(
        !extractors.is_empty(),
        "Orchestrator should have at least one registered extractor"
    );
}

#[test]
fn test_list_extractors() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);
    let orch = Orchestrator::new(config);

    let extractors = orch.list_extractors();

    // Verify we have the expected extractors registered
    // Based on the project, we should have at least TNAFlix, EMPFlix, and MovieFap
    assert!(
        extractors.len() >= 3,
        "Expected at least 3 extractors (TNAFlix, EMPFlix, MovieFap), got {}",
        extractors.len()
    );

    // Verify expected extractors are present
    let extractor_names = extractors.join(", ");
    println!("Registered extractors: {extractor_names}");

    // Check for known extractors (case-insensitive)
    let has_tnaflix = extractors
        .iter()
        .any(|e| e.to_lowercase().contains("tnaflix"));
    let has_empflix = extractors
        .iter()
        .any(|e| e.to_lowercase().contains("empflix"));
    let has_moviefap = extractors
        .iter()
        .any(|e| e.to_lowercase().contains("moviefap"));

    assert!(
        has_tnaflix,
        "TNAFlix extractor should be registered. Found: {extractor_names}"
    );
    assert!(
        has_empflix,
        "EMPFlix extractor should be registered. Found: {extractor_names}"
    );
    assert!(
        has_moviefap,
        "MovieFap extractor should be registered. Found: {extractor_names}"
    );
}

#[test]
fn test_orchestrator_with_custom_config() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Create config with custom settings
    let config = Config {
        output_directory: temp_dir.path().to_path_buf(),
        progress: false,
        format: "best".to_string(),
        concurrent_fragments: 8,
        ..Default::default()
    };

    // Verify config is applied correctly
    assert_eq!(config.format, "best");
    assert_eq!(config.concurrent_fragments, 8);
    assert!(!config.progress);

    // Create orchestrator with custom config
    let orch = Orchestrator::new(config);

    // Verify orchestrator works with custom config
    assert!(!orch.list_extractors().is_empty());
}

#[test]
fn test_multiple_orchestrators() {
    // Verify multiple orchestrators can coexist
    let temp_dir1 = tempfile::tempdir().unwrap();
    let temp_dir2 = tempfile::tempdir().unwrap();

    let config1 = create_test_config(&temp_dir1);
    let config2 = create_test_config(&temp_dir2);

    let orch1 = Orchestrator::new(config1);
    let orch2 = Orchestrator::new(config2);

    // Both should work independently
    assert!(!orch1.list_extractors().is_empty());
    assert!(!orch2.list_extractors().is_empty());

    // Both should have the same extractors available
    assert_eq!(orch1.list_extractors(), orch2.list_extractors());
}

// Phase 7: Advanced Integration Tests with Mock Registries

#[tokio::test]
async fn test_full_download_workflow_with_mocks() {
    // Create temporary directory
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    // Create mock extractor
    let info_dict = create_test_info_dict("Test Video", "mock://example.com/video123");
    let extractor = Arc::new(MockExtractor::new(
        "MockExtractor",
        "example.com",
        info_dict,
    ));

    let mut extractor_registry = MockExtractorRegistry::new();
    extractor_registry.register(extractor);

    // Create mock downloader
    let downloader = Arc::new(MockDownloader::new(
        "mock",
        true,  // should_succeed
        1024 * 1024, // 1 MB file
        100,   // 100ms delay
    ));

    let mut downloader_registry = MockDownloaderRegistry::new();
    downloader_registry.register(downloader);

    // Create orchestrator with mock registries
    let orch = Orchestrator::with_registries(
        config,
        Arc::new(extractor_registry),
        Arc::new(downloader_registry),
    );

    // Execute download workflow
    let result = orch.download("mock://example.com/video123", false).await;

    // Verify download succeeded
    assert!(result.is_ok(), "Download should succeed");
    let output_path = result.unwrap();
    assert!(output_path.is_some(), "Output path should be returned");

    let path = output_path.unwrap();
    assert!(path.exists(), "Downloaded file should exist");

    // Verify file size
    let metadata = tokio::fs::metadata(&path).await.unwrap();
    assert_eq!(metadata.len(), 1024 * 1024, "File size should be 1 MB");
}

#[tokio::test]
async fn test_download_with_resume() {
    // Create temporary directory
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    // Create mock extractor
    let info_dict = create_test_info_dict("Resume Test Video", "mock://example.com/resume123");
    let extractor = Arc::new(MockExtractor::new(
        "MockExtractor",
        "example.com",
        info_dict,
    ));

    let mut extractor_registry = MockExtractorRegistry::new();
    extractor_registry.register(extractor);

    // Create mock downloader
    let downloader = Arc::new(MockDownloader::new(
        "mock",
        true,
        2 * 1024 * 1024, // 2 MB file
        50,
    ));

    let mut downloader_registry = MockDownloaderRegistry::new();
    downloader_registry.register(downloader);

    // Create orchestrator
    let orch = Orchestrator::with_registries(
        config,
        Arc::new(extractor_registry),
        Arc::new(downloader_registry),
    );

    // First download: Create partial file
    let output_path = temp_dir.path().join("Resume Test Video.mp4");
    let partial_content = vec![0u8; 1024 * 1024]; // 1 MB partial download
    tokio::fs::write(&output_path, &partial_content)
        .await
        .unwrap();

    // Second download: Resume from partial
    let result = orch.download("mock://example.com/resume123", false).await;

    // Verify download succeeded
    assert!(result.is_ok(), "Resume download should succeed");
    let path = result.unwrap();
    assert!(path.is_some(), "Output path should be returned");

    // Verify file was resumed and completed
    let metadata = tokio::fs::metadata(&path.unwrap()).await.unwrap();
    assert_eq!(
        metadata.len(),
        2 * 1024 * 1024,
        "File should be completed to 2 MB"
    );
}

#[tokio::test]
async fn test_download_failure_handling() {
    // Create temporary directory
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    // Create mock extractor
    let info_dict = create_test_info_dict("Failure Test", "mock://example.com/fail123");
    let extractor = Arc::new(MockExtractor::new(
        "MockExtractor",
        "example.com",
        info_dict,
    ));

    let mut extractor_registry = MockExtractorRegistry::new();
    extractor_registry.register(extractor);

    // Create failing mock downloader
    let downloader = Arc::new(MockDownloader::new(
        "mock",
        false, // should_succeed = false
        0,
        0,
    ));

    let mut downloader_registry = MockDownloaderRegistry::new();
    downloader_registry.register(downloader);

    // Create orchestrator
    let orch = Orchestrator::with_registries(
        config,
        Arc::new(extractor_registry),
        Arc::new(downloader_registry),
    );

    // Execute download workflow
    let result = orch.download("mock://example.com/fail123", false).await;

    // Verify download failed with appropriate error
    assert!(result.is_err(), "Download should fail");
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Download failed")
            || error.to_string().contains("Mock download failed"),
        "Error message should indicate download failure"
    );
}

#[tokio::test]
async fn test_no_extractor_found() {
    // Create temporary directory
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    // Create empty extractor registry (no extractors)
    let extractor_registry = MockExtractorRegistry::new();

    // Create mock downloader
    let downloader = Arc::new(MockDownloader::new("mock", true, 1024, 50));
    let mut downloader_registry = MockDownloaderRegistry::new();
    downloader_registry.register(downloader);

    // Create orchestrator
    let orch = Orchestrator::with_registries(
        config,
        Arc::new(extractor_registry),
        Arc::new(downloader_registry),
    );

    // Execute download workflow
    let result = orch.download("mock://unknown.com/video123", false).await;

    // Verify error is returned
    assert!(result.is_err(), "Should fail when no extractor found");
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("No extractor"),
        "Error should indicate missing extractor"
    );
}

#[tokio::test]
async fn test_multiple_format_selection() {
    // Create temporary directory
    let temp_dir = tempfile::tempdir().unwrap();
    let mut config = create_test_config(&temp_dir);
    config.format = "1".to_string(); // Select format with ID "1"

    // Create mock extractor with multiple formats
    let info_dict = create_test_info_dict("Multi Format Test", "mock://example.com/multi123");
    // The helper already creates multiple formats (format IDs "1" and "2")

    let extractor = Arc::new(MockExtractor::new(
        "MockExtractor",
        "example.com",
        info_dict,
    ));

    let mut extractor_registry = MockExtractorRegistry::new();
    extractor_registry.register(extractor);

    // Create mock downloader
    let downloader = Arc::new(MockDownloader::new("mock", true, 512 * 1024, 50));
    let mut downloader_registry = MockDownloaderRegistry::new();
    downloader_registry.register(downloader);

    // Create orchestrator
    let orch = Orchestrator::with_registries(
        config,
        Arc::new(extractor_registry),
        Arc::new(downloader_registry),
    );

    // Execute download workflow
    let result = orch.download("mock://example.com/multi123", false).await;

    // Verify download succeeded
    assert!(result.is_ok(), "Download with format selection should succeed: {:?}", result.as_ref().err());
    assert!(result.unwrap().is_some(), "Should return output path");
}
