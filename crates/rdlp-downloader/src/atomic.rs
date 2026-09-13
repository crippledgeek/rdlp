//! Shared sidecar-save utilities: an atomic JSON writer, a monotonic-ish
//! wall-clock helper, a consecutive-save-failure tracker, and the one bounded
//! sidecar reader ([`read_json_sidecar`]). Used by the three resume-state
//! sidecars below, each written through [`atomic_write_json`], and by the
//! chunk-completion manifest (`http::chunk_manifest::ChunkManifest`, #675),
//! which shares the writer and the reader but keys on `download_id` +
//! `ChunkKind` rather than on a resume identity:
//!
//! - `fragments::state::HlsResumeState` (`<output>.hls_state.json`) —
//!   matches on schema version plus a path-only FNV-1a-64 fingerprint over
//!   the ordered fragment list, folded with the fragment count.
//! - `dash::state::DashDownloadState` (`<output>.dash_state.json`) —
//!   matches on schema version plus the MPD URL's path and the chosen
//!   video/audio representation ids.
//! - `http::state::HttpResumeState` (`<output>.http_state.json`) — matches
//!   on schema version only; the sidecar *carries* the strong validator
//!   rather than matching against one, and the caller checks it against the
//!   server's response (`If-Range`, RFC 9110 §13.1.5) instead of the sidecar
//!   checking it against the request.

use std::path::Path;
use std::time::SystemTime;

use log::warn;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncReadExt;

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

/// Maximum size, in bytes, of any JSON sidecar [`read_json_sidecar`] will
/// read. A bound on the RESOURCE the read touches (memory for the body,
/// parse time), not on any semantic property of the content — deliberately
/// not tied to a chunk count or id value (see the history in `rdlp-api`'s
/// `collect_contiguous_chunks` for why an id-shaped bound was wrong). The
/// largest legitimate sidecar in this codebase is the chunk-completion
/// manifest (#675), whose `completed` map serializes at roughly 25-30 bytes
/// per entry (`"<digits>":<digits>,`); even an extreme but entirely
/// legitimate ~24 GiB adaptive download (100,000 chunks at the 256 KiB AIMD
/// floor) produces a manifest of a few MB. The next largest is DASH's
/// `completed_segments` (a 24-hour file at 1-second segments, two
/// representations, is about 1 MiB). This cap leaves several times that as
/// headroom before refusing to read a sidecar at all, while still bounding
/// what a corrupt, truncated-and-regrown, or hostile file can pull into
/// memory: a sidecar is trusted state read before any download begins, and
/// reading it must never itself be the thing that exhausts memory.
pub(crate) const MAX_SIDECAR_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB

/// Read and parse a JSON sidecar of at most [`MAX_SIDECAR_BYTES`] at `path`,
/// or `None` for any reason it can't be trusted at the byte level: missing
/// file, unreadable, malformed JSON, or over the bound. Returns the raw
/// deserialized value with NO identity or version check — every sidecar
/// (HLS `HlsResumeState`, DASH `DashDownloadState`, HTTP `HttpResumeState`,
/// and the chunk-completion `ChunkManifest`, #675) has its own notion of
/// "matches what the caller expects" (a fingerprint, an MPD path +
/// representation ids, a schema version, a `download_id` + `ChunkKind`), so
/// that gate stays in each type's own `load`/`load_matching`, which calls
/// this for the read-and-parse mechanics they'd otherwise all duplicate.
///
/// The bound is enforced on the read itself (`take`), not on a size checked
/// beforehand, so a file that grows between the two cannot slip past it.
pub(crate) async fn read_json_sidecar<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut body = Vec::new();
    // One byte past the bound is read so an over-long file is detected as
    // such rather than parsed as a truncation of itself.
    file.take(MAX_SIDECAR_BYTES.saturating_add(1))
        .read_to_end(&mut body)
        .await
        .ok()?;
    if body.len() as u64 > MAX_SIDECAR_BYTES {
        warn!(
            len = body.len(),
            max = MAX_SIDECAR_BYTES;
            "Refusing to read oversized sidecar {}: exceeds the {MAX_SIDECAR_BYTES} byte cap; \
             starting over",
            path.display()
        );
        return None;
    }
    serde_json::from_slice(&body).ok()
}

/// Whether an on-disk artifact whose length was recorded at write time is
/// still intact: an exact-match gate against silent truncation (#675, #677).
///
/// A chunk or segment file cut short by an interrupted write is still
/// non-empty, so "the file exists and has bytes" is not evidence it is
/// complete — only a length match against a value recorded when the write
/// actually finished is. Returns the recorded length (not `on_disk_len`) so
/// that a caller accumulating a verified total can only ever add up lengths
/// it actually checked: both inputs are plain `u64`s captured before this
/// call, so there is no stat/truncate race here to close — the point is
/// simply that the return value traces back to the trusted record, not to
/// whatever happened to be on disk at the moment it was measured.
#[must_use]
pub fn intact_len(on_disk_len: u64, recorded_len: u64) -> Option<u64> {
    (on_disk_len == recorded_len).then_some(recorded_len)
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

    #[test]
    fn intact_len_accepts_exact_match_at_boundary() {
        assert_eq!(intact_len(512, 512), Some(512));
    }

    #[test]
    fn intact_len_rejects_one_byte_short() {
        assert_eq!(intact_len(511, 512), None, "truncated by one byte");
    }

    #[test]
    fn intact_len_rejects_one_byte_over() {
        assert_eq!(intact_len(513, 512), None, "grew past the recorded length");
    }

    #[test]
    fn intact_len_rejects_zero_on_disk_against_nonzero_recorded() {
        assert_eq!(intact_len(0, 512), None);
    }

    /// `Sample` JSON padded with trailing whitespace to exactly `len` bytes;
    /// JSON permits the padding, so only the size bound can reject it.
    fn padded_sample(len: u64) -> Vec<u8> {
        let mut body = serde_json::to_vec(&Sample {
            a: 1,
            b: "x".into(),
        })
        .expect("serialize");
        let len = usize::try_from(len).expect("fits usize");
        assert!(body.len() <= len);
        body.resize(len, b' ');
        body
    }

    /// The bound is inclusive: a sidecar of exactly `MAX_SIDECAR_BYTES` is
    /// read, one byte more is not. Both sides are pinned so a `>=`-for-`>`
    /// slip cannot pass.
    #[tokio::test]
    async fn read_json_sidecar_accepts_at_the_bound_and_rejects_one_past_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let at = dir.path().join("at.json");
        let past = dir.path().join("past.json");
        tokio::fs::write(&at, padded_sample(MAX_SIDECAR_BYTES))
            .await
            .expect("write");
        tokio::fs::write(&past, padded_sample(MAX_SIDECAR_BYTES + 1))
            .await
            .expect("write");
        assert_eq!(
            read_json_sidecar::<Sample>(&at).await,
            Some(Sample {
                a: 1,
                b: "x".into()
            })
        );
        assert_eq!(read_json_sidecar::<Sample>(&past).await, None);
    }

    #[tokio::test]
    async fn read_json_sidecar_is_none_for_missing_or_unparsable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope.json");
        assert_eq!(read_json_sidecar::<Sample>(&missing).await, None);
        let garbage = dir.path().join("garbage.json");
        tokio::fs::write(&garbage, b"{not json")
            .await
            .expect("write");
        assert_eq!(read_json_sidecar::<Sample>(&garbage).await, None);
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
