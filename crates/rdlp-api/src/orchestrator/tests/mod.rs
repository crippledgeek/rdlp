//! Tests for the orchestrator module

mod property_tests;
mod resume_tests;
mod template_tests;

use super::state::DownloadState;
use super::*;
use crate::events::Event;
use crate::handle::DownloadId;
use rdlp_types::Codec;
use rdlp_types::{Config, Format, InfoDict};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Helper function to create a test orchestrator with event channel
pub(super) fn create_test_orchestrator() -> Orchestrator {
    let config = Arc::new(Config::default());
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let id = DownloadId::next();
    let token = CancellationToken::new();
    Orchestrator::new(config, tx, id, token, None)
}

/// Helper function to wrap formats in an InfoDict for testing
fn create_test_info_dict(formats: Vec<Format>) -> InfoDict {
    let mut info = InfoDict::new(
        "test_id",
        "Test Video",
        "TestExtractor",
        "https://example.com/video",
    );
    info.formats = formats;
    info
}

/// Helper function to create a test format
fn create_test_format(format_id: &str, quality: &str, filesize: Option<u64>) -> Format {
    let mut format = Format::new(
        format_id,
        "https://example.com/video.mp4",
        "mp4",
        rdlp_types::DownloadProtocol::Https,
    );
    format.format_note = Some(quality.to_string());
    format.filesize = filesize;
    format.width = Some(1920);
    format.height = Some(1080);
    format.vcodec = Codec::from("h264".to_string());
    format.acodec = Codec::from("aac".to_string());
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
    assert_eq!(orchestrator.sanitize_filename("file\0name"), "filename");
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
    assert_eq!(orchestrator.sanitize_filename("file\tname"), "filename");
    assert_eq!(orchestrator.sanitize_filename("file\nname"), "filename");
    // Space is preserved
    assert_eq!(orchestrator.sanitize_filename("file name"), "file name");
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
    assert!(
        result.len() <= 200,
        "Filename should be truncated to 200 chars"
    );

    // With extension - extension should be preserved
    let long_with_ext = format!("{}.mp4", "b".repeat(300));
    let result = orchestrator.sanitize_filename(&long_with_ext);
    assert!(result.len() <= 200);
    assert!(result.ends_with(".mp4"), "Extension should be preserved");
}

#[test]
fn test_generate_output_path() {
    let orchestrator = create_test_orchestrator();
    let mut info =
        rdlp_types::InfoDict::new("test123", "Test Video", "test", "https://example.com/test");
    info.formats = vec![];
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    // Default template is `%(title|Unknown)s [%(id)s].%(ext)s` —
    // see `Config::default` in `rdlp-types/src/config.rs`. The
    // `[id]` suffix is always present so empty-title extracts
    // (e.g. godresource) still land at distinguishable filenames.
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video [test123].mp4"
    );
}

#[test]
fn test_generate_output_path_sanitizes_invalid_chars() {
    let orchestrator = create_test_orchestrator();
    let mut info = rdlp_types::InfoDict::new(
        "test123",
        "Invalid/Characters\\In:Title*?.mp4",
        "test",
        "https://example.com/test",
    );
    info.formats = vec![];
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Invalid_Characters_In_Title__.mp4 [test123].mp4"
    );
}

#[test]
fn test_generate_output_path_hls_extension() {
    let orchestrator = create_test_orchestrator();
    let mut info = rdlp_types::InfoDict::new(
        "test123",
        "HLS Test Video",
        "test",
        "https://example.com/test",
    );
    info.formats = vec![];

    // HLS with fMP4 segments (default)
    let mut format = create_test_format("720p", "720p", Some(1000000));
    format.ext = "hls".to_string();
    format.url = "https://example.com/playlist.m3u8".to_string();

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "HLS Test Video [test123].mp4"
    );

    // HLS with MPEG-TS segments (detected from segment metadata)
    format.container = Some("ts".to_string());
    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "HLS Test Video [test123].ts"
    );

    // HLS with explicit container field
    format.url = "https://example.com/playlist.m3u8".to_string();
    format.container = Some("mp4".to_string());
    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "HLS Test Video [test123].mp4"
    );

    // HLS with container suffix (e.g., "mp4_dash")
    format.container = Some("mp4_dash".to_string());
    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "HLS Test Video [test123].mp4"
    );
}

#[tokio::test]
async fn test_select_format_automatic_mode() {
    let orchestrator = create_test_orchestrator();
    let formats = vec![
        create_test_format("360p", "360p", Some(500000)),
        create_test_format("720p", "720p", Some(1000000)),
        create_test_format("1080p", "1080p", Some(2000000)),
    ];
    let info = create_test_info_dict(formats);

    // Non-interactive mode should use format selector
    let result = orchestrator.select_format(&info, false).await;
    assert!(result.is_ok());
    let selected = result.unwrap();
    assert!(selected.is_some());
    // Default selector should pick best quality
    let plan = selected.unwrap();
    let format = match plan {
        DownloadPlan::Single(f) => f,
        other => panic!("Expected Single, got {other}"),
    };
    assert_eq!(format.format_id, "1080p");
}

#[tokio::test]
async fn test_select_format_empty_formats() {
    let orchestrator = create_test_orchestrator();
    let info = create_test_info_dict(vec![]);

    // Should fail with empty formats in automatic mode
    let result = orchestrator.select_format(&info, false).await;
    assert!(result.is_err());
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
    assert!(
        extractors.contains(&"TNAFlix")
            || extractors.contains(&"EMPFlix")
            || extractors.contains(&"MovieFap"),
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
