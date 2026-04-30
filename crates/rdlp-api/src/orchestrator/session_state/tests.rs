//! Tests for session state persistence
#![allow(
    clippy::indexing_slicing,
    clippy::match_wildcard_for_single_variants,
    unused_variables
)]

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
        None,
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
        false,
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
        None,
    ));
    state.save(&path).await;

    // Different path -> should return None
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
        None,
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
async fn test_merge_plan_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.rdlp_state.json");
    let url = "https://cdn1.example.com/watch/video-123?token=abc";

    let state = SessionState::SingleVideo(SingleVideoState::new(
        url,
        "Test Video",
        "v1080",
        vec!["en".to_string()],
        Some("a256".to_string()),
    ));

    state.save(&path).await;

    let loaded =
        SessionState::load(&path, "https://cdn2.example.com/watch/video-123?token=xyz").await;
    assert!(loaded.is_some());
    match loaded.unwrap() {
        SessionState::SingleVideo(s) => {
            assert_eq!(s.format_id, "v1080");
            assert_eq!(s.audio_format_id, Some("a256".to_string()));
            assert_eq!(s.subtitle_langs, vec!["en"]);
        }
        _ => panic!("Expected SingleVideo variant"),
    }
}

#[tokio::test]
async fn test_single_plan_has_no_audio_format_id() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.rdlp_state.json");
    let url = "https://example.com/watch/video-123";

    let state = SessionState::SingleVideo(SingleVideoState::new(
        url,
        "Test Video",
        "c720",
        vec![],
        None,
    ));

    state.save(&path).await;

    let loaded = SessionState::load(&path, url).await;
    assert!(loaded.is_some());
    match loaded.unwrap() {
        SessionState::SingleVideo(s) => {
            assert_eq!(s.format_id, "c720");
            assert!(s.audio_format_id.is_none());
        }
        _ => panic!("Expected SingleVideo variant"),
    }
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
        None,
    ));
    state.save(&path).await;

    // Load with different CDN host but same path
    let loaded = SessionState::load(
        &path,
        "https://s9.cdn.example.com/hls/video/master.m3u8?token=xyz",
    )
    .await;
    assert!(loaded.is_some());

    // Load with different path -> should fail
    let loaded =
        SessionState::load(&path, "https://s1.cdn.example.com/hls/other/master.m3u8").await;
    assert!(loaded.is_none());
}
