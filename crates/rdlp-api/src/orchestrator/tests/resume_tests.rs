//! Tests for chunk merge and resume compatibility
#![allow(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::*;

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
    let msg = format!("{err}");
    assert!(
        msg.contains("video.mp4.part2") || msg.contains("missing chunk"),
        "Expected missing chunk error mentioning part2, got: {msg}"
    );
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
        assert_eq!(&content[0..256], &[10u8; 256]);

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

        // Should have merged the download ID 2 chunks (3 x 512 = 1536 bytes)
        assert_eq!(resume_offset, 1536);

        // Verify content came from download ID 2
        let content = tokio::fs::read(&output_path).await.unwrap();
        assert_eq!(content.len(), 1536);
        assert_eq!(&content[0..512], &[10u8; 512]);

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
        assert!(temp_dir.path().join("video.mp4.0.part0").exists());
        assert!(temp_dir.path().join("video.mp4.0.part1").exists());
    }

    #[tokio::test]
    async fn test_cleanup_orphaned_chunks_when_file_complete() {
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
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Create 100 small chunks
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

        // Should have merged all 100 chunks (100 x 128 = 12800 bytes)
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
