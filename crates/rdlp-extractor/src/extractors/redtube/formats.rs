//! Format extraction for RedTube
//!
//! Extracts video formats from JavaScript sources, mediaDefinition arrays,
//! and the `getVideoById` JSON API response.

use log::{debug, warn};
use rdlp_core::{ExtractionContext, Format, RdlpError, Result, Thumbnail};
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::sync::LazyLock;

use crate::base::common::BaseExtractor;
use crate::utils::{extract_extension_from_url, make_absolute_url};

use super::patterns::{MEDIA_DEF_PATTERN, SOURCES_PATTERN};
use super::search::parse_duration_string;

// ============================================================================
// API Video Info response types (getVideoById)
// ============================================================================

/// Top-level API response from `redtube.Videos.getVideoById`.
///
/// The response wraps the video in a `video` object inside a one-element array.
/// Example: `{"video": {"video_id": "123", "title": "...", ...}}`
#[derive(Debug, Deserialize)]
pub(crate) struct ApiVideoInfoResponse {
    /// The nested video object.
    pub video: ApiVideoInfo,
}

/// Video info returned by the `getVideoById` API endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiVideoInfo {
    /// Video title.
    pub title: String,
    /// Duration string in "MM:SS" or "H:MM:SS" format.
    #[serde(default)]
    pub duration: Option<String>,
    /// Publish date string.
    #[serde(default)]
    pub publish_date: Option<String>,
    /// View count (may be number or string).
    #[serde(default, deserialize_with = "super::search::deserialize_views")]
    pub views: Option<String>,
    /// Tag list.
    #[serde(default)]
    pub tags: Vec<ApiVideoTag>,
    /// Thumbnail list (requested via `thumbsize=all`).
    #[serde(default)]
    pub thumbs: Vec<ApiThumb>,
}

/// A single tag entry from the video info API.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiVideoTag {
    /// Tag display name.
    pub tag_name: String,
}

/// A single thumbnail entry from the video info API.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiThumb {
    /// Size descriptor (e.g. "small", "medium", "big", "all").
    pub size: String,
    /// Width in pixels as a string.
    #[serde(default)]
    pub width: Option<String>,
    /// Height in pixels as a string.
    #[serde(default)]
    pub height: Option<String>,
    /// Thumbnail image URL.
    #[serde(default)]
    pub src: Option<String>,
}

/// Parsed metadata extracted from the `getVideoById` API response.
///
/// Contains the fields needed to populate `InfoDict` metadata.
#[derive(Debug)]
pub(crate) struct ApiVideoMetadata {
    /// Video title.
    pub title: String,
    /// Duration in seconds.
    pub duration: Option<f64>,
    /// Primary thumbnail URL (largest available).
    pub thumbnail: Option<String>,
    /// All available thumbnails with dimensions.
    pub thumbnails: Option<Vec<Thumbnail>>,
    /// Tag names.
    pub tags: Option<Vec<String>>,
    /// View count.
    pub view_count: Option<u64>,
    /// Publish date string.
    pub upload_date: Option<String>,
}

/// Parse the `getVideoById` API JSON response into metadata.
///
/// # Arguments
/// * `json` - Raw JSON response body from the API.
///
/// # Returns
/// Parsed `ApiVideoMetadata` or an error if JSON parsing fails.
pub(crate) fn parse_api_video_response(json: &str) -> Result<ApiVideoMetadata> {
    let response: ApiVideoInfoResponse = serde_json::from_str(json).map_err(|e| {
        RdlpError::Extraction(format!("Failed to parse RedTube video API response: {e}"))
    })?;

    let video = response.video;

    let duration = video
        .duration
        .as_deref()
        .and_then(parse_duration_string)
        .map(|s| s as f64);

    let view_count = video
        .views
        .as_deref()
        .and_then(super::search::parse_view_count);

    let tags = if video.tags.is_empty() {
        None
    } else {
        Some(video.tags.into_iter().map(|t| t.tag_name).collect())
    };

    // Build thumbnail list — pick the largest as primary
    let mut thumbnails = Vec::new();
    let mut best_thumb: Option<String> = None;
    let mut best_width: u32 = 0;

    for thumb in &video.thumbs {
        if let Some(src) = &thumb.src {
            let width = thumb.width.as_deref().and_then(|w| w.parse::<u32>().ok());
            let height = thumb.height.as_deref().and_then(|h| h.parse::<u32>().ok());

            thumbnails.push(Thumbnail {
                url: src.clone(),
                id: Some(thumb.size.clone()),
                width,
                height,
                preference: None,
            });

            let w = width.unwrap_or(0);
            if w > best_width {
                best_width = w;
                best_thumb = Some(src.clone());
            }
        }
    }

    // If no thumb was selected by width, use the first available
    if best_thumb.is_none() && !thumbnails.is_empty() {
        best_thumb = Some(thumbnails[0].url.clone());
    }

    Ok(ApiVideoMetadata {
        title: video.title,
        duration,
        thumbnail: best_thumb,
        thumbnails: if thumbnails.is_empty() {
            None
        } else {
            Some(thumbnails)
        },
        tags,
        view_count,
        upload_date: video.publish_date,
    })
}

/// Extract quality string from JSON value (handles both string and number types)
pub fn parse_quality(item: &Value) -> String {
    item.get("quality")
        .and_then(|q| {
            q.as_str()
                .map(String::from)
                .or_else(|| q.as_i64().map(|i| i.to_string()))
        })
        .unwrap_or_else(|| "unknown".into())
}

/// Extract bitrate from URL pattern like "1080P_4000K_378558032.mp4"
/// Returns bitrate in kbps if found
fn extract_bitrate_from_url(url: &str) -> Option<f64> {
    // Pattern: digits followed by K (case insensitive) before file extension
    // Example: 4000K, 2000K, 1500K
    static BITRATE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(\d+)[Kk]_\d+\.[a-zA-Z0-9]+$").expect("Valid bitrate pattern")
    });

    BITRATE_PATTERN
        .captures(url)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

/// Build Format from quality string and URL using BaseExtractor utilities.
///
/// Delegates to `BaseExtractor::build_format()` for shared logic
/// (height/width, format_note, codec defaults, quality score),
/// then applies RedTube-specific fields (container, bitrate, fallback format_note).
pub fn build_format(quality_str: &str, url: String, format_type: &str) -> Format {
    let height = BaseExtractor::parse_quality_height(quality_str);
    let bitrate = extract_bitrate_from_url(&url);

    let mut format =
        BaseExtractor::build_format(quality_str.to_owned(), url, format_type.to_owned(), height);

    // RedTube-specific: set format_note to raw quality string when height is unknown
    if height.is_none() {
        format.format_note = Some(quality_str.to_owned());
    }

    // RedTube-specific: container field
    format.container = Some(format_type.to_owned());

    // RedTube-specific: extract bitrate from URL (e.g., "4000K" = 4000 kbps)
    if let Some(bitrate) = bitrate {
        format.tbr = Some(bitrate);
    }

    format
}

/// Extract format type from URL using utility function
fn get_format_type_from_url(url_str: &str) -> &'static str {
    match extract_extension_from_url(url_str).as_deref() {
        Some("mp4") => "mp4",
        Some("m3u8") => "hls",
        Some("webm") => "webm",
        Some("mkv") => "mkv",
        _ => "unknown",
    }
}

/// Extract video formats from JavaScript sources object
///
/// Looks for: sources: {"720": "https://...", "1080": "https://...", ...}
pub fn extract_from_sources(webpage: &str) -> Vec<Format> {
    let mut formats = Vec::new();

    if let Some(caps) = SOURCES_PATTERN.captures(webpage) {
        if let Some(sources_str) = caps.get(1) {
            debug!(sources:? = sources_str.as_str(); "[RedTube] Found sources object");

            // Try to parse as JSON
            match serde_json::from_str::<Value>(sources_str.as_str()) {
                Ok(sources) => {
                    if let Some(obj) = sources.as_object() {
                        for (quality, url) in obj {
                            if let Some(url_str) = url.as_str() {
                                let format_type = get_format_type_from_url(url_str);
                                let format =
                                    build_format(quality, url_str.to_string(), format_type);

                                debug!(
                                    format_id:? = format.format_id,
                                    note:? = format.format_note.as_deref().unwrap_or("unknown");
                                    "[RedTube] Extracted format"
                                );

                                formats.push(format);
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!(
                        line = e.line(),
                        column = e.column();
                        "[RedTube] Failed to parse sources JSON: {e}"
                    );
                }
            }
        }
    }

    formats
}

/// Extract video formats from mediaDefinition array
///
/// Looks for: mediaDefinition: [{videoUrl: "...", format: "...", quality: "..."}, ...]
///
/// Note: If format is "mp4" without quality field, videoUrl is a JSON endpoint
/// that needs to be fetched to get the actual format list
pub async fn extract_from_media_definition(webpage: &str, ctx: &ExtractionContext) -> Vec<Format> {
    let mut formats = Vec::new();

    if let Some(caps) = MEDIA_DEF_PATTERN.captures(webpage) {
        if let Some(media_def_str) = caps.get(1) {
            BaseExtractor::log_if_verbose(
                ctx,
                "RedTube",
                &format!(
                    "Found mediaDefinition array: {}",
                    &media_def_str.as_str().chars().take(200).collect::<String>()
                ),
            );

            // Try to parse as JSON
            match serde_json::from_str::<Value>(media_def_str.as_str()) {
                Ok(media_def) => {
                    let Some(arr) = media_def.as_array() else {
                        BaseExtractor::log_if_verbose(
                            ctx,
                            "RedTube",
                            "mediaDefinition is not an array",
                        );
                        return formats;
                    };
                    BaseExtractor::log_if_verbose(
                        ctx,
                        "RedTube",
                        &format!("Found {} media items", arr.len()),
                    );

                    for (idx, item) in arr.iter().enumerate() {
                        BaseExtractor::log_if_verbose(
                            ctx,
                            "RedTube",
                            &format!("Processing item {idx}: {item:?}"),
                        );

                        if let Some(video_url) = item.get("videoUrl").and_then(|v| v.as_str()) {
                            let format_type =
                                item.get("format").and_then(|v| v.as_str()).unwrap_or("mp4");

                            let has_quality = item.get("quality").is_some();

                            // If format is mp4/hls without quality, fetch JSON to get actual formats
                            if matches!(format_type, "mp4" | "hls") && !has_quality {
                                if let Some(fetched) =
                                    fetch_formats_from_endpoint(video_url, ctx).await
                                {
                                    formats.extend(fetched);
                                }
                            } else {
                                // Has quality field, process directly
                                let quality_str = parse_quality(item);
                                let format =
                                    build_format(&quality_str, video_url.to_string(), format_type);

                                BaseExtractor::log_if_verbose(
                                    ctx,
                                    "RedTube",
                                    &format!(
                                        "Extracted format: {} - {} ({}x{})",
                                        format.format_id,
                                        format.format_note.as_deref().unwrap_or("unknown"),
                                        format.width.unwrap_or(0),
                                        format.height.unwrap_or(0)
                                    ),
                                );

                                formats.push(format);
                            }
                        }
                    }
                }
                Err(e) => {
                    BaseExtractor::log_if_verbose(
                        ctx,
                        "RedTube",
                        &format!(
                            "Failed to parse mediaDefinition JSON at {}:{}: {}",
                            e.line(),
                            e.column(),
                            e
                        ),
                    );
                }
            }
        }
    }

    formats
}

/// Fetch formats from a JSON endpoint (used when mediaDefinition contains an endpoint URL)
async fn fetch_formats_from_endpoint(
    video_url: &str,
    ctx: &ExtractionContext,
) -> Option<Vec<Format>> {
    // Convert relative URL to absolute using utility
    let absolute_url = make_absolute_url("https://www.redtube.com", video_url);

    BaseExtractor::log_if_verbose(
        ctx,
        "RedTube",
        &format!("Fetching format JSON from: {absolute_url}"),
    );

    // Fetch the JSON endpoint
    let response = match ctx.http_client.get(&absolute_url).send().await {
        Ok(r) => r,
        Err(e) => {
            if e.is_timeout() {
                warn!(url:? = absolute_url; "[RedTube] Request timed out");
            } else if e.is_connect() {
                warn!(url:? = absolute_url; "[RedTube] Connection failed: {e}");
            } else {
                warn!("[RedTube] Request failed: {e}");
            }
            return None;
        }
    };

    // Validate HTTP status code
    if !response.status().is_success() {
        BaseExtractor::log_if_verbose(
            ctx,
            "RedTube",
            &format!("HTTP {} for URL: {}", response.status(), absolute_url),
        );
        return None;
    }

    let json_text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            BaseExtractor::log_if_verbose(
                ctx,
                "RedTube",
                &format!("Failed to read response body: {e}"),
            );
            return None;
        }
    };

    BaseExtractor::log_content_if_verbose(ctx, "RedTube", "JSON response", &json_text, 500);

    // Parse JSON array of formats
    let more_media: Value = match serde_json::from_str(&json_text) {
        Ok(v) => v,
        Err(e) => {
            BaseExtractor::log_if_verbose(
                ctx,
                "RedTube",
                &format!(
                    "Failed to parse formats JSON at {}:{}: {}",
                    e.line(),
                    e.column(),
                    e
                ),
            );
            return None;
        }
    };

    let more_arr = more_media.as_array()?;
    let mut formats = Vec::new();

    for media_item in more_arr {
        if let Some(media_url) = media_item.get("videoUrl").and_then(|v| v.as_str()) {
            let quality_str = parse_quality(media_item);
            // Extract format type from JSON (hls, mp4, etc.)
            let format_type = media_item
                .get("format")
                .and_then(|f| f.as_str())
                .unwrap_or("unknown");
            let format = build_format(&quality_str, media_url.to_string(), format_type);

            BaseExtractor::log_if_verbose(
                ctx,
                "RedTube",
                &format!(
                    "Extracted format from JSON: {} - {} ({}x{}) [{}]",
                    format.format_id,
                    format.format_note.as_deref().unwrap_or("unknown"),
                    format.width.unwrap_or(0),
                    format.height.unwrap_or(0),
                    format.ext.to_uppercase()
                ),
            );

            formats.push(format);
        }
    }

    Some(formats)
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // Check that we got both formats
        assert!(formats.iter().any(|f| f.format_id == "720"));
        assert!(formats.iter().any(|f| f.format_id == "1080"));

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

        assert_eq!(format.format_id, "720");
        assert_eq!(format.height, Some(720));
        assert_eq!(format.width, Some(1280));
        assert_eq!(format.format_note, Some("720p".to_string()));
        assert_eq!(format.vcodec, Some("h264".to_string()));
    }

    #[test]
    fn test_build_format_with_string_quality() {
        let format = build_format("hd", "https://example.com/video.mp4".to_string(), "mp4");

        assert_eq!(format.format_id, "hd");
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
}
