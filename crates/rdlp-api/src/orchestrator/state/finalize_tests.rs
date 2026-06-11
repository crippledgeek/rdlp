use crate::orchestrator::naming::{
    discard_part, finalize_part, finalize_to_clean, part_path, seam_path,
};

// Pins the round-trip the Single download path relies on: a .rdlp-part file
// finalizes to exactly the clean output name (no marker residue, same dir).
#[tokio::test]
async fn part_file_finalizes_to_clean_name() {
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("Title.mp4");
    let part = part_path(&clean);
    assert_eq!(part, dir.path().join("Title.rdlp-part.mp4"));

    tokio::fs::write(&part, b"bytes").await.unwrap();
    tokio::fs::rename(&part, &clean).await.unwrap();

    assert!(clean.exists(), "clean name must exist after finalize");
    assert!(!part.exists(), "part name must be gone after finalize");
    assert_eq!(finalize_to_clean(&part), Some(clean));
}

#[tokio::test]
async fn finalize_part_renames_and_reports_missing() {
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("Show.mp4");
    let part = part_path(&clean);

    // Success: part exists -> renamed to clean.
    tokio::fs::write(&part, b"data").await.unwrap();
    finalize_part(&part, &clean).await.unwrap();
    assert!(clean.exists() && !part.exists());

    // Failure surfaces with context when the part is missing.
    let err = finalize_part(&part, &clean).await.unwrap_err();
    assert!(
        format!("{err:#}").contains("failed to finalize download"),
        "got: {err:#}"
    );
}

#[tokio::test]
async fn seam_then_finalize_yields_clean_name() {
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("Show.mp4");
    let part = part_path(&clean);
    tokio::fs::write(&part, b"x").await.unwrap();

    let seam = seam_path(&clean);
    finalize_part(&part, &seam).await.unwrap();
    assert!(seam.exists() && !part.exists());

    let target = finalize_to_clean(&seam).unwrap();
    finalize_part(&seam, &target).await.unwrap();
    assert_eq!(target, clean);
    assert!(clean.exists() && !seam.exists());
}

/// Slice 2 (#406): pins the contract of the `finalize_to_clean` + `finalize_part`
/// helper pair that `process_local_file` (and the playlist episode path) depends on.
///
/// - `Some` arm: a temp-named survivor (`Title.rdlp-tmp-<uuid>.mkv`) is renamed
///   to `Title.mkv` on disk; `finalize_to_clean` returns the clean path.
/// - `None` arm: an already-clean-named survivor (`Title.mkv`) passes
///   `finalize_to_clean` returning `None` — the orchestrator treats this as
///   "already at the final name, no rename needed" and uses the path as-is.
///
/// This test pins the naming helper contract itself, not the client/mod.rs
/// wiring (integration testing the full download→finalize path requires
/// `FFmpeg` and is out of scope for unit tests).
#[tokio::test]
async fn process_local_finalize_some_renames_none_passes_through() {
    let dir = tempfile::tempdir().unwrap();

    // --- Some arm: temp-named survivor → clean name via finalize_part ---
    let clean_mkv = dir.path().join("Title.mkv");
    let seam = seam_path(&clean_mkv);
    // The seam name must contain the tmp marker.
    assert!(
        seam.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(".rdlp-tmp-")),
        "seam_path must produce a .rdlp-tmp-* name"
    );
    tokio::fs::write(&seam, b"pipeline output").await.unwrap();

    // finalize_to_clean on the temp-named file returns Some(clean).
    let derived_clean = finalize_to_clean(&seam).expect("Some arm must derive clean path");
    assert_eq!(
        derived_clean, clean_mkv,
        "derived clean path must match target"
    );
    finalize_part(&seam, &derived_clean).await.unwrap();
    assert!(
        clean_mkv.exists(),
        "clean file must exist after finalize_part"
    );
    assert!(!seam.exists(), "seam file must be gone after finalize_part");

    // --- None arm: already-clean survivor → finalize_to_clean returns None ---
    // This is the passthrough case: the file is already at its final name
    // (no pipeline ran, or caller already finalised it).
    assert_eq!(
        finalize_to_clean(&clean_mkv),
        None,
        "finalize_to_clean must return None for a clean-named file (no rename needed)"
    );
    // The file is still intact — no rename or deletion occurred.
    assert!(
        clean_mkv.exists(),
        "clean-named file must be untouched when finalize_to_clean returns None"
    );
}

#[tokio::test]
async fn discard_part_removes_and_is_noop_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("Ep.mp4");
    let part = part_path(&clean);

    tokio::fs::write(&part, b"partial").await.unwrap();
    discard_part(&part).await;
    assert!(!part.exists(), "cancel must remove the .rdlp-part file");
    assert!(!clean.exists(), "cancel must not create the clean name");

    // Idempotent: discarding an already-absent part does not panic/error.
    discard_part(&part).await;
}
