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
