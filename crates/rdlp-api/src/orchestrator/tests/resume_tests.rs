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

    /// #559 acceptance: the legacy `0..10` scan bound never leaked in
    /// practice (legacy `concurrent_fragments` was capped at 10), but a
    /// hardcoded ceiling on cleanup is still a defect waiting to happen if
    /// that assumption ever drifts. 12 legacy chunks (2 beyond the old
    /// bound) must ALL be cleaned up when the file is already complete.
    #[tokio::test]
    async fn test_cleanup_legacy_chunks_beyond_old_ten_chunk_bound() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let complete_data = vec![7u8; 4096];
        tokio::fs::write(&output_path, &complete_data)
            .await
            .unwrap();

        for i in 0..12 {
            tokio::fs::write(
                temp_dir.path().join(format!("video.mp4.part{i}")),
                &[i as u8; 64],
            )
            .await
            .unwrap();
        }

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, Some(4096))
            .await
            .unwrap();

        assert_eq!(resume_offset, 4096);
        for i in 0..12 {
            assert!(
                !temp_dir.path().join(format!("video.mp4.part{i}")).exists(),
                "chunk {i} should have been cleaned up (beyond the old 0..10 bound)"
            );
        }
    }

    /// #571 MEDIUM follow-up: `cleanup_old_chunks` gates its
    /// [`resume::CHUNK_SCAN_CEILING`]-wide scan on chunk 0's presence (same
    /// sentinel as `log_orphaned_resume_chunks`), but MUST NOT reintroduce
    /// break-on-first-hole *within* a set that does exist — an interrupted
    /// parallel download completes chunks out of order, so `part0`, `part2`,
    /// `part5` present with `part1`/`part3`/`part4` missing is the normal
    /// shape, not evidence the set ends at `part0`. A break-on-first-miss
    /// scan would only remove `part0` here; the fix must remove all three.
    #[tokio::test]
    async fn test_cleanup_legacy_chunks_across_holes_when_chunk_zero_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let complete_data = vec![7u8; 4096];
        tokio::fs::write(&output_path, &complete_data)
            .await
            .unwrap();

        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 64])
            .await
            .unwrap();
        // part1 deliberately absent.
        tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &[2u8; 64])
            .await
            .unwrap();
        // part3, part4 deliberately absent.
        tokio::fs::write(temp_dir.path().join("video.mp4.part5"), &[3u8; 64])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, Some(4096))
            .await
            .unwrap();

        assert_eq!(resume_offset, 4096);
        assert!(
            !temp_dir.path().join("video.mp4.part0").exists(),
            "part0 should have been cleaned up"
        );
        assert!(
            !temp_dir.path().join("video.mp4.part2").exists(),
            "part2 should have been cleaned up despite the hole at part1"
        );
        assert!(
            !temp_dir.path().join("video.mp4.part5").exists(),
            "part5 should have been cleaned up despite the holes at part3/part4"
        );
    }

    /// The chunk-0 sentinel must short-circuit the full scan when chunk 0 is
    /// absent: a legacy set that never wrote id 0 is not a real set to clean
    /// up, and later ids belonging to some other (foreign) file must survive
    /// untouched. This is the observable side effect of the short-circuit —
    /// with the gate skipped, `cleanup_old_chunks` would fall through to the
    /// unconditional `0..CHUNK_SCAN_CEILING` loop and delete these files too.
    #[tokio::test]
    async fn test_cleanup_short_circuits_when_chunk_zero_absent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let complete_data = vec![7u8; 4096];
        tokio::fs::write(&output_path, &complete_data)
            .await
            .unwrap();

        // part0 deliberately absent -- part1/part2 exist but must NOT be
        // reached by the scan once the chunk-0 sentinel gate is in place.
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[1u8; 64])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &[2u8; 64])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, Some(4096))
            .await
            .unwrap();

        assert_eq!(resume_offset, 4096);
        assert!(
            temp_dir.path().join("video.mp4.part1").exists(),
            "part1 must survive: the chunk-0 sentinel gate should skip the scan entirely"
        );
        assert!(
            temp_dir.path().join("video.mp4.part2").exists(),
            "part2 must survive: the chunk-0 sentinel gate should skip the scan entirely"
        );
    }

    /// A foreign file that merely shares the output file's prefix must never
    /// be deleted by chunk cleanup — cleanup only removes exact computed
    /// chunk paths, never a directory sweep.
    #[tokio::test]
    async fn test_cleanup_does_not_delete_foreign_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let complete_data = vec![9u8; 1024];
        tokio::fs::write(&output_path, &complete_data)
            .await
            .unwrap();

        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 64])
            .await
            .unwrap();
        let foreign = temp_dir.path().join("video.mp4.notes.txt");
        tokio::fs::write(&foreign, b"keep me").await.unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, Some(1024))
            .await
            .unwrap();

        assert_eq!(resume_offset, 1024);
        assert!(!temp_dir.path().join("video.mp4.part0").exists());
        assert!(foreign.exists(), "foreign file must survive cleanup");
    }

    /// #568/#559/#571 HIGH: a `.resume{N}` chunk set orphaned by an
    /// abandoned resume attempt must be DISCOVERED (so the operator learns
    /// about it) but MUST NOT be deleted automatically — `download_id` is a
    /// per-process counter starting at 0, so a second concurrent rdlp
    /// process downloading the same output name may be actively writing to
    /// that exact low id right now. Deleting on a bare existence probe would
    /// reintroduce the #558 defect class one crate over. Scenario: the main
    /// output file still exists (ordinary partial-file resume applies), and
    /// orphaned resume chunks sit alongside it.
    #[tokio::test]
    async fn test_orphaned_resume_chunks_not_deleted_when_file_partial() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        tokio::fs::write(&output_path, &vec![1u8; 1000])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.3.resume0"), &[2u8; 64])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.3.resume1"), &[3u8; 64])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, None)
            .await
            .unwrap();

        assert_eq!(resume_offset, 1000);
        // Never deleted: a live concurrent process could own this download_id.
        assert!(temp_dir.path().join("video.mp4.3.resume0").exists());
        assert!(temp_dir.path().join("video.mp4.3.resume1").exists());
    }

    /// Same as above, but there is no main output file and no fresh/legacy
    /// chunk set either — only orphaned resume chunks. They must still be
    /// left untouched (never auto-deleted), same as every other branch.
    #[tokio::test]
    async fn test_orphaned_resume_chunks_not_deleted_when_no_other_chunks_or_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        tokio::fs::write(temp_dir.path().join("video.mp4.5.resume0"), &[4u8; 64])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, None)
            .await
            .unwrap();

        assert_eq!(resume_offset, 0);
        assert!(temp_dir.path().join("video.mp4.5.resume0").exists());
    }

    /// #568 C3: when `detect_chunk_files` resolves to the LEGACY set
    /// (`download_id: None`), the old code's `if
    /// chunk_info.download_id.is_some()` guard skipped the resume-chunk pass
    /// entirely (it was nested inside `cleanup_old_chunks`, only called from
    /// that one `if` arm). Orphaned resume chunks alongside a legacy chunk
    /// set must survive (not be silently unreachable) exactly as they do on
    /// every other branch, and the legacy merge must still proceed normally.
    #[tokio::test]
    async fn test_orphaned_resume_chunks_survive_legacy_chunk_branch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Legacy (old-style) fresh chunks -- no main file, so this is the
        // `detect_chunk_files` branch with `download_id: None`.
        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 512])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[2u8; 512])
            .await
            .unwrap();

        // Orphaned resume chunks for an unrelated download_id.
        tokio::fs::write(temp_dir.path().join("video.mp4.7.resume0"), &[9u8; 64])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, None)
            .await
            .unwrap();

        // Legacy chunks merged normally.
        assert_eq!(resume_offset, 1024);
        assert!(!temp_dir.path().join("video.mp4.part0").exists());
        assert!(!temp_dir.path().join("video.mp4.part1").exists());
        // Orphaned resume chunk untouched.
        assert!(temp_dir.path().join("video.mp4.7.resume0").exists());
    }

    /// #568 C1: resume chunks complete out of order on the adaptive
    /// (`try_buffer_unordered`) path, so a holed set — e.g. `resume0`,
    /// `resume2`, `resume4` present, `resume1`/`resume3` missing — is the
    /// NORMAL case, not evidence the set ends at the first hole. A
    /// break-on-first-missing scan would only discover `resume0` here; the
    /// fix must discover all three. This calls `log_orphaned_resume_chunks`
    /// directly (rather than asserting on file survival, which a no-op
    /// would also satisfy) so the assertion pins the discovery logic itself.
    #[tokio::test]
    async fn test_log_orphaned_resume_chunks_discovers_across_holes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        tokio::fs::write(temp_dir.path().join("video.mp4.9.resume0"), &[1u8; 8])
            .await
            .unwrap();
        // resume1 deliberately absent.
        tokio::fs::write(temp_dir.path().join("video.mp4.9.resume2"), &[2u8; 8])
            .await
            .unwrap();
        // resume3 deliberately absent.
        tokio::fs::write(temp_dir.path().join("video.mp4.9.resume4"), &[3u8; 8])
            .await
            .unwrap();

        let discovered = resume::log_orphaned_resume_chunks(&output_path).await;

        assert_eq!(
            discovered.len(),
            3,
            "expected resume0, resume2, resume4 despite the holes at 1 and 3; got {discovered:?}"
        );
        assert!(discovered.contains(&temp_dir.path().join("video.mp4.9.resume0")));
        assert!(discovered.contains(&temp_dir.path().join("video.mp4.9.resume2")));
        assert!(discovered.contains(&temp_dir.path().join("video.mp4.9.resume4")));

        // Discovery never deletes.
        assert!(temp_dir.path().join("video.mp4.9.resume0").exists());
        assert!(temp_dir.path().join("video.mp4.9.resume2").exists());
        assert!(temp_dir.path().join("video.mp4.9.resume4").exists());
    }

    /// The chunk-0 sentinel optimization must not cause a real orphaned set
    /// to be skipped when chunk 0 IS present alongside later holes (the
    /// common shape); this is the negative-space complement of the previous
    /// test, pinning that an entirely-unused `download_id` (chunk 0 absent)
    /// contributes nothing to the discovered set.
    #[tokio::test]
    async fn test_log_orphaned_resume_chunks_skips_unused_download_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // No files at all for this output path.
        let discovered = resume::log_orphaned_resume_chunks(&output_path).await;
        assert!(discovered.is_empty());
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
