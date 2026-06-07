//! Shared sidecar-save utilities: an atomic JSON writer, a monotonic-ish
//! wall-clock helper, and a consecutive-save-failure tracker. Used by both the
//! HLS (`fragments`) and DASH (`dash`) resume-state modules.

use std::path::Path;
use std::time::SystemTime;

use log::warn;
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

/// Current Unix epoch seconds, or 0 if the system clock predates the epoch.
/// Shared by the HLS and DASH resume-state modules for their `updated_at` stamp.
#[must_use]
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Consecutive resume-sidecar save *attempts* that must fail before a single
/// operator-facing "resume unavailable" notice is emitted. Three consecutive
/// failures indicate a structural condition (disk full, permissions) rather
/// than transient flush jitter — early enough to signal, late enough to avoid
/// false alarms. Applies across protocols (HLS saves per fragment; DASH saves
/// at init, every batch, and final flush). Mirrors the crate's
/// `MAX_CHUNK_RETRIES = 3` retry-count idiom.
pub(crate) const SIDECAR_SAVE_FAILURE_THRESHOLD: u32 = 3;

/// Tracks consecutive resume-sidecar save failures so the caller can emit a
/// single "resume unavailable" notice instead of one log line per failed save.
///
/// `record_failure` returns `true` exactly once — when the consecutive-failure
/// count first reaches the threshold. A successful save resets the consecutive
/// count, but the one-shot warned latch persists for the tracker's lifetime
/// (one notice per download; the operator already has the information).
pub(crate) struct SaveFailureTracker {
    consecutive: u32,
    threshold: u32,
    warned: bool,
}

impl SaveFailureTracker {
    pub(crate) const fn new(threshold: u32) -> Self {
        Self {
            consecutive: 0,
            threshold,
            warned: false,
        }
    }

    /// Record a failed save. Returns `true` exactly once, when the consecutive
    /// failure count first reaches `threshold`, so the caller warns once.
    pub(crate) const fn record_failure(&mut self) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        if !self.warned && self.consecutive >= self.threshold {
            self.warned = true;
            return true;
        }
        false
    }

    /// Record a successful save: resets the consecutive counter. The one-shot
    /// `warned` latch is intentionally NOT reset.
    pub(crate) const fn record_success(&mut self) {
        self.consecutive = 0;
    }
}

/// Record the outcome of a resume-sidecar save attempt and emit a single
/// operator-facing notice once failures become persistent. `proto` is the
/// protocol label for the message ("HLS" / "DASH"). Centralizes the save-result
/// handling shared by the HLS fragment downloader and the 3 DASH save sites.
pub(crate) fn note_sidecar_save(
    result: std::io::Result<()>,
    tracker: &mut SaveFailureTracker,
    proto: &str,
) {
    match result {
        Ok(()) => tracker.record_success(),
        Err(e) => {
            if tracker.record_failure() {
                warn!(
                    "{proto} resume state could not be saved after {SIDECAR_SAVE_FAILURE_THRESHOLD} consecutive attempts ({e}); resume will be unavailable if this download is interrupted"
                );
            }
        }
    }
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

    #[cfg_attr(miri, ignore)]
    #[test]
    fn now_secs_is_after_2020() {
        // 1_577_836_800 = 2020-01-01T00:00:00Z. A real clock is well past it;
        // this guards against a regression that returns 0 on the happy path.
        assert!(
            now_secs() > 1_577_836_800,
            "now_secs must reflect a real clock"
        );
    }

    #[test]
    fn tracker_warns_once_at_threshold_then_stays_silent() {
        let mut t = SaveFailureTracker::new(3);
        assert!(!t.record_failure(), "1st failure: below threshold");
        assert!(!t.record_failure(), "2nd failure: below threshold");
        assert!(
            t.record_failure(),
            "3rd consecutive failure: warn exactly here"
        );
        assert!(
            !t.record_failure(),
            "4th failure: already warned, stay silent"
        );
        assert!(!t.record_failure(), "5th failure: still silent");
    }

    #[test]
    fn tracker_success_resets_consecutive_count() {
        let mut t = SaveFailureTracker::new(3);
        assert!(!t.record_failure());
        assert!(!t.record_failure());
        t.record_success(); // resets the streak
        assert!(!t.record_failure(), "post-reset 1st failure");
        assert!(!t.record_failure(), "post-reset 2nd failure");
        assert!(t.record_failure(), "post-reset 3rd failure trips threshold");
    }

    #[test]
    fn tracker_warned_latch_survives_later_success_and_failures() {
        // Once warned, a recovery then a fresh failure streak does NOT re-warn:
        // one notice per download (operator already has the information).
        let mut t = SaveFailureTracker::new(2);
        assert!(!t.record_failure());
        assert!(t.record_failure(), "warns at threshold 2");
        t.record_success();
        assert!(!t.record_failure(), "post-warn streak: no second warning");
        assert!(!t.record_failure(), "still no second warning");
    }

    #[test]
    fn tracker_threshold_one_warns_on_first_failure() {
        let mut t = SaveFailureTracker::new(1);
        assert!(t.record_failure(), "threshold 1 warns immediately");
        assert!(!t.record_failure());
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
