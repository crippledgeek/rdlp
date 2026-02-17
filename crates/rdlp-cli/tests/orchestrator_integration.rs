//! Integration tests for RdlpClient from the CLI perspective
//!
//! These tests verify the public API surface used by rdlp-cli.
//! Internal orchestrator tests (mock registries, state machine,
//! resume, templates) live in `rdlp-api/src/orchestrator/tests.rs`.

use rdlp_api::RdlpClient;
use rdlp_core::Config;
use tempfile::TempDir;

/// Helper function to create a test config with a temporary directory
fn create_test_config(temp_dir: &TempDir) -> Config {
    Config {
        output_directory: temp_dir.path().to_path_buf(),
        progress: false,
        ..Default::default()
    }
}

#[test]
fn test_client_creation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    let client = RdlpClient::builder()
        .config(config)
        .build()
        .expect("RdlpClient should build successfully");

    let extractors = client.list_extractors();
    assert!(
        !extractors.is_empty(),
        "Client should have at least one registered extractor"
    );
}

#[test]
fn test_list_extractors() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    let client = RdlpClient::builder()
        .config(config)
        .build()
        .expect("build should succeed");

    let extractors = client.list_extractors();

    assert!(
        extractors.len() >= 3,
        "Expected at least 3 extractors, got {}",
        extractors.len()
    );

    let extractor_names = extractors.join(", ");

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
fn test_list_downloaders() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    let client = RdlpClient::builder()
        .config(config)
        .build()
        .expect("build should succeed");

    let downloaders = client.list_downloaders();
    assert!(
        !downloaders.is_empty(),
        "Client should have at least one registered downloader"
    );
}

#[test]
fn test_client_with_custom_config() {
    let temp_dir = tempfile::tempdir().unwrap();

    let config = Config {
        output_directory: temp_dir.path().to_path_buf(),
        progress: false,
        format: "best".to_string(),
        concurrent_fragments: 8,
        ..Default::default()
    };

    assert_eq!(config.format, "best");
    assert_eq!(config.concurrent_fragments, 8);

    let client = RdlpClient::builder()
        .config(config)
        .build()
        .expect("build should succeed");

    assert!(!client.list_extractors().is_empty());
}

#[test]
fn test_multiple_clients() {
    let temp_dir1 = tempfile::tempdir().unwrap();
    let temp_dir2 = tempfile::tempdir().unwrap();

    let config1 = create_test_config(&temp_dir1);
    let config2 = create_test_config(&temp_dir2);

    let client1 = RdlpClient::builder()
        .config(config1)
        .build()
        .expect("build should succeed");

    let client2 = RdlpClient::builder()
        .config(config2)
        .build()
        .expect("build should succeed");

    assert!(!client1.list_extractors().is_empty());
    assert!(!client2.list_extractors().is_empty());

    assert_eq!(client1.list_extractors(), client2.list_extractors());
}

#[tokio::test]
async fn test_extract_info_unsupported_url() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = create_test_config(&temp_dir);

    let client = RdlpClient::builder()
        .config(config)
        .build()
        .expect("build should succeed");

    let result = client
        .extract_info("https://totally-unknown-site.example.com/video123")
        .await;
    assert!(result.is_err(), "Should fail for unsupported URL");
}
