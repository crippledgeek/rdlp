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
/// completes. Every test that expects `resolve_resume` to merge a
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

/// #573 follow-up: pins that a successful merge still ends in the same
/// observable state as before the fix — every chunk copied into a flushed,
/// fully-readable output, then removed — now that deletion happens strictly
/// after `flush()` rather than interleaved with each copy. Reads the output
/// back via a fresh `tokio::fs::read` (so a merge that "succeeded" but left
/// buffered, unflushed bytes would fail the content check) and asserts every
/// consumed chunk is gone only after that content check passes.
#[tokio::test]
async fn test_merge_chunk_files_deletes_only_after_flush() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output_path = temp_dir.path().join("video.mp4");

    let chunks: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8; 300]).collect();
    let chunk_paths: Vec<_> = (0..5)
        .map(|i| temp_dir.path().join(format!("video.mp4.part{i}")))
        .collect();
    for (path, data) in chunk_paths.iter().zip(&chunks) {
        tokio::fs::write(path, data).await.unwrap();
    }

    let chunk_info = resume::ChunkInfo {
        download_id: None,
        chunk_paths: chunk_paths.clone(),
        chunk_lengths: vec![300; 5],
        total_size: 1500,
        first_bad: None,
        stranded_beyond_gap: 0,
    };

    let total_size = resume::merge_chunk_files(&output_path, &chunk_info)
        .await
        .unwrap();
    assert_eq!(total_size, 1500);

    // The output must be fully flushed and readable — a merge that deleted
    // chunks before flushing could still pass a size-only check.
    let content = tokio::fs::read(&output_path).await.unwrap();
    assert_eq!(content.len(), 1500);
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(&content[i * 300..(i + 1) * 300], chunk.as_slice());
    }

    for path in &chunk_paths {
        assert!(
            !path.exists(),
            "{} should be removed once the merge succeeded and flushed",
            path.display()
        );
    }
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

    // Merge should fail
    let result = resume::merge_chunk_files(&output_path, &chunk_info).await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("video.mp4.part2") || msg.contains("missing chunk"),
        "Expected missing chunk error mentioning part2, got: {msg}"
    );

    // The chunks already copied before the failure are NOT deleted: removal
    // happens only after every chunk is copied and the output is flushed
    // (#573 follow-up), so a failed merge never strands bytes in an
    // unflushed output.
    assert!(
        chunk0_path.exists(),
        "chunk0 must survive a merge that failed on a later chunk"
    );
    assert!(
        chunk1_path.exists(),
        "chunk1 must survive a merge that failed on a later chunk"
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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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

        // #573: the unused old-style set carries no ownership proof (no
        // download_id in its grammar), so it is discovered and logged, never
        // deleted. Inverted from an earlier version of this test that
        // asserted deletion.
        assert!(temp_dir.path().join("video.mp4.part0").exists());
        assert!(temp_dir.path().join("video.mp4.part1").exists());
        assert!(temp_dir.path().join("video.mp4.part2").exists());
    }

    /// #559 acceptance (scan bound) + #573 (ownership): the legacy `0..10`
    /// scan bound never leaked in practice (legacy `concurrent_fragments`
    /// was capped at 10), but a hardcoded ceiling on *discovery* is still a
    /// defect waiting to happen if that assumption ever drifts, so all 12
    /// legacy chunks (2 beyond the old bound) must be DISCOVERED even though
    /// none of them are deleted. Inverted from an earlier version of this
    /// test that asserted deletion — #573 established that a probed legacy
    /// path carries no ownership proof.
    #[tokio::test]
    async fn test_legacy_chunks_beyond_old_ten_chunk_bound_survive() {
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
            .resolve_resume(&output_path, Some(4096))
            .await
            .unwrap()
            .size();

        assert_eq!(resume_offset, 4096);
        for i in 0..12 {
            assert!(
                temp_dir.path().join(format!("video.mp4.part{i}")).exists(),
                "chunk {i} should survive (beyond the old 0..10 bound, #573)"
            );
        }
    }

    /// #571 MEDIUM follow-up (hole tolerance) + #573 (ownership): legacy
    /// discovery gates its [`resume::CHUNK_SCAN_CEILING`]-wide scan on chunk
    /// 0's presence (same sentinel as `log_orphaned_resume_chunks`), but
    /// MUST NOT reintroduce break-on-first-hole *within* a set that does
    /// exist — an interrupted parallel download completes chunks out of
    /// order, so `part0`, `part2`, `part5` present with `part1`/`part3`/
    /// `part4` missing is the normal shape, not evidence the set ends at
    /// `part0`. A break-on-first-miss scan would only discover `part0` here;
    /// the discovery must find all three. Inverted from an earlier version
    /// of this test that asserted deletion — #573 established that a probed
    /// legacy path carries no ownership proof, so nothing here is removed.
    #[tokio::test]
    async fn test_legacy_chunks_across_holes_survive_when_chunk_zero_present() {
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
            .resolve_resume(&output_path, Some(4096))
            .await
            .unwrap()
            .size();

        assert_eq!(resume_offset, 4096);
        assert!(
            temp_dir.path().join("video.mp4.part0").exists(),
            "part0 should survive (#573)"
        );
        assert!(
            temp_dir.path().join("video.mp4.part2").exists(),
            "part2 should survive despite the hole at part1 (#573)"
        );
        assert!(
            temp_dir.path().join("video.mp4.part5").exists(),
            "part5 should survive despite the holes at part3/part4 (#573)"
        );
    }

    /// The chunk-0 sentinel must short-circuit [`resume::log_legacy_chunks`]'s
    /// scan when chunk 0 is absent: a legacy set that never wrote id 0 is not
    /// a real set to discover, and later ids belonging to some other
    /// (foreign) file must not be reported. Since #573 made discovery
    /// log-only, file survival alone is tautological here (nothing is ever
    /// deleted, gated or not) — the sentinel is instead pinned directly by
    /// asserting the discovery call returns an EMPTY vec despite part1/part2
    /// existing on disk.
    #[tokio::test]
    async fn test_log_legacy_chunks_short_circuits_when_chunk_zero_absent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // part0 deliberately absent -- part1/part2 exist but must NOT be
        // reported once the chunk-0 sentinel gate is in place.
        tokio::fs::write(temp_dir.path().join("video.mp4.part1"), &[1u8; 64])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part2"), &[2u8; 64])
            .await
            .unwrap();

        let discovered = resume::log_legacy_chunks(&output_path).await;

        assert!(
            discovered.is_empty(),
            "the chunk-0 sentinel gate should skip the scan entirely, \
             despite part1/part2 existing; got {discovered:?}"
        );
        assert!(temp_dir.path().join("video.mp4.part1").exists());
        assert!(temp_dir.path().join("video.mp4.part2").exists());
    }

    /// A foreign file that merely shares the output file's prefix must never
    /// be touched — discovery only probes exact computed chunk paths, never
    /// a directory sweep. #573: `part0` itself now also survives, since a
    /// probed legacy chunk carries no ownership proof (inverted from an
    /// earlier version of this test that asserted `part0` was deleted).
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
            .resolve_resume(&output_path, Some(1024))
            .await
            .unwrap()
            .size();

        assert_eq!(resume_offset, 1024);
        assert!(
            temp_dir.path().join("video.mp4.part0").exists(),
            "part0 should survive (#573)"
        );
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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

        assert_eq!(resume_offset, 0);
        assert!(temp_dir.path().join("video.mp4.5.resume0").exists());
    }

    /// #568 C3: orphaned resume chunks alongside a legacy chunk set must
    /// survive exactly as they do on every other arm. Before #568 the
    /// resume-chunk pass was nested inside a legacy-cleanup helper that ran
    /// from only one `if` arm, so this layout never reached it. Under #675
    /// the legacy set has no manifest, so it is never merged: `resolve_resume`
    /// resolves to `Fresh` and both the legacy chunks and the orphaned resume
    /// chunk are left in place.
    #[tokio::test]
    async fn test_orphaned_resume_chunks_survive_legacy_chunk_branch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Legacy (old-style) chunks and no main file: `detect_chunk_files`
        // records them as an unverifiable claim, never a mergeable set.
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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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

    /// #573: an orphaned legacy chunk set found alongside an already-complete
    /// file is discovered and logged, never deleted — inverted from an
    /// earlier version of this test that asserted deletion.
    #[tokio::test]
    async fn test_orphaned_legacy_chunks_survive_when_file_complete() {
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
            .resolve_resume(&output_path, Some(2048))
            .await
            .unwrap()
            .size();

        // Should detect file is complete
        assert_eq!(resume_offset, 2048);

        // Verify orphaned chunks survive (#573 — no ownership proof)
        assert!(temp_dir.path().join("video.mp4.part0").exists());
        assert!(temp_dir.path().join("video.mp4.part1").exists());
    }

    /// #573 acceptance (a): a concurrent writer's legacy chunk file must
    /// survive `resolve_resume` on every arm that could otherwise
    /// have deleted it. This covers the PARTIAL-FILE branch specifically —
    /// the complete-file and new-style-chunks-present branches are covered
    /// by the tests above and by `test_prioritize_new_style_over_old_style`.
    #[tokio::test]
    async fn test_legacy_chunks_survive_partial_file_branch() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        // Partial output file: nonzero size, no expected_size given so
        // neither the "already complete" nor "oversized" checks apply —
        // this exercises the plain "found partial download" branch.
        tokio::fs::write(&output_path, &vec![1u8; 500])
            .await
            .unwrap();

        let chunk0 = temp_dir.path().join("video.mp4.part0");
        tokio::fs::write(&chunk0, &[9u8; 64]).await.unwrap();

        let orchestrator = create_test_orchestrator();
        let resume_offset = orchestrator
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

        assert_eq!(resume_offset, 500);
        assert!(
            chunk0.exists(),
            "legacy chunk must survive the partial-file branch (#573)"
        );
    }

    /// #573 acceptance (b): when `merge_chunk_files` fails partway through
    /// `resolve_resume`'s `MergeChunks` arm, the chunks it had not yet
    /// consumed carry no stronger ownership proof than having just been read
    /// once — a set that fails to merge will fail the same way on retry, so
    /// deleting them buys nothing and risks a concurrent writer's file. This
    /// must log, never delete.
    ///
    /// The set is manifest-backed (new-style `video.mp4.0.part{0,1,2}` plus
    /// `write_chunk_manifest`) so `detect_chunk_files` actually selects it
    /// for merging. A legacy `video.mp4.part{i}` set would never reach
    /// `merge_chunk_files` at all under #675 — it becomes an
    /// `UnverifiableClaim` and `resolve_resume` returns `Fresh` from
    /// its `Fresh` arm with nothing touched, which made an earlier version of
    /// this test pass without exercising the failure path (merge review of
    /// PR #744, finding F1). The `detect_chunk_files` assertion below is the
    /// guard against that regression: it fails if the set is ever not
    /// mergeable-shaped.
    ///
    /// `chunk0` ALSO survives: `merge_chunk_files` copies every chunk,
    /// flushes the merged output, and only THEN deletes the chunks it
    /// consumed — so a failure opening `chunk1` means nothing has been
    /// deleted yet, `chunk0` included. So does the manifest: it is deleted
    /// only on a fully successful merge, so the failed attempt can be
    /// re-scanned. `chunk1` is made unreadable (not the *directory*) so
    /// `File::open` fails inside `merge_chunk_files` while directory-level
    /// unlink permission stays intact — proving the survivors survive
    /// because of policy, not because a permission error also happened to
    /// block deletion.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_merge_failure_leaves_chunks_in_place() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        let chunk0 = set.path_in(temp_dir.path(), 0);
        let chunk1 = set.path_in(temp_dir.path(), 1);
        let chunk2 = set.path_in(temp_dir.path(), 2);
        tokio::fs::write(&chunk0, &[1u8; 64]).await.unwrap();
        tokio::fs::write(&chunk1, &[2u8; 64]).await.unwrap();
        tokio::fs::write(&chunk2, &[3u8; 64]).await.unwrap();
        write_chunk_manifest(&set, temp_dir.path(), &[64, 64, 64]).await;
        let manifest_path = set.manifest_path_in(temp_dir.path()).unwrap();

        std::fs::set_permissions(&chunk1, std::fs::Permissions::from_mode(0o000)).unwrap();

        // Anti-vacuity guard: the set must be selected for merging (a
        // `stat` needs no read permission, so chunk1 still verifies here);
        // otherwise the `Fresh` below would come from the "nothing to
        // merge" arm and prove nothing about the failure path.
        let detected = resume::detect_chunk_files(&output_path).await;
        let restore = || {
            let _ = std::fs::set_permissions(&chunk1, std::fs::Permissions::from_mode(0o644));
        };
        let Some(detected) = detected else {
            restore();
            panic!("manifest-backed set must be detected as mergeable");
        };
        if detected.chunk_paths.len() != 3 {
            restore();
            panic!(
                "expected 3 verified chunks, got {}",
                detected.chunk_paths.len()
            );
        }

        let orchestrator = create_test_orchestrator();
        let result = orchestrator.resolve_resume(&output_path, None).await;

        // Restore permissions before any assertion can panic and skip
        // cleanup of the temp dir. Ignore the error: against a deleting
        // (mutated) merge, chunk1 may no longer exist at this point.
        restore();

        let outcome = result.unwrap();
        assert_eq!(
            outcome,
            resume::ResumeOutcome::Fresh,
            "a failed merge resets to a fresh download"
        );
        assert!(
            chunk0.exists(),
            "chunk0 should survive a failed merge: nothing is deleted until \
             the merged output is flushed"
        );
        assert!(
            chunk1.exists(),
            "chunk1 should survive a failed merge (#573)"
        );
        assert!(
            chunk2.exists(),
            "chunk2 should survive a failed merge (#573)"
        );
        assert!(
            manifest_path.exists(),
            "the manifest is only deleted on a successful merge, so the failed \
             attempt can be re-scanned (#675)"
        );
    }

    /// A failed merge must not leave `merge_chunk_files`'s partially written
    /// `output_path` behind: the next run's `plan_resume` would see a
    /// non-empty main file and take the `Resume(size)` arm, stranding the
    /// still-valid chunks and their manifest forever. The file is provably
    /// this arm's own — the chunk arm is only reached when no non-empty main
    /// file existed, and `merge_chunk_files` `File::create`d it — so removing
    /// it is the one deletion here with an ownership proof (#573 keeps every
    /// chunk and the manifest in place, as asserted by
    /// `test_merge_failure_leaves_chunks_in_place`).
    ///
    /// Same fixture as that test: manifest-backed set, chunk 1 unreadable.
    /// RED by mutation: dropping the `remove_file` of `output_path` in the
    /// `Err` arm makes the second `plan_resume` return `Resume(64)`.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_merge_failure_removes_partial_output_so_chunks_are_rediscovered() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        let chunk0 = set.path_in(temp_dir.path(), 0);
        let chunk1 = set.path_in(temp_dir.path(), 1);
        tokio::fs::write(&chunk0, &[1u8; 64]).await.unwrap();
        tokio::fs::write(&chunk1, &[2u8; 64]).await.unwrap();
        write_chunk_manifest(&set, temp_dir.path(), &[64, 64]).await;

        std::fs::set_permissions(&chunk1, std::fs::Permissions::from_mode(0o000)).unwrap();

        let orchestrator = create_test_orchestrator();
        let first = orchestrator.resolve_resume(&output_path, None).await;
        let second = orchestrator.plan_resume(&output_path, None).await;

        // Restore before any assertion can panic and skip temp-dir cleanup.
        let _ = std::fs::set_permissions(&chunk1, std::fs::Permissions::from_mode(0o644));

        assert_eq!(first.unwrap(), resume::ResumeOutcome::Fresh);
        assert!(
            !output_path.exists(),
            "the partially written output of a failed merge must be removed"
        );
        assert!(
            matches!(second, resume::ResumePlan::MergeChunks(_)),
            "the next run must re-discover the manifest-backed chunk set, got {second:?}"
        );
        assert!(
            chunk0.exists() && chunk1.exists(),
            "chunks stay in place (#573)"
        );
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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();

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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();
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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();
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
    /// sum, `merge_chunk_files` fails rather than accepting a misassembled
    /// file. Exercises `merge_chunk_files` directly with a hand-built
    /// `ChunkInfo`: `detect_chunk_files` always derives `total_size` from
    /// the very lengths it verified, so this disagreement cannot be staged
    /// through `resolve_resume` — its fallback to `Fresh` on a merge
    /// failure is covered by `test_merge_failure_leaves_chunks_in_place`.
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
            .resolve_resume(&output_path, None)
            .await
            .unwrap()
            .size();
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

    // ── security review: the scan must be bounded by manifest ENTRY COUNT,
    //    never by an id VALUE the manifest itself claims ──
    //
    // The manifest file is attacker-influenceable by anyone who can write to
    // the output directory (schema/download_id/kind carry no secret). An
    // earlier fix rejected any manifest whose max recorded id exceeded a
    // fixed ceiling at load time — but that ceiling was itself wrong for
    // real inputs: the adaptive download path's chunk size floors at 256
    // KiB indefinitely, so a legitimate download past ~2.44 GiB can produce
    // more chunks than any such ceiling, and would be wrongly rejected as
    // "start fresh". `collect_contiguous_chunks` now walks
    // `manifest.completed` in key order instead of scanning by id span, so
    // its cost is O(entries) regardless of what id values those entries
    // name — these tests pin that directly, including a key near
    // `u64::MAX` completing instantly rather than being rejected or hung.

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

    /// A two-chunk verified prefix followed by a large-valued stranded key:
    /// pins that the prefix length and stranded count are correct when the
    /// break isn't at position 0, and that a large id value (20,000, well
    /// past the old fixed ceiling of 10,000) neither slows nor rejects the
    /// scan — only entry COUNT matters now, not id VALUE.
    #[tokio::test]
    async fn prefix_of_two_then_a_large_valued_stranded_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        for id in [0u64, 1] {
            tokio::fs::write(
                temp_dir.path().join(format!("video.mp4.0.part{id}")),
                &[id as u8; 4],
            )
            .await
            .unwrap();
        }

        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 12);
        manifest.record_completed(0, 4);
        manifest.record_completed(1, 4);
        manifest.record_completed(20_000, 4);

        let scanned = resume::collect_contiguous_chunks(&set, temp_dir.path(), &manifest).await;

        assert_eq!(scanned.chunk_paths.len(), 2, "chunks 0 and 1 verify");
        assert_eq!(
            scanned.first_bad,
            Some(resume::BadChunk {
                id: 2,
                on_disk: None,
                recorded: None,
            }),
            "id 2 is the first gap"
        );
        assert_eq!(scanned.stranded_beyond_gap, 1, "chunk 20,000 is stranded");
    }

    /// A manifest recording a single chunk id near `u64::MAX` must complete
    /// essentially instantly. The pre-fix, id-span scanning implementation
    /// (`for chunk_id in 0..=max_recorded_id`) would have iterated toward
    /// that value and never realistically finished; run here instead of
    /// described only, since walking the map is O(1) for a single entry and
    /// cannot hang — the point being pinned is that a huge key is not
    /// itself special-cased or rejected, just cheap by construction.
    #[tokio::test]
    async fn manifest_with_key_near_u64_max_completes_immediately() {
        let temp_dir = tempfile::tempdir().unwrap();
        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        let mut manifest = ChunkManifest::new(0, ChunkKind::Fresh, 1);
        manifest.record_completed(u64::MAX - 1, 1);

        let started = std::time::Instant::now();
        let scanned = resume::collect_contiguous_chunks(&set, temp_dir.path(), &manifest).await;
        assert!(started.elapsed() < std::time::Duration::from_secs(1));

        assert_eq!(scanned.chunk_paths.len(), 0, "id 0 itself is the gap");
        assert_eq!(
            scanned.first_bad,
            Some(resume::BadChunk {
                id: 0,
                on_disk: None,
                recorded: None,
            })
        );
        assert_eq!(scanned.stranded_beyond_gap, 1);
    }
}

/// #561: `plan_resume`/`resolve_resume` CQS split. `plan_resume` must be a
/// pure query and `resolve_resume` must never silently delete data it can't
/// verify against an (unverified, #674) extractor-reported size.
///
/// The explicit `#[cfg(test)]` below is redundant (this whole file is only
/// ever compiled under `#[cfg(test)] mod tests;` in `orchestrator/mod.rs`),
/// but `scripts/check-no-dir-sweep-delete.sh` textually treats everything
/// before a file's first literal `#[cfg(test)]` as production code; without
/// this marker, this module itself trips the gate: it contains both a
/// `read_dir` call (the survivor assertion in
/// `resolve_resume_never_deletes_an_oversized_file`) and a `remove_file`
/// call (the read-only-dir write probe in
/// `resolve_resume_propagates_a_failed_set_aside`), and the gate cannot tell
/// that neither deletes what the other enumerated.
#[cfg(test)]
mod cqs_resume_split_tests {
    use super::*;
    use crate::orchestrator::resume::ResumeOutcome;

    /// Negative (the issue's own repro): a COMPLETE file whose reported
    /// `expected_size` under-counts it (4096 bytes on disk, 2048 reported)
    /// must survive `resolve_resume` — RED against the pre-#561 code, which
    /// deleted it via `remove_file(..).ok()`.
    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // std::fs helpers in test fixtures — per clippy.toml policy (c)
    async fn resolve_resume_never_deletes_an_oversized_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");
        tokio::fs::write(&output_path, vec![9u8; 4096])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let outcome = orchestrator
            .resolve_resume(&output_path, Some(2048))
            .await
            .unwrap();

        assert_eq!(outcome, ResumeOutcome::Fresh);

        // The bytes must still exist SOMEWHERE under the directory — set
        // aside, not deleted.
        let mut entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "exactly one file must remain: {entries:?}"
        );
        let survivor = entries.remove(0);
        assert_ne!(
            survivor, output_path,
            "the survivor must have been renamed off the original name"
        );
        assert_eq!(
            tokio::fs::read(&survivor).await.unwrap().len(),
            4096,
            "the set-aside file must retain the original bytes"
        );

        // Pin the naming invariant, not just "some file survived": the
        // set-aside must use the `.rdlp-bak-` marker specifically (invisible
        // to `TempRegistry::cleanup_stale`), never `.rdlp-tmp-` (which that
        // sweep marker-scans and would delete on the next stale-cleanup
        // pass — see `naming::bak_sidecar_path`).
        let survivor_name = survivor.file_name().unwrap().to_str().unwrap();
        assert!(
            survivor_name.contains(crate::orchestrator::naming::BAK_MARKER),
            "survivor must carry the .rdlp-bak- marker, got: {survivor_name}"
        );
        assert!(
            !survivor_name.contains(".rdlp-tmp-"),
            "survivor must NOT carry the .rdlp-tmp- marker \
             (TempRegistry::cleanup_stale would delete it), got: {survivor_name}"
        );
    }

    /// `plan_resume` performs no mutation: legacy chunks + an oversized main
    /// file must all still be present on disk after the call. RED against
    /// the pre-#561 `detect_resume_point`, which deleted the oversized file
    /// and the legacy chunks as part of computing the answer.
    #[tokio::test]
    async fn plan_resume_never_mutates_the_filesystem() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");
        tokio::fs::write(&output_path, vec![9u8; 4096])
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("video.mp4.part0"), &[1u8; 64])
            .await
            .unwrap();

        let orchestrator = create_test_orchestrator();
        let _plan = orchestrator.plan_resume(&output_path, Some(2048)).await;

        assert!(
            output_path.exists(),
            "plan_resume must not delete the main file"
        );
        assert!(
            temp_dir.path().join("video.mp4.part0").exists(),
            "plan_resume must not delete legacy chunks"
        );
    }

    /// A failed set-aside must surface as `Err`, never silently fall back to
    /// `Fresh` — RED against the pre-#561 `.ok()`. Making the containing
    /// directory read-only forces the rename underlying
    /// `set_aside_oversized` to fail with a permission error.
    ///
    /// Unix-only: mode-bit permissions are a POSIX concept, and
    /// `PermissionsExt::set_mode` doesn't exist on Windows.
    #[cfg(unix)]
    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // std::fs helpers in test fixtures — per clippy.toml policy (c)
    async fn resolve_resume_propagates_a_failed_set_aside() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");
        std::fs::write(&output_path, vec![9u8; 4096]).unwrap();

        let mut perms = std::fs::metadata(temp_dir.path()).unwrap().permissions();
        perms.set_mode(0o555); // read + execute only: rename needs write on the dir
        std::fs::set_permissions(temp_dir.path(), perms.clone()).unwrap();

        // Mode bits are advisory to a privileged process: CAP_DAC_OVERRIDE
        // (i.e. running as root) bypasses the read-only dir above, which
        // would make the assertion below vacuous. Rather than trust an env
        // var or add a uid syscall dependency, probe directly — attempt a
        // write to the dir we just locked down; if it succeeds, permissions
        // aren't actually being enforced here and the test can't say anything.
        let probe = temp_dir.path().join(".write-probe");
        let permissions_enforced = std::fs::write(&probe, b"x").is_err();
        let _ = std::fs::remove_file(&probe);

        if !permissions_enforced {
            perms.set_mode(0o755);
            std::fs::set_permissions(temp_dir.path(), perms).unwrap();
            eprintln!(
                "skipping resolve_resume_propagates_a_failed_set_aside: \
                 write succeeded through a read-only dir (running as root?) — \
                 the permission-denial this test relies on isn't being enforced"
            );
            return;
        }

        let orchestrator = create_test_orchestrator();
        let result = orchestrator.resolve_resume(&output_path, Some(2048)).await;

        // Restore write permission before the TempDir's Drop tries to clean up.
        perms.set_mode(0o755);
        std::fs::set_permissions(temp_dir.path(), perms).unwrap();

        assert!(
            result.is_err(),
            "a failed set-aside rename must propagate as Err, not silently become Fresh"
        );
    }

    /// The `state/mod.rs` / `playlist/episode.rs` mapping (Complete / Resume
    /// / Fresh) now lives once, inside `resolve_resume` itself.
    #[tokio::test]
    async fn resolve_resume_maps_complete_resume_and_fresh() {
        let temp_dir = tempfile::tempdir().unwrap();
        let orchestrator = create_test_orchestrator();

        let complete_path = temp_dir.path().join("complete.mp4");
        tokio::fs::write(&complete_path, vec![1u8; 100])
            .await
            .unwrap();
        assert_eq!(
            orchestrator
                .resolve_resume(&complete_path, Some(100))
                .await
                .unwrap(),
            ResumeOutcome::Complete { size: 100 }
        );

        let partial_path = temp_dir.path().join("partial.mp4");
        tokio::fs::write(&partial_path, vec![1u8; 50])
            .await
            .unwrap();
        assert_eq!(
            orchestrator
                .resolve_resume(&partial_path, Some(100))
                .await
                .unwrap(),
            ResumeOutcome::Resume(50)
        );

        let fresh_path = temp_dir.path().join("fresh.mp4");
        assert_eq!(
            orchestrator
                .resolve_resume(&fresh_path, Some(100))
                .await
                .unwrap(),
            ResumeOutcome::Fresh
        );
    }

    /// #565: a `.rdlp-part` found already complete is finalized by the
    /// orchestrator without the downloader running again, so the plain-HTTP
    /// resume sidecar the downloader would have removed on its own success
    /// must be removed here — through the sidecar owner's API — or it
    /// outlives the file it described. A partial's sidecar, by contrast,
    /// is exactly what the next resume needs and stays.
    #[tokio::test]
    async fn resolve_resume_complete_removes_the_http_resume_sidecar() {
        use rdlp_downloader::http::HttpResumeState;

        let temp_dir = tempfile::tempdir().unwrap();
        let orchestrator = create_test_orchestrator();

        let complete_path = temp_dir.path().join("complete.mp4");
        tokio::fs::write(&complete_path, vec![1u8; 100])
            .await
            .unwrap();
        let complete_sidecar = HttpResumeState::sidecar_path(&complete_path);
        tokio::fs::write(&complete_sidecar, b"{}").await.unwrap();

        let partial_path = temp_dir.path().join("partial.mp4");
        tokio::fs::write(&partial_path, vec![1u8; 50])
            .await
            .unwrap();
        let partial_sidecar = HttpResumeState::sidecar_path(&partial_path);
        tokio::fs::write(&partial_sidecar, b"{}").await.unwrap();

        assert_eq!(
            orchestrator
                .resolve_resume(&complete_path, Some(100))
                .await
                .unwrap(),
            ResumeOutcome::Complete { size: 100 }
        );
        assert!(
            !complete_sidecar.exists(),
            "a Complete outcome must remove the sidecar the finalized file leaves behind"
        );

        assert_eq!(
            orchestrator
                .resolve_resume(&partial_path, Some(100))
                .await
                .unwrap(),
            ResumeOutcome::Resume(50)
        );
        assert!(
            partial_sidecar.exists(),
            "a Resume outcome must leave the sidecar the next attempt sends as If-Range"
        );
    }

    /// #561 spec-review MEDIUM: a chunk set whose merged total equals
    /// `expected_size` must finalize as `Complete`, not `Resume(expected)`
    /// (which would ask the downloader to resume from EOF and never call
    /// `finalize_part`).
    ///
    /// The set is manifest-backed new-style (`video.mp4.0.part{0,1}` plus
    /// `write_chunk_manifest`) so `detect_chunk_files` actually selects it:
    /// a legacy `video.mp4.part{i}` set is never merged under #675 and would
    /// resolve to `Fresh` without exercising the post-merge mapping at all.
    #[tokio::test]
    async fn resolve_resume_completes_when_merged_total_matches_expected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("video.mp4");

        let set = ChunkSet::for_attempt("video.mp4", 0, ChunkKind::Fresh).unwrap();
        tokio::fs::write(set.path_in(temp_dir.path(), 0), &[1u8; 1024])
            .await
            .unwrap();
        tokio::fs::write(set.path_in(temp_dir.path(), 1), &[2u8; 1024])
            .await
            .unwrap();
        write_chunk_manifest(&set, temp_dir.path(), &[1024, 1024]).await;

        let orchestrator = create_test_orchestrator();
        let outcome = orchestrator
            .resolve_resume(&output_path, Some(2048))
            .await
            .unwrap();

        assert_eq!(
            outcome,
            ResumeOutcome::Complete { size: 2048 },
            "a merged total equal to expected_size must be Complete, not Resume(expected)"
        );
    }
}
