//! Resume state for DASH downloads.
//!
//! State is persisted to `<output>.dash_state.json` and loaded on retry.
//! URL match is path-only — CDN host swaps don't break resume.
//!
//! Load/save are async (`tokio::fs`) because the workspace clippy config
//! bans blocking `std::fs` in async contexts. The file is tiny.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;
use url::Url;

use crate::atomic::now_secs;

/// Current schema version. Bump on incompatible field changes.
pub const STATE_VERSION: u32 = 1;

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
    /// `true` once the video init segment has been written to disk.
    pub init_video_done: bool,
    /// `true` once the audio init segment has been written to disk.
    pub init_audio_done: bool,
    /// `repr_id -> sorted indices of completed segments`.
    pub completed_segments: HashMap<String, Vec<u64>>,
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
            init_video_done: false,
            init_audio_done: false,
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
        let body = fs::read_to_string(path).await.ok()?;
        let s: Self = serde_json::from_str(&body).ok()?;
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

    /// Mark segment `idx` of representation `repr_id` as completed.
    /// Sorted-insert; idempotent for duplicates.
    pub fn record_segment(&mut self, repr_id: &str, idx: u64) {
        let entry = self
            .completed_segments
            .entry(repr_id.to_string())
            .or_default();
        if !entry.contains(&idx) {
            entry.push(idx);
            entry.sort_unstable();
        }
    }

    /// Returns `true` if segment `idx` of `repr_id` has been recorded.
    #[must_use]
    pub fn is_segment_done(&self, repr_id: &str, idx: u64) -> bool {
        self.completed_segments
            .get(repr_id)
            .is_some_and(|v| v.binary_search(&idx).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_then_load_matching_roundtrips_through_atomic_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&url, "v1".into(), Some("a1".into()));
        state.record_segment("v1", 0);
        state.record_segment("v1", 1);
        state.init_video_done = true;
        state.save(&path).await.expect("save");

        let loaded = DashDownloadState::load_matching(&path, &url, "v1", Some("a1"))
            .await
            .expect("must load matching state");
        assert_eq!(loaded.state_version, STATE_VERSION);
        assert_eq!(loaded.video_repr_id, "v1");
        assert!(loaded.init_video_done);
        assert!(loaded.is_segment_done("v1", 0));
        assert!(loaded.is_segment_done("v1", 1));
    }
}
