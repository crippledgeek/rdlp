//! Shared sidecar-save utilities: an atomic JSON writer, a monotonic-ish
//! wall-clock helper, and a consecutive-save-failure tracker. Used by both the
//! HLS (`fragments`) and DASH (`dash`) resume-state modules.
//!
//! # Durability position (#676): no `fsync` on any resume path
//!
//! Each protocol tolerates a crash between a write and its commit to disk
//! without syncing:
//!
//! - **DASH** writes one file per segment; a torn segment costs one re-fetch.
//! - **HTTP** has no sidecar on `develop`; each ranged chunk is re-validated
//!   against the origin's `Content-Range` (#526). PR #747 adds a
//!   validator-carrying `<output>.http_state.json` checked with `If-Range` on
//!   every resume — in both shapes nothing depends on on-disk write ordering,
//!   so no sync.
//! - **HLS** appends to one output and records the running CRC-32 of every
//!   byte written in the sidecar; a resume re-hashes the partial's
//!   `[0..byte_len)` and starts fresh on a mismatch (`fragments::state`).
//!
//! The sidecar itself is not the hazard: it is replaced by `rename(2)`, which
//! is an atomic name swap, so a reader sees either the old or the new document
//! (btrfs FAQ; ext4's default-on `auto_da_alloc` flushes the data of a
//! replace-via-rename before the rename) and a torn or zero-length one fails
//! JSON parse ⇒ fresh start. The output file is: its size can be durable
//! while its appended data is not — ext4 `data=ordered` with delayed
//! allocation (Ts'o, 2009: `auto_da_alloc` "will not solve the problem for
//! newly created files") and XFS, which journals metadata only, can leave
//! `byte_len` bytes of which the tail reads back as zeros; btrfs is exempt
//! ("waits until data extents are on disk before updating metadata"). `fsync(2)`
//! NOTES documents that flushing is the application's responsibility.
//!
//! Syncing instead was measured (btrfs on dm-crypt SSD, 2 MiB × 200
//! fragments, best of 3): 0.49 ms/fragment unsynced vs 16.2 ms with an
//! `fsync` of the sidecar, 28.2 ms with `fdatasync` of the output plus the
//! sidecar `fsync`, 11.6 ms for `fdatasync` of the output alone, and a
//! barrier every 8th fragment 0.5 ms median but 42 ms p99 (the middle ground
//! only moves the stall, it does not remove it). A 2 MiB fragment arrives in
//! ~16 ms on gigabit, so a per-fragment sync halves HLS throughput on a fast
//! link, and HDDs are worse. Detecting the hole on
//! resume costs one read of the partial and catches any hole anywhere in the
//! prefix, on every filesystem, with no ordering assumption. yt-dlp and aria2
//! write their state files unsynced as well.

use std::path::Path;
use std::time::SystemTime;

use log::warn;
use serde::Serialize;
use tokio::io::AsyncReadExt as _;

/// Read-buffer size for hashing a partial on resume. 1 MiB keeps the verify
/// to a handful of syscalls per fragment-sized partial without holding a
/// whole multi-GiB output in memory.
const VERIFY_READ_BUF: usize = 1024 * 1024;

/// Atomically write `value` as JSON to `path` (write-temp-in-same-dir + rename).
/// A kill mid-write leaves the previous file intact — never a torn file.
///
/// Does NOT fsync (see the module doc for the per-protocol position).
/// Same-filesystem only — `new_in(dir)` places the
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

/// CRC-32/IEEE of the first `len` bytes of the file at `path`, read in
/// `VERIFY_READ_BUF` chunks. The one prefix-hashing routine: the HLS resume
/// verify and the tests that seed sidecars both use it, so the stored value
/// has a single definition. Lives here because this is the module every
/// sidecar shares, so there is one hashing definition for production and
/// tests rather than one per protocol.
///
/// # Errors
/// Returns the underlying I/O error, or `UnexpectedEof` if the file is
/// shorter than `len`.
pub(crate) async fn crc32_of_prefix(path: &Path, len: u64) -> std::io::Result<u32> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; VERIFY_READ_BUF];
    let mut remaining = len;
    while remaining > 0 {
        let want =
            usize::try_from(remaining.min(VERIFY_READ_BUF as u64)).unwrap_or(VERIFY_READ_BUF);
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("partial is shorter than the recorded {len} bytes"),
            ));
        }
        hasher.update(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(hasher.finalize())
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

    #[tokio::test]
    async fn crc32_of_prefix_matches_the_ieee_check_value() {
        // CRC-32/IEEE check value: crc("123456789") == 0xCBF43926.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("partial");
        tokio::fs::write(&path, b"123456789").await.expect("write");
        assert_eq!(crc32_of_prefix(&path, 9).await.expect("hash"), 0xCBF4_3926);
    }

    #[tokio::test]
    async fn crc32_of_prefix_ignores_bytes_past_len_and_hashes_empty_as_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("partial");
        tokio::fs::write(&path, b"123456789TRAILING")
            .await
            .expect("write");
        assert_eq!(
            crc32_of_prefix(&path, 9).await.expect("hash"),
            0xCBF4_3926,
            "bytes beyond len must not contribute"
        );
        assert_eq!(crc32_of_prefix(&path, 0).await.expect("hash"), 0);
    }

    #[tokio::test]
    async fn crc32_of_prefix_errors_when_file_is_shorter_than_len() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("partial");
        tokio::fs::write(&path, b"12345").await.expect("write");
        let err = crc32_of_prefix(&path, 6).await.expect_err("short file");
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        let missing = dir.path().join("nope");
        assert!(crc32_of_prefix(&missing, 1).await.is_err());
    }

    #[tokio::test]
    async fn crc32_of_prefix_distinguishes_zeroed_tail_from_real_tail() {
        // The property the HLS resume gate rests on (#676): a prefix whose
        // tail never reached disk reads back as zeros of the same length, and
        // CRC-32/IEEE (all-ones init and xorout) does not hash appended zeros
        // to the same value as the real bytes.
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real");
        let zeroed = dir.path().join("zeroed");
        let prefix = b"prefix-bytes-";
        let tail_len = 4096usize;
        let mut real_bytes = prefix.to_vec();
        real_bytes.extend((0..tail_len).map(|i| (i % 253) as u8 + 1));
        let mut zeroed_bytes = prefix.to_vec();
        zeroed_bytes.extend(std::iter::repeat_n(0u8, tail_len));
        tokio::fs::write(&real, &real_bytes).await.expect("write");
        tokio::fs::write(&zeroed, &zeroed_bytes)
            .await
            .expect("write");
        let len = real_bytes.len() as u64;
        assert_ne!(
            crc32_of_prefix(&real, len).await.expect("hash"),
            crc32_of_prefix(&zeroed, len).await.expect("hash"),
            "same length, zeroed tail must not collide with the real tail"
        );
    }

    #[tokio::test]
    async fn crc32_of_prefix_spans_read_buffer_boundaries() {
        // len > VERIFY_READ_BUF forces the chunk loop; the one-shot library
        // hash over the same bytes is the oracle.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("partial");
        let len = VERIFY_READ_BUF * 2 + 7;
        let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        tokio::fs::write(&path, &bytes).await.expect("write");
        assert_eq!(
            crc32_of_prefix(&path, len as u64).await.expect("hash"),
            crc32fast::hash(&bytes)
        );
        assert_eq!(
            crc32_of_prefix(&path, VERIFY_READ_BUF as u64 + 1)
                .await
                .expect("hash"),
            crc32fast::hash(&bytes[..=VERIFY_READ_BUF])
        );
    }
}
