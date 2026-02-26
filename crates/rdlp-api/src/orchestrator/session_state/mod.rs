//! Session state persistence for resuming interrupted downloads
//!
//! Saves user selections (format, subtitles, audio type) to a JSON file
//! so interrupted downloads can resume without re-prompting. Download
//! progress (HLS segments, HTTP byte offsets) is handled separately by
//! the downloader crate.

use chrono::{DateTime, Utc};
use log::debug;
use rdlp_security::extract_url_path;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// Current schema version. Bumped on breaking changes to force fresh state.
const STATE_VERSION: u32 = 1;

/// Session state persisted to disk for resume support.
///
/// Uses a serde-tagged enum so the JSON `state_type` field determines
/// which variant to deserialize into.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state_type")]
pub(super) enum SessionState {
    /// State for a single video download
    #[serde(rename = "single_video")]
    SingleVideo(SingleVideoState),
    /// State for a playlist download
    #[serde(rename = "playlist")]
    Playlist(PlaylistState),
}

/// Persisted selections for a single video download.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct SingleVideoState {
    /// Schema version for forward compatibility
    pub version: u32,
    /// URL path (no host/query) for CDN-tolerant matching
    pub source_url: String,
    /// Video title at time of selection
    pub title: String,
    /// Selected format identifier (matched against available formats on resume)
    pub format_id: String,
    /// Audio format ID for merge downloads (None for single-format downloads)
    #[serde(default)]
    pub audio_format_id: Option<String>,
    /// Selected subtitle language codes (empty if none)
    pub subtitle_langs: Vec<String>,
    /// When this state was first created
    pub created_at: DateTime<Utc>,
    /// When this state was last updated
    pub updated_at: DateTime<Utc>,
}

/// Persisted selections for a playlist download.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct PlaylistState {
    /// Schema version for forward compatibility
    pub version: u32,
    /// URL path (no host/query) for CDN-tolerant matching
    pub source_url: String,
    /// Playlist title at time of selection
    pub playlist_title: String,
    /// Selected audio type (e.g. "SUB", "DUB"), None if all kept
    pub selected_audio: Option<String>,
    /// Selected subtitle language codes (empty if none)
    pub selected_sub_langs: Vec<String>,
    /// Whether strict subtitle mode was active
    #[serde(default)]
    pub strict_subs: bool,
    /// Episodes that failed in the previous run (empty if none)
    #[serde(default)]
    pub failed_episodes: Vec<FailedEpisode>,
    /// When this state was first created
    pub created_at: DateTime<Utc>,
    /// When this state was last updated
    pub updated_at: DateTime<Utc>,
}

/// Record of a failed episode for cross-run retry tracking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct FailedEpisode {
    /// 1-based position in the playlist
    pub position: usize,
    /// Episode title at time of failure
    pub title: String,
    /// Last error message
    pub last_error: String,
}

impl SingleVideoState {
    /// Create a new single video state with normalized URL.
    pub fn new(
        url: &str,
        title: impl Into<String>,
        format_id: impl Into<String>,
        subtitle_langs: Vec<String>,
        audio_format_id: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            version: STATE_VERSION,
            source_url: extract_url_path(url),
            title: title.into(),
            format_id: format_id.into(),
            audio_format_id,
            subtitle_langs,
            created_at: now,
            updated_at: now,
        }
    }
}

impl PlaylistState {
    /// Create a new playlist state with normalized URL.
    pub fn new(
        url: &str,
        playlist_title: impl Into<String>,
        selected_audio: Option<String>,
        selected_sub_langs: Vec<String>,
        strict_subs: bool,
    ) -> Self {
        let now = Utc::now();
        Self {
            version: STATE_VERSION,
            source_url: extract_url_path(url),
            playlist_title: playlist_title.into(),
            selected_audio,
            selected_sub_langs,
            strict_subs,
            failed_episodes: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

impl SessionState {
    /// Load session state from a file, validating version and URL.
    ///
    /// Returns `None` silently on any failure: missing file, corrupt JSON,
    /// version mismatch, or URL mismatch. This ensures a fresh start when
    /// state is invalid without blocking the user.
    pub async fn load(path: &Path, url: &str) -> Option<Self> {
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(_) => return None,
        };

        let state: Self = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(e) => {
                debug!(
                    path = path.display().to_string().as_str();
                    "Corrupt session state, starting fresh: {e}"
                );
                return None;
            }
        };

        // Validate version
        let version = match &state {
            Self::SingleVideo(s) => s.version,
            Self::Playlist(s) => s.version,
        };
        if version != STATE_VERSION {
            debug!(
                found = version, expected = STATE_VERSION;
                "Session state version mismatch, starting fresh"
            );
            return None;
        }

        // Validate URL (CDN-tolerant path comparison)
        let stored_url = match &state {
            Self::SingleVideo(s) => &s.source_url,
            Self::Playlist(s) => &s.source_url,
        };
        let current_path = extract_url_path(url);
        if *stored_url != current_path {
            debug!(
                stored = stored_url.as_str(),
                current = current_path.as_str();
                "Session state URL mismatch, starting fresh"
            );
            return None;
        }

        debug!(
            path = path.display().to_string().as_str();
            "Loaded session state"
        );
        Some(state)
    }

    /// Save session state to a file using atomic write (tmp + rename).
    ///
    /// Errors are logged as warnings but not propagated — saving state
    /// is best-effort and must never block a download.
    pub async fn save(&self, path: &Path) {
        let json = match serde_json::to_string_pretty(self) {
            Ok(j) => j,
            Err(e) => {
                debug!("Failed to serialize session state: {e}");
                return;
            }
        };

        let temp_path = path.with_extension("json.tmp");

        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && !parent.exists()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            debug!("Failed to create directory for session state: {e}");
            return;
        }

        if let Err(e) = tokio::fs::write(&temp_path, json.as_bytes()).await {
            debug!("Failed to write session state temp file: {e}");
            return;
        }

        if let Err(e) = tokio::fs::rename(&temp_path, path).await {
            debug!("Failed to rename session state file: {e}");
            // Clean up temp file on rename failure
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }

        debug!(
            path = path.display().to_string().as_str();
            "Saved session state"
        );
    }

    /// Delete session state file if it exists.
    ///
    /// Errors are logged as warnings but not propagated.
    pub async fn delete(path: &Path) {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {
                debug!(
                    path = path.display().to_string().as_str();
                    "Deleted session state"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Already gone — nothing to do
            }
            Err(e) => {
                debug!("Failed to delete session state at {}: {e}", path.display());
            }
        }
    }
}

/// Compute the state file path for a single video download.
///
/// Pattern: `{output_dir}/{sanitized_title}.rdlp_state.json`
pub(super) fn single_video_state_path(output_dir: &Path, sanitized_title: &str) -> PathBuf {
    output_dir.join(format!("{sanitized_title}.rdlp_state.json"))
}

/// Compute the state file path for a playlist download.
///
/// Pattern: `{playlist_dir}/.rdlp_playlist_state.json`
pub(super) fn playlist_state_path(playlist_dir: &Path) -> PathBuf {
    playlist_dir.join(".rdlp_playlist_state.json")
}
