use crate::orchestrator::naming::{finalize_to_clean, part_path};

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
