//! Resume state for DASH downloads.
//!
//! State is persisted to `<output>.dash_state.json` and loaded on retry.
//! URL match is path-only — CDN host swaps don't break resume.
//!
//! Load/save are async (`tokio::fs`) because the workspace clippy config
//! bans blocking `std::fs` in async contexts. The file is tiny.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::atomic::{now_secs, read_json_sidecar};

/// Current schema version.
///
/// v2 (this version) adds the expected byte length alongside each recorded
/// segment/init part (see #677): a part file was previously trusted as
/// "done" purely on non-emptiness. Neither the part write nor the sidecar
/// rename is fsynced (`crate::atomic`), so after a power loss the sidecar
/// can survive with a part recorded while that part's data blocks never
/// reached disk — a short file that was resumed as complete and never
/// re-fetched. A v1 sidecar carries no lengths to validate against, so
/// `load_matching` rejects it outright — the affected download restarts
/// from scratch rather than risk trusting stale records.
pub const STATE_VERSION: u32 = 2;

/// Zero-based index of a media segment within one representation.
pub type SegmentIndex = u64;
/// Byte length of a part as written to disk.
pub type ByteLen = u64;

/// Persisted state of an in-progress DASH download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashDownloadState {
    /// Schema version. Mismatches force a fresh start.
    pub state_version: u32,
    /// Path component of the MPD URL (path-only — CDN-tolerant).
    pub mpd_path: String,
    /// `id` of the chosen video representation.
    pub video_repr_id: String,
    /// `id` of the chosen audio representation, when one is present.
    pub audio_repr_id: Option<String>,
    /// Byte length of the video init segment once written to disk, `None`
    /// until then. Resume trusts the on-disk part only when its length
    /// matches this value exactly.
    pub init_video_len: Option<ByteLen>,
    /// Byte length of the audio init segment once written to disk, `None`
    /// until then. Same matching rule as `init_video_len`.
    pub init_audio_len: Option<ByteLen>,
    /// `repr_id -> (segment index -> expected byte length)` for completed
    /// segments. `BTreeMap` so the serialised sidecar is deterministic.
    pub completed_segments: HashMap<String, BTreeMap<SegmentIndex, ByteLen>>,
    /// Unix epoch seconds — for stale-state diagnosis.
    pub updated_at: u64,
}

impl DashDownloadState {
    /// Construct a fresh state for the given MPD + representation pair.
    #[must_use]
    pub fn new(mpd_url: &Url, video_repr_id: String, audio_repr_id: Option<String>) -> Self {
        Self {
            state_version: STATE_VERSION,
            mpd_path: mpd_url.path().to_string(),
            video_repr_id,
            audio_repr_id,
            init_video_len: None,
            init_audio_len: None,
            completed_segments: HashMap::new(),
            updated_at: now_secs(),
        }
    }

    /// Load state if present and matching the requested MPD URL (path-only).
    /// Returns `None` for: missing file, parse failure, version mismatch,
    /// path mismatch, or repr-id mismatch.
    #[must_use]
    pub async fn load_matching(
        path: &Path,
        mpd_url: &Url,
        video_repr_id: &str,
        audio_repr_id: Option<&str>,
    ) -> Option<Self> {
        let s: Self = read_json_sidecar(path).await?;
        if s.state_version != STATE_VERSION
            || s.mpd_path != mpd_url.path()
            || s.video_repr_id != video_repr_id
            || s.audio_repr_id.as_deref() != audio_repr_id
        {
            return None;
        }
        Some(s)
    }

    /// Persist state to `path`, updating the `updated_at` stamp.
    ///
    /// # Errors
    /// Returns the underlying I/O error from the file write, or a JSON
    /// serialization error wrapped as `io::Error::other`.
    pub async fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.updated_at = now_secs();
        crate::atomic::atomic_write_json(path, self.clone()).await
    }

    /// Record segment `idx` of representation `repr_id` as completed with
    /// its written byte length `len`. Overwrites any prior record for the
    /// same index (idempotent for retries).
    pub fn record_segment(&mut self, repr_id: &str, idx: SegmentIndex, len: ByteLen) {
        self.completed_segments
            .entry(repr_id.to_string())
            .or_default()
            .insert(idx, len);
    }

    /// Returns the recorded byte length for segment `idx` of `repr_id`, or
    /// `None` if it has not been recorded.
    #[must_use]
    pub fn recorded_len(&self, repr_id: &str, idx: SegmentIndex) -> Option<ByteLen> {
        self.completed_segments.get(repr_id)?.get(&idx).copied()
    }

    /// Drop the record for segment `idx` of `repr_id`, so the caller
    /// re-fetches it. No-op if the segment was never recorded.
    pub fn forget_segment(&mut self, repr_id: &str, idx: SegmentIndex) {
        if let Some(v) = self.completed_segments.get_mut(repr_id) {
            v.remove(&idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_then_load_matching_roundtrips_lengths_through_atomic_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&url, "v1".into(), Some("a1".into()));
        state.record_segment("v1", 0, 100);
        state.record_segment("v1", 1, 200);
        state.init_video_len = Some(50);
        state.save(&path).await.expect("save");

        let loaded = DashDownloadState::load_matching(&path, &url, "v1", Some("a1"))
            .await
            .expect("must load matching state");
        assert_eq!(loaded.state_version, STATE_VERSION);
        assert_eq!(loaded.video_repr_id, "v1");
        assert_eq!(loaded.init_video_len, Some(50));
        assert_eq!(loaded.recorded_len("v1", 0), Some(100));
        assert_eq!(loaded.recorded_len("v1", 1), Some(200));
    }

    #[tokio::test]
    async fn v1_shaped_sidecar_is_rejected_not_misread() {
        // A v1 sidecar has no lengths to validate a resumed part against —
        // it must be rejected outright rather than partially parsed, per
        // the STATE_VERSION doc-comment (#677).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let v1_json = format!(
            r#"{{"state_version":1,"mpd_path":"{}","video_repr_id":"v1","audio_repr_id":null,"init_video_done":true,"init_audio_done":false,"completed_segments":{{"v1":[0]}},"updated_at":0}}"#,
            url.path()
        );
        tokio::fs::write(&path, v1_json)
            .await
            .expect("write v1 sidecar");

        assert!(
            DashDownloadState::load_matching(&path, &url, "v1", None)
                .await
                .is_none(),
            "a v1-shaped sidecar must not load — it carries no lengths"
        );
    }

    #[test]
    fn forget_segment_clears_recorded_len() {
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&url, "v1".into(), None);
        state.record_segment("v1", 0, 100);
        assert_eq!(state.recorded_len("v1", 0), Some(100));

        state.forget_segment("v1", 0);
        assert_eq!(state.recorded_len("v1", 0), None);
    }

    #[tokio::test]
    async fn v2_shaped_sidecar_with_old_version_number_is_rejected() {
        // Pins the version gate itself: the body parses as v2, so only the
        // `state_version` comparison in `load_matching` can reject it.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&url, "v1".into(), None);
        state.record_segment("v1", 0, 2);
        state.state_version = STATE_VERSION - 1;
        state.save(&path).await.expect("save");

        assert!(
            DashDownloadState::load_matching(&path, &url, "v1", None)
                .await
                .is_none(),
            "an older state_version must be rejected even when the body parses"
        );
    }
}
