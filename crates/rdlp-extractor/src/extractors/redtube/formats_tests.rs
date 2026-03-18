//! Tests for RedTube format extraction

use super::*;
use serde_json::Value;

#[test]
fn test_extract_from_sources() {
    let webpage = r#"
        var playerConfig = {
            sources: {"720": "https://example.com/720.mp4", "1080": "https://example.com/1080.mp4"},
            title: "Test Video"
        };
    "#;

    let formats = extract_from_sources(webpage);
    assert_eq!(formats.len(), 2);

    // Check that we got both formats (ID includes format type for uniqueness)
    assert!(formats.iter().any(|f| f.format_id == "720-mp4"));
    assert!(formats.iter().any(|f| f.format_id == "1080-mp4"));

    // Check format_note is set
    assert!(
        formats
            .iter()
            .any(|f| f.format_note == Some("720p".to_string()))
    );
    assert!(
        formats
            .iter()
            .any(|f| f.format_note == Some("1080p".to_string()))
    );
}

#[test]
fn test_build_format_with_numeric_quality() {
    let format = build_format("720", "https://example.com/video.mp4".to_string(), "mp4");

    assert_eq!(format.format_id, "720-mp4");
    assert_eq!(format.height, Some(720));
    assert_eq!(format.width, Some(1280));
    assert_eq!(format.format_note, Some("720p".to_string()));
    assert_eq!(format.vcodec, Some("h264".to_string()));
}

#[test]
fn test_build_format_with_string_quality() {
    let format = build_format("hd", "https://example.com/video.mp4".to_string(), "mp4");

    assert_eq!(format.format_id, "hd-mp4");
    assert_eq!(format.height, None);
    assert_eq!(format.format_note, Some("hd".to_string()));
}

#[test]
fn test_parse_quality_from_string() {
    let item: Value = serde_json::json!({"quality": "720"});
    assert_eq!(parse_quality(&item), "720");
}

#[test]
fn test_parse_quality_from_number() {
    let item: Value = serde_json::json!({"quality": 1080});
    assert_eq!(parse_quality(&item), "1080");
}

#[test]
fn test_parse_quality_missing() {
    let item: Value = serde_json::json!({"other": "field"});
    assert_eq!(parse_quality(&item), "unknown");
}

#[test]
fn test_get_format_type_from_url() {
    assert_eq!(
        get_format_type_from_url("https://example.com/video.mp4"),
        "mp4"
    );
    assert_eq!(
        get_format_type_from_url("https://example.com/playlist.m3u8"),
        "hls"
    );
    assert_eq!(
        get_format_type_from_url("https://example.com/video.webm"),
        "webm"
    );
    assert_eq!(
        get_format_type_from_url("https://example.com/video"),
        "unknown"
    );
}

#[test]
fn test_extract_bitrate_from_url() {
    // Standard RedTube URL pattern
    assert_eq!(
        extract_bitrate_from_url("https://example.com/videos/1080P_4000K_378558032.mp4"),
        Some(4000.0)
    );
    assert_eq!(
        extract_bitrate_from_url("https://example.com/videos/720P_2000K_123456.mp4"),
        Some(2000.0)
    );
    // Lowercase k
    assert_eq!(
        extract_bitrate_from_url("https://example.com/videos/480P_1500k_789.mp4"),
        Some(1500.0)
    );
    // No bitrate in URL
    assert_eq!(
        extract_bitrate_from_url("https://example.com/video.mp4"),
        None
    );
}

#[test]
fn test_build_format_extracts_bitrate() {
    let format = build_format(
        "1080",
        "https://example.com/1080P_4000K_12345.mp4".to_string(),
        "mp4",
    );

    assert_eq!(format.tbr, Some(4000.0));
    // vbr and abr not set - we only know total bitrate from URL
    assert_eq!(format.vbr, None);
    assert_eq!(format.abr, None);
}

// ====================================================================
// API video info (getVideoById) parsing tests
// ====================================================================

/// Sample API response matching the `getVideoById` endpoint shape.
fn sample_api_video_response() -> &'static str {
    r#"{
        "video": {
            "video_id": "123456",
            "title": "Test API Video",
            "url": "https://www.redtube.com/123456",
            "duration": "15:30",
            "publish_date": "2024-03-10",
            "views": 42000,
            "tags": [
                {"tag_name": "amateur"},
                {"tag_name": "hd"}
            ],
            "thumbs": [
                {
                    "size": "small",
                    "width": "130",
                    "height": "97",
                    "src": "https://thumb-small.jpg"
                },
                {
                    "size": "medium",
                    "width": "320",
                    "height": "240",
                    "src": "https://thumb-medium.jpg"
                },
                {
                    "size": "big",
                    "width": "640",
                    "height": "480",
                    "src": "https://thumb-big.jpg"
                }
            ]
        }
    }"#
}

#[test]
fn test_parse_api_video_response_full() {
    let meta = parse_api_video_response(sample_api_video_response()).unwrap();
    assert_eq!(meta.title, "Test API Video");
    assert_eq!(meta.duration, Some(930.0)); // 15*60 + 30
    assert_eq!(meta.view_count, Some(42000));
    assert_eq!(meta.upload_date, Some("2024-03-10".to_string()));
    assert_eq!(
        meta.tags,
        Some(vec!["amateur".to_string(), "hd".to_string()])
    );
}

#[test]
fn test_parse_api_video_response_thumbnails() {
    let meta = parse_api_video_response(sample_api_video_response()).unwrap();
    let thumbs = meta.thumbnails.as_ref().unwrap();
    assert_eq!(thumbs.len(), 3);

    // Primary thumbnail should be the largest (640px wide)
    assert_eq!(meta.thumbnail, Some("https://thumb-big.jpg".to_string()));

    // Check individual thumbnail entries
    let small = thumbs.iter().find(|t| t.id == Some("small".to_string()));
    assert!(small.is_some());
    assert_eq!(small.unwrap().width, Some(130));
    assert_eq!(small.unwrap().height, Some(97));
}

#[test]
fn test_parse_api_video_response_minimal() {
    let json = r#"{
        "video": {
            "video_id": "999",
            "title": "Minimal Video",
            "tags": [],
            "thumbs": []
        }
    }"#;
    let meta = parse_api_video_response(json).unwrap();
    assert_eq!(meta.title, "Minimal Video");
    assert_eq!(meta.duration, None);
    assert_eq!(meta.view_count, None);
    assert_eq!(meta.upload_date, None);
    assert_eq!(meta.thumbnail, None);
    assert_eq!(meta.thumbnails, None);
    assert_eq!(meta.tags, None); // empty vec becomes None
}

#[test]
fn test_parse_api_video_response_string_views() {
    let json = r#"{
        "video": {
            "video_id": "111",
            "title": "String Views",
            "views": "1,234,567",
            "tags": [],
            "thumbs": []
        }
    }"#;
    let meta = parse_api_video_response(json).unwrap();
    assert_eq!(meta.view_count, Some(1234567));
}

#[test]
fn test_parse_api_video_response_invalid_json() {
    let result = parse_api_video_response("not json");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Failed to parse"));
}

#[test]
fn test_parse_api_video_response_wrong_structure() {
    // Valid JSON but wrong structure (missing video wrapper)
    let json = r#"{"error": "Video not found"}"#;
    let result = parse_api_video_response(json);
    assert!(result.is_err());
}

#[test]
fn test_parse_api_video_response_hms_duration() {
    let json = r#"{
        "video": {
            "video_id": "222",
            "title": "Long Video",
            "duration": "1:30:45",
            "tags": [],
            "thumbs": []
        }
    }"#;
    let meta = parse_api_video_response(json).unwrap();
    // 1*3600 + 30*60 + 45 = 5445
    assert_eq!(meta.duration, Some(5445.0));
}

#[test]
fn test_parse_api_video_response_thumb_without_dimensions() {
    let json = r#"{
        "video": {
            "video_id": "333",
            "title": "No Dimensions",
            "tags": [],
            "thumbs": [
                {
                    "size": "default",
                    "src": "https://thumb-default.jpg"
                }
            ]
        }
    }"#;
    let meta = parse_api_video_response(json).unwrap();
    let thumbs = meta.thumbnails.as_ref().unwrap();
    assert_eq!(thumbs.len(), 1);
    assert_eq!(thumbs[0].width, None);
    assert_eq!(thumbs[0].height, None);
    // Still selected as primary since it's the only one
    assert_eq!(
        meta.thumbnail,
        Some("https://thumb-default.jpg".to_string())
    );
}

#[test]
fn test_parse_api_video_response_thumb_without_src() {
    let json = r#"{
        "video": {
            "video_id": "444",
            "title": "No Src",
            "tags": [],
            "thumbs": [
                {
                    "size": "empty",
                    "width": "100",
                    "height": "75"
                }
            ]
        }
    }"#;
    let meta = parse_api_video_response(json).unwrap();
    // Thumb without src is skipped
    assert_eq!(meta.thumbnails, None);
    assert_eq!(meta.thumbnail, None);
}
