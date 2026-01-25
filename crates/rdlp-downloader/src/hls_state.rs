//! HLS download state persistence for resume support
//!
//! This module provides state tracking for HLS downloads, enabling
//! resumption of interrupted downloads by tracking which segments
//! have been successfully downloaded.
//!
//! # State File Format
//!
//! State is stored as JSON in `{output_path}.hls_state.json`:
//!
//! ```json
//! {
//!   "version": 1,
//!   "playlist_url": "https://example.com/playlist.m3u8",
//!   "segment_count": 754,
//!   "completed_segments": [0, 1, 2, ...],
//!   "total_bytes_downloaded": 123456789,
//!   "created_at": "2025-01-25T12:00:00Z",
//!   "updated_at": "2025-01-25T12:05:00Z"
//! }
//! ```

use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use rdlp_security::extract_url_path;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Current state file format version
const STATE_VERSION: u32 = 1;

/// HLS download state for resume support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlsDownloadState {
    /// State format version (for future migrations)
    pub version: u32,

    /// Original playlist URL
    pub playlist_url: String,

    /// Total number of segments in the playlist
    pub segment_count: usize,

    /// Indices of completed segments
    pub completed_segments: HashSet<usize>,

    /// Total bytes downloaded across all completed segments
    pub total_bytes_downloaded: u64,

    /// When the download was started
    pub created_at: DateTime<Utc>,

    /// When the state was last updated
    pub updated_at: DateTime<Utc>,
}

impl HlsDownloadState {
    /// Create a new state for a fresh download
    ///
    /// The playlist URL is normalized (query parameters stripped) to handle
    /// dynamic tokens that change on each request.
    pub fn new(playlist_url: String, segment_count: usize) -> Self {
        let now = Utc::now();
        Self {
            version: STATE_VERSION,
            playlist_url: extract_url_path(&playlist_url),
            segment_count,
            completed_segments: HashSet::new(),
            total_bytes_downloaded: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Get the state file path for a given output path
    pub fn state_file_path(output_path: &Path) -> PathBuf {
        let mut state_path = output_path.to_path_buf();
        let filename = state_path
            .file_name()
            .map(|f| format!("{}.hls_state.json", f.to_string_lossy()))
            .unwrap_or_else(|| "download.hls_state.json".to_string());
        state_path.set_file_name(filename);
        state_path
    }

    /// Load state from file if it exists and is valid
    ///
    /// Returns `None` if:
    /// - State file doesn't exist
    /// - State file is corrupted
    /// - Playlist URL doesn't match (different download)
    /// - Segment count doesn't match (playlist changed)
    pub async fn load(
        output_path: &Path,
        playlist_url: &str,
        segment_count: usize,
    ) -> Option<Self> {
        let state_path = Self::state_file_path(output_path);

        if !state_path.exists() {
            debug!("No HLS state file found at {}", state_path.display());
            return None;
        }

        // Read and parse state file
        let state: Self = match File::open(&state_path).await {
            Ok(mut file) => {
                let mut contents = String::new();
                if let Err(e) = file.read_to_string(&mut contents).await {
                    warn!("Failed to read HLS state file: {e}");
                    return None;
                }

                match serde_json::from_str(&contents) {
                    Ok(state) => state,
                    Err(e) => {
                        warn!("Failed to parse HLS state file: {e}");
                        return None;
                    }
                }
            }
            Err(e) => {
                warn!("Failed to open HLS state file: {e}");
                return None;
            }
        };

        // Validate state
        if state.version != STATE_VERSION {
            warn!(
                "HLS state version mismatch (expected {}, got {}), starting fresh",
                STATE_VERSION, state.version
            );
            return None;
        }

        // Compare normalized URLs to handle dynamic tokens
        let normalized_url = extract_url_path(playlist_url);
        if state.playlist_url != normalized_url {
            warn!("Playlist URL changed, starting fresh");
            debug!("  Old: {}", state.playlist_url);
            debug!("  New: {normalized_url}");
            return None;
        }

        if state.segment_count != segment_count {
            warn!(
                "Segment count changed ({} -> {}), starting fresh",
                state.segment_count, segment_count
            );
            return None;
        }

        let completed = state.completed_segments.len();
        let mb = state.total_bytes_downloaded as f64 / (1024.0 * 1024.0);
        info!("Resuming HLS download: {completed}/{segment_count} segments completed ({mb:.1} MB)");

        Some(state)
    }

    /// Save state to file
    ///
    /// Uses atomic write pattern: write to temp file, then rename
    pub async fn save(&self, output_path: &Path) -> Result<(), std::io::Error> {
        let state_path = Self::state_file_path(output_path);
        let temp_path = state_path.with_extension("json.tmp");

        // Serialize to JSON
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;

        // Write to temp file
        let mut file = File::create(&temp_path).await?;
        file.write_all(json.as_bytes()).await?;
        file.sync_all().await?; // Ensure data is on disk

        // Atomic rename
        tokio::fs::rename(&temp_path, &state_path).await?;

        debug!(
            "Saved HLS state: {}/{} segments",
            self.completed_segments.len(),
            self.segment_count
        );
        Ok(())
    }

    /// Mark a segment as completed
    pub fn mark_completed(&mut self, segment_index: usize, bytes: u64) {
        self.completed_segments.insert(segment_index);
        self.total_bytes_downloaded += bytes;
        self.updated_at = Utc::now();
    }

    /// Check if a segment is already completed
    pub fn is_completed(&self, segment_index: usize) -> bool {
        self.completed_segments.contains(&segment_index)
    }

    /// Get list of segments that still need to be downloaded
    pub fn pending_segments(&self) -> Vec<usize> {
        (0..self.segment_count)
            .filter(|idx| !self.completed_segments.contains(idx))
            .collect()
    }

    /// Get progress as a fraction (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        if self.segment_count == 0 {
            return 1.0;
        }
        self.completed_segments.len() as f64 / self.segment_count as f64
    }

    /// Delete the state file
    pub async fn delete(output_path: &Path) -> Result<(), std::io::Error> {
        let state_path = Self::state_file_path(output_path);
        if state_path.exists() {
            tokio::fs::remove_file(&state_path).await?;
            debug!("Deleted HLS state file: {}", state_path.display());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_file_path() {
        let output = Path::new("/tmp/video.mp4");
        let state_path = HlsDownloadState::state_file_path(output);
        assert_eq!(state_path, Path::new("/tmp/video.mp4.hls_state.json"));
    }

    #[test]
    fn test_new_state() {
        let state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 100);
        assert_eq!(state.version, STATE_VERSION);
        assert_eq!(state.segment_count, 100);
        assert!(state.completed_segments.is_empty());
        assert_eq!(state.total_bytes_downloaded, 0);
    }

    #[test]
    fn test_mark_completed() {
        let mut state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 100);

        state.mark_completed(5, 1024);
        state.mark_completed(10, 2048);

        assert!(state.is_completed(5));
        assert!(state.is_completed(10));
        assert!(!state.is_completed(0));
        assert_eq!(state.total_bytes_downloaded, 3072);
    }

    #[test]
    fn test_pending_segments() {
        let mut state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 5);

        state.mark_completed(1, 100);
        state.mark_completed(3, 100);

        let pending = state.pending_segments();
        assert_eq!(pending, vec![0, 2, 4]);
    }

    #[test]
    fn test_progress() {
        let mut state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 10);

        assert_eq!(state.progress(), 0.0);

        state.mark_completed(0, 100);
        state.mark_completed(1, 100);
        state.mark_completed(2, 100);

        assert!((state.progress() - 0.3).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("video.mp4");

        // Create and save state
        let mut state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 100);
        state.mark_completed(0, 1000);
        state.mark_completed(5, 2000);
        state.save(&output_path).await.unwrap();

        // Load state
        let loaded = HlsDownloadState::load(&output_path, "https://example.com/playlist.m3u8", 100)
            .await
            .unwrap();

        assert_eq!(loaded.completed_segments.len(), 2);
        assert!(loaded.is_completed(0));
        assert!(loaded.is_completed(5));
        assert_eq!(loaded.total_bytes_downloaded, 3000);
    }

    #[tokio::test]
    async fn test_load_mismatched_url() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("video.mp4");

        // Create and save state
        let state = HlsDownloadState::new("https://example.com/old.m3u8".to_string(), 100);
        state.save(&output_path).await.unwrap();

        // Load with different URL should fail
        let loaded =
            HlsDownloadState::load(&output_path, "https://example.com/new.m3u8", 100).await;

        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_load_mismatched_segment_count() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("video.mp4");

        // Create and save state
        let state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 100);
        state.save(&output_path).await.unwrap();

        // Load with different segment count should fail
        let loaded = HlsDownloadState::load(
            &output_path,
            "https://example.com/playlist.m3u8",
            200, // Different count
        )
        .await;

        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("video.mp4");

        // Create and save state
        let state = HlsDownloadState::new("https://example.com/playlist.m3u8".to_string(), 100);
        state.save(&output_path).await.unwrap();

        let state_path = HlsDownloadState::state_file_path(&output_path);
        assert!(state_path.exists());

        // Delete
        HlsDownloadState::delete(&output_path).await.unwrap();
        assert!(!state_path.exists());
    }

    #[test]
    fn test_extract_url_path() {
        // Basic URL - extracts path only
        assert_eq!(
            extract_url_path("https://example.com/video/master.m3u8"),
            "/video/master.m3u8"
        );

        // URL with query parameters - extracts path without query
        assert_eq!(
            extract_url_path(
                "https://cdn.example.com/hls/master.m3u8?token=abc123&expires=1706140800"
            ),
            "/hls/master.m3u8"
        );

        // Different CDN hostnames with same path should match
        let url1 = "https://ev-h-ph.cdn.com/video.m3u8?token=abc";
        let url2 = "https://ev-a-ph.cdn.com/video.m3u8?token=xyz";
        assert_eq!(extract_url_path(url1), extract_url_path(url2));

        // URL with port - path extracted regardless
        assert_eq!(
            extract_url_path("https://cdn.example.com:8080/master.m3u8?token=abc"),
            "/master.m3u8"
        );

        // URL with fragment - path only
        assert_eq!(
            extract_url_path("https://example.com/video.m3u8#section"),
            "/video.m3u8"
        );
    }

    #[tokio::test]
    async fn test_resume_with_different_cdn_hostname() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("video.mp4");

        // Create state with URL from one CDN edge server
        let mut state = HlsDownloadState::new(
            "https://ev-h-ph.rdtcdn.com/hls/videos/123/master.m3u8?token=abc123".to_string(),
            100,
        );
        state.mark_completed(0, 1000);
        state.mark_completed(1, 1000);
        state.save(&output_path).await.unwrap();

        // Load with DIFFERENT CDN hostname but same path - should SUCCEED
        let loaded = HlsDownloadState::load(
            &output_path,
            "https://ev-a-ph.rdtcdn.com/hls/videos/123/master.m3u8?token=xyz789",
            100,
        )
        .await;

        assert!(
            loaded.is_some(),
            "Should resume with different CDN hostname"
        );
        let loaded = loaded.unwrap();
        assert_eq!(loaded.completed_segments.len(), 2);
    }
}
