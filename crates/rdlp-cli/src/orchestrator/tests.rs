//! Tests for the orchestrator module

use super::*;
use rdlp_core::{Config, Format, InfoDict};
use std::path::Path;

/// Helper function to create a test orchestrator
pub(crate) fn create_test_orchestrator() -> Orchestrator {
    let config = Config::default();
    Orchestrator::new(config, MultiProgress::new())
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
        rdlp_core::DownloadProtocol::Https,
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
        rdlp_core::InfoDict::new("test123", "Test Video", "test", "https://example.com/test");
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
        "Invalid_Characters_In_Title__.mp4.mp4"
    );
}

#[test]
fn test_generate_output_path_hls_extension() {
    let orchestrator = create_test_orchestrator();
    let mut info = rdlp_core::InfoDict::new(
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
        "HLS Test Video.mp4"
    );

    // HLS with MPEG-TS segments (detected from segment metadata)
    format.container = Some("ts".to_string());
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
    let format = selected.unwrap();
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

#[test]
fn test_create_progress_bar_disabled() {
    let config = Config {
        progress: false,
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());

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
    let orchestrator = Orchestrator::new(config, MultiProgress::new());

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
    let orchestrator = Orchestrator::new(config, MultiProgress::new());

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
        chunk_paths: vec![
            chunk0_path.clone(),
            chunk1_path.clone(),
            chunk2_path.clone(),
        ],
        total_size: 1536,
    };

    // Merge chunks
    let total_size = resume::merge_chunk_files(&output_path, &chunk_info)
        .await
        .unwrap();

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

    let total_size = resume::merge_chunk_files(&output_path, &chunk_info)
        .await
        .unwrap();

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
            assert!(
                !temp_dir
                    .path()
                    .join(format!("video.mp4.0.part{i}"))
                    .exists()
            );
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
            assert!(
                !temp_dir
                    .path()
                    .join(format!("video.mp4.0.part{i}"))
                    .exists()
            );
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
            assert!(
                !temp_dir
                    .path()
                    .join(format!("video.mp4.2.part{i}"))
                    .exists()
            );
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
            assert!(
                !temp_dir
                    .path()
                    .join(format!("video.mp4.0.part{i}"))
                    .exists()
            );
        }
    }
}

// === Output Template Tests ===

#[test]
fn test_generate_output_path_custom_template() {
    let config = Config {
        output_template: "%(id)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "test_id.mp4");
}

#[test]
fn test_generate_output_path_subdirectory_template() {
    let config = Config {
        output_template: "%(extractor)s/%(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    // Should create a subdirectory structure
    let components: Vec<_> = path.components().collect();
    let len = components.len();
    assert!(len >= 2);
    // Last component is the filename
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video.mp4"
    );
    // Second-to-last is the extractor directory
    let parent = path.parent().unwrap();
    assert_eq!(
        parent.file_name().unwrap().to_str().unwrap(),
        "TestExtractor"
    );
}

#[test]
fn test_generate_output_path_field_with_default() {
    let config = Config {
        output_template: "%(uploader|Unknown)s - %(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    // uploader is None in test_info_dict, so default "Unknown" should be used
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Unknown - Test Video.mp4"
    );
}

#[test]
fn test_generate_output_path_sanitizes_template_field_values() {
    let config = Config {
        output_template: "%(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let mut info = InfoDict::new(
        "test123",
        "Bad/Title\\With:Chars",
        "test",
        "https://example.com/test",
    );
    info.formats = vec![];
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    let filename = path.file_name().unwrap().to_str().unwrap();
    // Slashes and colons should be replaced
    assert!(!filename.contains('/'));
    assert!(!filename.contains('\\'));
    assert!(!filename.contains(':'));
}

#[test]
fn test_generate_output_path_with_format_fields() {
    let config = Config {
        output_template: "%(title)s_%(height)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video_1080.mp4"
    );
}

#[test]
fn test_generate_output_path_numeric_padding() {
    let config = Config {
        output_template: "%(playlist_index)03d - %(title)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let mut info = create_test_info_dict(vec![]);
    info.playlist_index = Some(7);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "007 - Test Video.mp4"
    );
}

#[test]
fn test_generate_output_path_missing_field_defaults_to_na() {
    let config = Config {
        output_template: "%(nonexistent_field)s.%(ext)s".to_string(),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    // yt-dlp behavior: missing fields produce "NA" instead of an error
    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "NA.mp4");
}

#[test]
fn test_generate_output_path_with_output_directory() {
    let config = Config {
        output_template: "%(title)s.%(ext)s".to_string(),
        output_directory: PathBuf::from("/tmp/downloads"),
        ..Default::default()
    };
    let orchestrator = Orchestrator::new(config, MultiProgress::new());
    let info = create_test_info_dict(vec![]);
    let format = create_test_format("720p", "720p", Some(1000000));

    let path = orchestrator.generate_output_path(&info, &format).unwrap();
    assert!(path.starts_with("/tmp/downloads") || path.starts_with("\\tmp\\downloads"));
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "Test Video.mp4"
    );
}

mod property_tests {
    use super::create_test_orchestrator;
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
