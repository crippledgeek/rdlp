//! Integration tests for the Orchestrator
//!
//! These tests verify the full orchestrator workflow with real components.
//! Note: Full end-to-end tests with mocked HTTP servers will be added in Phase 7
//! when registries become mockable.

use rdlp_cli::Orchestrator;
use rdlp_core::Config;
use tempfile::TempDir;

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
    println!("Registered extractors: {}", extractor_names);

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
        "TNAFlix extractor should be registered. Found: {}",
        extractor_names
    );
    assert!(
        has_empflix,
        "EMPFlix extractor should be registered. Found: {}",
        extractor_names
    );
    assert!(
        has_moviefap,
        "MovieFap extractor should be registered. Found: {}",
        extractor_names
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

// NOTE: The following tests are placeholders for Phase 7 (Advanced Refactoring)
// when registries become mockable for full integration testing.

#[cfg(feature = "integration-tests-phase7")]
mod future_tests {
    use super::*;

    #[tokio::test]
    async fn test_full_download_workflow_with_mock_server() {
        // TODO: Implement once registries are mockable
        // This will:
        // 1. Set up a wiremock server
        // 2. Mock extractor responses
        // 3. Mock download responses
        // 4. Execute full download workflow
        // 5. Verify output file is created correctly
        unimplemented!("Phase 7: Requires mockable registries")
    }

    #[tokio::test]
    async fn test_download_with_resume() {
        // TODO: Test download resume functionality with mock server
        unimplemented!("Phase 7: Requires mockable registries")
    }

    #[tokio::test]
    async fn test_download_with_interruption() {
        // TODO: Test Ctrl+C interruption handling
        unimplemented!("Phase 7: Requires mockable registries")
    }

    #[tokio::test]
    async fn test_parallel_download_workflow() {
        // TODO: Test parallel chunk download workflow
        unimplemented!("Phase 7: Requires mockable registries")
    }

    #[tokio::test]
    async fn test_interactive_format_selection() {
        // TODO: Test interactive format selection
        unimplemented!("Phase 7: Requires mockable registries")
    }
}
