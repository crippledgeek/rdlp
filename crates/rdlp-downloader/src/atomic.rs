//! Shared atomic JSON sidecar writer.

use std::path::Path;

use serde::Serialize;

/// Atomically write `value` as JSON to `path` (write-temp-in-same-dir + rename).
/// A kill mid-write leaves the previous file intact — never a torn file.
///
/// Does NOT fsync: acceptable for resume sidecars (worst case on power loss =
/// re-download one segment). Same-filesystem only — `new_in(dir)` places the
/// temp next to the destination so `persist` is a same-fs rename, not a copy.
/// `tempfile` RAII-cleans the temp on any early return.
///
/// POSIX uses `rename(2)` (fully atomic replace). Windows uses `MoveFileExW` +
/// `MOVEFILE_REPLACE_EXISTING`, which is not atomic if the destination is held
/// open by another process — fine here, the sidecar is a private per-download
/// file. `PersistError` is generic (`PersistError<File>`); `.error` is the
/// inner `std::io::Error` we surface.
///
/// # Errors
/// Returns an `std::io::Error` if serialization, the temp write, or the rename
/// fails, or if the blocking task panics/cancels.
// Callsite is in Task 2 (DASH save migration) — not yet wired.
#[allow(dead_code)]
pub async fn atomic_write_json<T: Serialize + Send + 'static>(
    path: &Path,
    value: T,
) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| std::path::PathBuf::from("."), Path::to_path_buf);
    let dest = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let body = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
        let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
        tmp.write_all(&body)?;
        tmp.persist(&dest).map_err(|e| e.error)?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        a: u32,
        b: String,
    }

    #[tokio::test]
    async fn writes_roundtrippable_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let value = Sample {
            a: 7,
            b: "hi".into(),
        };
        atomic_write_json(&path, value).await.expect("write");
        let body = tokio::fs::read_to_string(&path).await.expect("read");
        let back: Sample = serde_json::from_str(&body).expect("parse");
        assert_eq!(
            back,
            Sample {
                a: 7,
                b: "hi".into()
            }
        );
    }

    #[tokio::test]
    async fn leaves_no_temp_file_behind_on_success() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        atomic_write_json(
            &path,
            Sample {
                a: 1,
                b: "x".into(),
            },
        )
        .await
        .expect("write");
        let mut entries = tokio::fs::read_dir(dir.path()).await.expect("read_dir");
        let mut count = 0;
        while let Some(e) = entries.next_entry().await.expect("entry") {
            assert_eq!(e.file_name(), "state.json", "unexpected leftover temp file");
            count += 1;
        }
        assert_eq!(count, 1, "exactly one file (the destination) must remain");
    }

    #[tokio::test]
    async fn overwrites_existing_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        atomic_write_json(
            &path,
            Sample {
                a: 1,
                b: "first".into(),
            },
        )
        .await
        .expect("write1");
        atomic_write_json(
            &path,
            Sample {
                a: 2,
                b: "second".into(),
            },
        )
        .await
        .expect("write2");
        let body = tokio::fs::read_to_string(&path).await.expect("read");
        let back: Sample = serde_json::from_str(&body).expect("parse");
        assert_eq!(
            back,
            Sample {
                a: 2,
                b: "second".into()
            }
        );
    }
}
