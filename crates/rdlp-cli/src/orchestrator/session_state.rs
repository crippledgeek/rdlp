//! Session state persistence for resuming interrupted downloads
//!
//! Saves user selections (format, subtitles, audio type) to a JSON file
//! so interrupted downloads can resume without re-prompting. Download
//! progress (HLS segments, HTTP byte offsets) is handled separately by
//! the downloader crate.

use chrono::{DateTime, Utc};
use log::{debug, warn};
use rdlp_security::extract_url_path;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Current schema version. Bumped on breaking changes to force fresh state.
const STATE_VERSION: u32 = 1;

/// Session state persisted to disk for resume support.
///
/// Uses a serde-tagged enum so the JSON `state_type` field determines
/// which variant to deserialize into.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state_type")]
pub(crate) enum SessionState {
    /// State for a single video download
    #[serde(rename = "single_video")]
    SingleVideo(SingleVideoState),
    /// State for a playlist download
    #[serde(rename = "playlist")]
    Playlist(PlaylistState),
}

/// Persisted selections for a single video download.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct SingleVideoState {
    /// Schema version for forward compatibility
    pub version: u32,
    /// URL path (no host/query) for CDN-tolerant matching
    pub source_url: String,
    /// Video title at time of selection
    pub title: String,
    /// Selected format identifier (matched against available formats on resume)
    pub format_id: String,
    /// Selected subtitle language codes (empty if none)
    pub subtitle_langs: Vec<String>,
    /// When this state was first created
    pub created_at: DateTime<Utc>,
    /// When this state was last updated
    pub updated_at: DateTime<Utc>,
}

/// Persisted selections for a playlist download.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct PlaylistState {
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
pub(crate) struct FailedEpisode {
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
    ) -> Self {
        let now = Utc::now();
        Self {
            version: STATE_VERSION,
            source_url: extract_url_path(url),
            title: title.into(),
            format_id: format_id.into(),
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
    ) -> Self {
        let now = Utc::now();
        Self {
            version: STATE_VERSION,
            source_url: extract_url_path(url),
            playlist_title: playlist_title.into(),
            selected_audio,
            selected_sub_langs,
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
                warn!("Failed to serialize session state: {e}");
                return;
            }
        };

        let temp_path = path.with_extension("json.tmp");

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    warn!("Failed to create directory for session state: {e}");
                    return;
                }
            }
        }

        if let Err(e) = tokio::fs::write(&temp_path, json.as_bytes()).await {
            warn!("Failed to write session state temp file: {e}");
            return;
        }

        if let Err(e) = tokio::fs::rename(&temp_path, path).await {
            warn!("Failed to rename session state file: {e}");
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
                warn!("Failed to delete session state at {}: {e}", path.display());
            }
        }
    }
}

/// Compute the state file path for a single video download.
///
/// Pattern: `{output_dir}/{sanitized_title}.rdlp_state.json`
pub(crate) fn single_video_state_path(output_dir: &Path, sanitized_title: &str) -> PathBuf {
    output_dir.join(format!("{sanitized_title}.rdlp_state.json"))
}

/// Compute the state file path for a playlist download.
///
/// Pattern: `{playlist_dir}/.rdlp_playlist_state.json`
pub(crate) fn playlist_state_path(playlist_dir: &Path) -> PathBuf {
    playlist_dir.join(".rdlp_playlist_state.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_single_video_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp_state.json");
        let url = "https://cdn1.example.com/watch/video-123?token=abc";

        let state = SessionState::SingleVideo(SingleVideoState::new(
            url,
            "Test Video",
            "720p-hls",
            vec!["en".to_string(), "ja".to_string()],
        ));

        state.save(&path).await;

        // Load with same URL (different CDN host)
        let loaded =
            SessionState::load(&path, "https://cdn2.example.com/watch/video-123?token=xyz").await;

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        match &loaded {
            SessionState::SingleVideo(s) => {
                assert_eq!(s.format_id, "720p-hls");
                assert_eq!(s.subtitle_langs, vec!["en", "ja"]);
                assert_eq!(s.title, "Test Video");
            }
            _ => panic!("Expected SingleVideo variant"),
        }
    }

    #[tokio::test]
    async fn test_playlist_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".rdlp_playlist_state.json");
        let url = "https://example.com/watch/show-1234";

        let state = SessionState::Playlist(PlaylistState::new(
            url,
            "Season 1",
            Some("SUB".to_string()),
            vec!["en".to_string()],
        ));

        state.save(&path).await;

        let loaded = SessionState::load(&path, url).await;
        assert!(loaded.is_some());
        match loaded.unwrap() {
            SessionState::Playlist(s) => {
                assert_eq!(s.playlist_title, "Season 1");
                assert_eq!(s.selected_audio, Some("SUB".to_string()));
                assert_eq!(s.selected_sub_langs, vec!["en"]);
            }
            _ => panic!("Expected Playlist variant"),
        }
    }

    #[tokio::test]
    async fn test_url_mismatch_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp_state.json");

        let state = SessionState::SingleVideo(SingleVideoState::new(
            "https://example.com/watch/video-123",
            "Test",
            "720p",
            vec![],
        ));
        state.save(&path).await;

        // Different path → should return None
        let loaded = SessionState::load(&path, "https://example.com/watch/different-video").await;
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_version_mismatch_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp_state.json");

        // Write state with a future version
        let json = r#"{
            "state_type": "single_video",
            "version": 999,
            "source_url": "/watch/video-123",
            "title": "Test",
            "format_id": "720p",
            "subtitle_langs": [],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#;
        tokio::fs::write(&path, json).await.unwrap();

        let loaded = SessionState::load(&path, "https://example.com/watch/video-123").await;
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_corrupt_json_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp_state.json");

        tokio::fs::write(&path, "not valid json {{{").await.unwrap();

        let loaded = SessionState::load(&path, "https://example.com/watch/video-123").await;
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_missing_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.rdlp_state.json");

        let loaded = SessionState::load(&path, "https://example.com/watch/video-123").await;
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp_state.json");

        let state = SessionState::SingleVideo(SingleVideoState::new(
            "https://example.com/watch/video-123",
            "Test",
            "720p",
            vec![],
        ));
        state.save(&path).await;
        assert!(path.exists());

        SessionState::delete(&path).await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.rdlp_state.json");

        // Should not panic or error
        SessionState::delete(&path).await;
    }

    #[test]
    fn test_single_video_state_path() {
        let dir = Path::new("/output");
        let path = single_video_state_path(dir, "My Video Title");
        assert_eq!(
            path,
            PathBuf::from("/output/My Video Title.rdlp_state.json")
        );
    }

    #[test]
    fn test_playlist_state_path() {
        let dir = Path::new("/playlists/Season 1");
        let path = playlist_state_path(dir);
        assert_eq!(
            path,
            PathBuf::from("/playlists/Season 1/.rdlp_playlist_state.json")
        );
    }

    #[tokio::test]
    async fn test_cdn_tolerant_url_matching() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rdlp_state.json");

        // Save with one CDN host
        let state = SessionState::SingleVideo(SingleVideoState::new(
            "https://s1.cdn.example.com/hls/video/master.m3u8?token=abc",
            "Test",
            "720p",
            vec![],
        ));
        state.save(&path).await;

        // Load with different CDN host but same path
        let loaded = SessionState::load(
            &path,
            "https://s9.cdn.example.com/hls/video/master.m3u8?token=xyz",
        )
        .await;
        assert!(loaded.is_some());

        // Load with different path → should fail
        let loaded =
            SessionState::load(&path, "https://s1.cdn.example.com/hls/other/master.m3u8").await;
        assert!(loaded.is_none());
    }
}
