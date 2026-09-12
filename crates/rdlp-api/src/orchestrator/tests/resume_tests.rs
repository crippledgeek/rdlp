//! Tests for chunk merge and resume compatibility
#![allow(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::*;
use rdlp_downloader::{ChunkKind, ChunkManifest, ChunkSet};

/// Write a chunk-completion manifest (#675) for a new-style Fresh chunk set,
/// standing in for what the real parallel downloader records as each chunk
/// completes. Every test that expects `detect_resume_point` to merge a
/// new-style chunk set now needs one — without it the set is unverifiable
/// and is left in place rather than merged (#675's policy change).
///
/// Takes the already-built `ChunkSet` (rather than the `(filename,
/// download_id)` pair it's built from) so this stays a 3-argument helper —
/// `set.download_id()` recovers the id its manifest content needs.
async fn write_chunk_manifest(set: &ChunkSet, dir: &std::path::Path, lengths: &[u64]) {
    let download_id = set
        .download_id()
        .expect("a manifest is only ever written for a new-style (Fresh) set");
    let total: u64 = lengths.iter().sum();
    let mut manifest = ChunkManifest::new(download_id, ChunkKind::Fresh, total);
    for (chunk_id, len) in lengths.iter().enumerate() {
        manifest.record_completed(chunk_id as u64, *len);
    }
    let manifest_path = set
        .manifest_path_in(dir)
        .expect("new-style set has a manifest path");
    manifest.save(&manifest_path).await.expect("save manifest");
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
        chunk_lengths: vec![512, 512, 512],
        total_size: 1536,
        first_bad: None,
        stranded_beyond_gap: 0,
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
        chunk_lengths: vec![512, 512, 512],
        total_size: 1536,
        first_bad: None,
        stranded_beyond_gap: 0,
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
        chunk_lengths: vec![0, 0],
        total_size: 0,
        first_bad: None,
        stranded_beyond_gap: 0,
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

    /// #675 policy change: the legacy chunk grammar predates the
    /// chunk-completion manifest and has no writer that could ever produce
    /// one, so a legacy chunk set is now permanently unverifiable and is
    /// never merged — this is a deliberate acceptance of the change, not the
    /// pre-#675 behaviour this test used to pin (which merged on bare
    /// non-empty-file trust).
    #[tokio::test]
    async fn test_detect_old_style_chunks_unverifiable_without_manifest() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

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

        assert_eq!(resume_offset, 0, "unverifiable legacy chunks start fresh");
        assert!(!output_path.exists());

        // Left in place, not deleted — #675's "leave the files" policy.
        assert!(temp_dir.path().join("video.mp4.part0").exists());
        assert!(temp_dir.path().join("video.mp4.part1").exists());
        assert!(temp_dir.path().join("video.mp4.part2").exists());
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
        // #675: without a manifest recording these 5 chunks complete, the
        // set is unverifiable and would never be merged.
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[256, 256, 256, 256, 256],
        )
        .await;

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
        assert!(
            !temp_dir.path().join("video.mp4.0.chunks.json").exists(),
            "the manifest must not outlive the merge it verified"
        );
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
        // #675: only the new-style set has a manifest — the legacy set can
        // never have one, so it stays unverifiable regardless of priority.
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[256, 256, 256, 256, 256],
        )
        .await;

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

        // #675: the legacy set can never have a manifest, so it is
        // unverifiable and is never merged — left in place, not "merged
        // normally" as this test pinned before #675.
        assert_eq!(resume_offset, 0);
        assert!(temp_dir.path().join("video.mp4.part0").exists());
        assert!(temp_dir.path().join("video.mp4.part1").exists());
        // Orphaned resume chunk untouched (unrelated assertion, unaffected
        // by #675 — this is what the test was originally pinning).
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
        // #675: only download_id 2 gets a manifest — download_id 0 stays
        // unverifiable, which is consistent with the pre-#675 assertion
        // below that its chunks are simply never touched (not chosen,
        // because it isn't the highest id — it was never merge-eligible
        // either way once verification applies).
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 2, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[512, 512, 512],
        )
        .await;

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
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[128; 100],
        )
        .await;

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

/// #675: chunk length trust must come from a recorded manifest, not a bare
/// non-empty-file check, and a gap must be reported rather than silently
/// truncating the merge.
mod issue_675_chunk_integrity_tests {
    use super::*;

    /// (i) A chunk truncated by one byte relative to its recorded length
    /// must not be accepted into the merged prefix. RED against the
    /// unpatched code, which trusted any non-empty file at its truncated
    /// length.
    #[tokio::test]
    async fn truncated_chunk_is_not_merged() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 256])
            .await
            .unwrap();
        // Chunk 1 recorded as 256 bytes complete, but only 255 are on disk —
        // an interrupted write.
        tokio::fs::write(temp_dir.path().join("video.mp4.0.part1"), &[2u8; 255])
            .await
            .unwrap();
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[256, 256],
        )
        .await;

        let info = resume::detect_chunk_files(&output_path)
            .await
            .expect("chunk 0 alone is a verified, non-empty prefix");

        assert_eq!(
            info.chunk_paths.len(),
            1,
            "only the intact chunk 0 is trusted; the truncated chunk 1 is not"
        );
        assert_eq!(info.total_size, 256);
        assert_eq!(
            info.stranded_beyond_gap, 0,
            "chunk 1 itself is the break, nothing recorded lies beyond it"
        );
        // Spec review finding 1: assert the REPORT itself, not just the
        // merged offset — a truncated LAST chunk (nothing recorded beyond
        // it) leaves `stranded_beyond_gap` at 0, so that alone can't tell
        // this apart from "the manifest just ended cleanly at chunk 0".
        assert_eq!(
            info.first_bad,
            Some(resume::BadChunk {
                id: 1,
                on_disk: Some(255),
                recorded: Some(256),
            }),
            "the truncation must be reported with the chunk id and both lengths"
        );

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, None)
            .await
            .unwrap();
        assert_eq!(resume_offset, 256, "only chunk 0 was merged");
        assert!(
            temp_dir.path().join("video.mp4.0.part1").exists(),
            "the truncated chunk is left in place, not silently consumed"
        );
    }

    /// (ii) A real gap (chunk 3 never recorded complete) with chunks 4 and 5
    /// present and recorded beyond it: the merge uses the verified prefix
    /// 0..=2, and the 2 chunks beyond the gap are reported rather than
    /// silently dropped.
    #[tokio::test]
    async fn gap_reports_chunks_stranded_beyond_it() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        for id in [0u64, 1, 2, 4, 5] {
            tokio::fs::write(
                temp_dir.path().join(format!("video.mp4.0.part{id}")),
                &[id as u8; 64],
            )
            .await
            .unwrap();
        }
        // Chunk 3 deliberately never recorded — a real gap. 4 and 5 ARE
        // recorded complete, standing in for chunks that finished on the
        // adaptive path's out-of-order completion before the process died.
        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 384);
        for id in [0u64, 1, 2, 4, 5] {
            manifest.record_completed(id, 64);
        }
        manifest
            .save(&set.manifest_path_in(temp_dir.path()).unwrap())
            .await
            .unwrap();

        let info = resume::detect_chunk_files(&output_path)
            .await
            .expect("0..=2 is a verified prefix");

        assert_eq!(info.chunk_paths.len(), 3, "only 0, 1, 2 are merge-eligible");
        assert_eq!(info.total_size, 192);
        assert_eq!(
            info.stranded_beyond_gap, 2,
            "chunks 4 and 5 are recorded complete but unreachable past the gap at 3"
        );
        assert_eq!(
            info.first_bad,
            Some(resume::BadChunk {
                id: 3,
                on_disk: None,
                recorded: None,
            }),
            "the gap itself must be reported by id, not just its downstream effect"
        );

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, None)
            .await
            .unwrap();
        assert_eq!(resume_offset, 192, "merge stops at the verified prefix");
        assert!(temp_dir.path().join("video.mp4.0.part4").exists());
        assert!(temp_dir.path().join("video.mp4.0.part5").exists());

        // Code-quality review finding 3: a PARTIAL merge must NOT delete the
        // manifest — chunks 4 and 5 are still only recorded there, and
        // deleting it would make that record unrecoverable (the exact
        // silence #675 exists to prevent).
        let manifest_path = set.manifest_path_in(temp_dir.path()).unwrap();
        assert!(
            tokio::fs::metadata(&manifest_path).await.is_ok(),
            "the manifest must survive a partial merge so stranded chunks stay reported"
        );

        // A retained manifest must re-warn on the next scan: chunk 0's file
        // was consumed by the merge above, so it is now the new break
        // (`first_bad{id:0}`), and 1, 2, 4, 5 are all still recorded
        // complete but unreachable — the manifest keeps doing its job
        // across repeated scans rather than being spent after one.
        // `detect_chunk_files` would return `None` here (chunk_paths is
        // empty), so assert via `collect_contiguous_chunks` directly.
        assert!(
            resume::detect_chunk_files(&output_path).await.is_none(),
            "chunk 0's file is gone, so nothing is merge-eligible on the rescan"
        );
        let reloaded = ChunkManifest::load_matching(&manifest_path, 0, ChunkKind::Fresh)
            .await
            .expect("the retained manifest must still load");
        let rescanned = resume::collect_contiguous_chunks(&set, temp_dir.path(), &reloaded).await;
        assert_eq!(
            rescanned.first_bad,
            Some(resume::BadChunk {
                id: 0,
                on_disk: None,
                recorded: Some(64),
            }),
            "chunk 0 is now the break: recorded complete but its file is gone"
        );
        assert_eq!(
            rescanned.stranded_beyond_gap, 4,
            "chunks 1, 2, 4, 5 are all still recorded complete but unreachable"
        );
    }

    /// (iii) If the assembled merge total no longer matches the recorded
    /// sum (a chunk changed size between detection and merge), the merge
    /// fails and `detect_resume_point` falls back to starting fresh rather
    /// than accepting a misassembled file.
    #[tokio::test]
    async fn merge_total_mismatch_against_recorded_sum_fails_the_merge() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let chunk0 = temp_dir.path().join("video.mp4.0.part0");
        tokio::fs::write(&chunk0, &[1u8; 256]).await.unwrap();

        let chunk_info = resume::ChunkInfo {
            download_id: Some(0),
            chunk_paths: vec![chunk0.clone()],
            // Matches the 256-byte file exactly, so the PER-CHUNK check
            // passes — this test's oracle is the separate merged-TOTAL
            // check below it, not the per-chunk one (spec review finding
            // 2: `chunk_lengths: [512]` against this same 256-byte file
            // trips the per-chunk check first and never reaches the total
            // check at all, so deleting that check leaves this test green).
            chunk_lengths: vec![256],
            // Disagrees with the sum of `chunk_lengths` (256), even though
            // every individual chunk was copied exactly as recorded.
            total_size: 512,
            first_bad: None,
            stranded_beyond_gap: 0,
        };

        let result = resume::merge_chunk_files(&output_path, &chunk_info).await;
        assert!(
            result.is_err(),
            "a merged total disagreeing with the recorded sum must fail the merge \
             even when every individual chunk matched its recorded length"
        );
    }

    /// (iv) A new-style chunk set with NO manifest at all (pre-#675 run, or
    /// the manifest itself lost) is unverifiable and is never merged — a
    /// deliberate policy decision (#675's acceptance criteria), not a
    /// default. Files are left in place.
    #[tokio::test]
    async fn chunk_set_with_no_manifest_is_never_merged() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 256])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.0.part1"), &[2u8; 256])
            .await
            .unwrap();
        // No manifest written.

        assert!(
            resume::detect_chunk_files(&output_path).await.is_none(),
            "an unverifiable chunk set must not be reported as mergeable"
        );

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .detect_resume_point(&output_path, None)
            .await
            .unwrap();
        assert_eq!(resume_offset, 0);
        assert!(temp_dir.path().join("video.mp4.0.part0").exists());
        assert!(temp_dir.path().join("video.mp4.0.part1").exists());
    }

    /// Boundary pair for `intact_len` at the recorded chunk length N,
    /// exercised through `detect_chunk_files` rather than the unit-level
    /// `intact_len` tests in `rdlp-downloader` (those pin the helper; this
    /// one pins that the caller actually wires it through).
    #[tokio::test]
    async fn on_disk_length_exactly_n_is_accepted() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");
        tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 512])
            .await
            .unwrap();
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[512],
        )
        .await;

        let info = resume::detect_chunk_files(&output_path).await.unwrap();
        assert_eq!(info.chunk_paths.len(), 1);
        assert_eq!(info.total_size, 512);
    }

    #[tokio::test]
    async fn on_disk_length_n_minus_one_is_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");
        tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 511])
            .await
            .unwrap();
        write_chunk_manifest(
            &ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap(),
            temp_dir.path(),
            &[512],
        )
        .await;

        assert!(resume::detect_chunk_files(&output_path).await.is_none());
    }

    // ── security review: unbounded scan via an attacker-controlled manifest key ──
    //
    // The manifest file is attacker-influenceable by anyone who can write to
    // the output directory (schema/download_id/kind carry no secret), and
    // its `completed` keys used to drive `collect_contiguous_chunks`'s scan
    // range directly. A single `{"<huge id>": len}` entry made recovery
    // iterate towards that id — near `u64::MAX` it never finished.

    /// RED against the pre-fix loader: a manifest recording a chunk id near
    /// `u64::MAX` used to load successfully.
    #[tokio::test]
    async fn manifest_with_near_max_key_is_rejected_at_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 1);
        manifest.record_completed(u64::MAX - 1, 1);
        manifest.save(&path).await.unwrap();

        assert!(
            ChunkManifest::load_matching(&path, 0, ChunkKind::Fresh)
                .await
                .is_none(),
            "a manifest key near u64::MAX must never be trusted"
        );
    }

    #[tokio::test]
    async fn manifest_with_key_one_beyond_ceiling_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 1);
        manifest.record_completed(rdlp_downloader::CHUNK_SCAN_CEILING + 1, 1);
        manifest.save(&path).await.unwrap();

        assert!(
            ChunkManifest::load_matching(&path, 0, ChunkKind::Fresh)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn manifest_with_key_exactly_at_ceiling_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 1);
        manifest.record_completed(rdlp_downloader::CHUNK_SCAN_CEILING, 1);
        manifest.save(&path).await.unwrap();

        assert!(
            ChunkManifest::load_matching(&path, 0, ChunkKind::Fresh)
                .await
                .is_some(),
            "the ceiling itself is a legitimate boundary value, not a rejection"
        );
    }

    /// Pins the count-from-map path: a sparse manifest `{0: n, 5000: n}`
    /// must complete quickly and report exactly 1 stranded chunk, without
    /// depending on iterating every id up to 5000.
    #[tokio::test]
    async fn sparse_manifest_scan_completes_bounded_and_counts_via_the_map() {
        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.0.part0"), &[1u8; 4])
            .await
            .unwrap();

        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 8);
        manifest.record_completed(0, 4);
        manifest.record_completed(5000, 4);

        let started = std::time::Instant::now();
        let scanned = resume::collect_contiguous_chunks(&set, temp_dir.path(), &manifest).await;
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "must not iterate id-by-id up to the sparse key"
        );

        assert_eq!(
            scanned.chunk_paths.len(),
            1,
            "only chunk 0 is merge-eligible"
        );
        assert_eq!(
            scanned.first_bad,
            Some(resume::BadChunk {
                id: 1,
                on_disk: None,
                recorded: None,
            }),
            "chunk 1 is a real gap"
        );
        assert_eq!(
            scanned.stranded_beyond_gap, 1,
            "chunk 5000 is recorded complete but unreachable past the gap at 1"
        );
    }
}
