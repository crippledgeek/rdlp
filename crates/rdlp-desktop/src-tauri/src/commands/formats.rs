//! Format listing IPC commands.
//!
//! Exposes available download formats for a given URL so the
//! frontend can present an interactive format selector. The
//! [`get_formats`] command validates the URL via SSRF protection,
//! extracts metadata through the rdlp-api client, and returns a
//! serialisable [`FormatListResponse`] containing format details,
//! subtitles, thumbnail URL, and duration.

use serde::Serialize;
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;

/// Frontend-facing format information.
///
/// A simplified projection of [`rdlp_types::Format`] that exposes
/// only the fields the UI needs, without internal download URLs or
/// protocol details.
#[derive(Debug, Serialize)]
pub struct FormatInfo {
    /// Unique format identifier (e.g. "137", "hls-720").
    pub format_id: String,
    /// File extension (e.g. "mp4", "webm").
    pub ext: String,
    /// Human-readable quality label (e.g. "720p", "1080p").
    pub format_note: Option<String>,
    /// Video width in pixels.
    pub width: Option<u32>,
    /// Video height in pixels.
    pub height: Option<u32>,
    /// Frames per second.
    pub fps: Option<f64>,
    /// Total bitrate in kbps (video + audio).
    pub tbr: Option<f64>,
    /// Video codec name (e.g. "h264", "vp9").
    pub vcodec: Option<String>,
    /// Audio codec name (e.g. "aac", "opus").
    pub acodec: Option<String>,
    /// File size in bytes (exact or approximate).
    pub filesize: Option<u64>,
    /// Whether this format contains a video stream.
    pub has_video: bool,
    /// Whether this format contains an audio stream.
    pub has_audio: bool,
}

/// Subtitle availability for a single language.
#[derive(Debug, Serialize)]
pub struct SubtitleInfo {
    /// Language code (e.g. "en", "ja").
    pub lang: String,
    /// Available subtitle file extensions (e.g. \["srt", "vtt"\]).
    pub formats: Vec<String>,
}

/// Complete response for the `get_formats` command.
///
/// Contains all metadata the frontend needs to render the format
/// selection UI.
#[derive(Debug, Serialize)]
pub struct FormatListResponse {
    /// Video title.
    pub title: String,
    /// Available download formats.
    pub formats: Vec<FormatInfo>,
    /// Available subtitle languages and their formats.
    pub subtitles: Vec<SubtitleInfo>,
    /// URL of the best thumbnail image.
    pub thumbnail_url: Option<String>,
    /// Video duration in seconds.
    pub duration: Option<f64>,
}

/// Retrieve available formats for a URL.
///
/// Validates the URL for SSRF safety, extracts metadata via the
/// rdlp-api client, and maps the result into a frontend-friendly
/// [`FormatListResponse`].
///
/// # Arguments
///
/// * `url` - The webpage URL to extract formats from.
/// * `state` - Application state containing the [`rdlp_api::RdlpClient`].
///
/// # Returns
///
/// A [`FormatListResponse`] on success, or an [`AppError`] on failure.
///
/// # Errors
///
/// * [`AppError::InvalidInput`] if the URL fails SSRF validation.
/// * [`AppError::ExtractionFailed`] if no metadata is returned.
/// * Other [`AppError`] variants mapped from [`rdlp_api::RdlpApiError`].
#[tauri::command]
pub async fn get_formats(
    url: String,
    state: State<'_, AppState>,
) -> Result<FormatListResponse, AppError> {
    // 1. SSRF / URL validation
    rdlp_security::validate_url_security(&url).map_err(|e| AppError::InvalidInput {
        field: "url".to_owned(),
        message: e.to_string(),
    })?;

    // 2. Extract metadata
    let info_dicts = state
        .client
        .extract_info(&url)
        .await
        .map_err(AppError::from)?;

    // 3. Take the first result (single video or first playlist entry)
    let info = info_dicts
        .into_iter()
        .next()
        .ok_or_else(|| AppError::ExtractionFailed {
            message: "No metadata returned for URL".to_owned(),
        })?;

    // 4. Map formats
    let formats = info
        .formats
        .iter()
        .map(|f| FormatInfo {
            format_id: f.format_id.clone(),
            ext: f.ext.clone(),
            format_note: f.format_note.clone(),
            width: f.width,
            height: f.height,
            fps: f.fps,
            tbr: f.tbr,
            vcodec: f.vcodec.clone(),
            acodec: f.acodec.clone(),
            filesize: f.get_filesize(),
            has_video: f.has_video(),
            has_audio: f.has_audio(),
        })
        .collect();

    // 5. Map subtitles
    let subtitles = info
        .subtitles
        .as_ref()
        .map(|subs| {
            subs.iter()
                .map(|(lang, entries)| SubtitleInfo {
                    lang: lang.clone(),
                    formats: entries.iter().map(|s| s.ext.clone()).collect(),
                })
                .collect()
        })
        .unwrap_or_default();

    // 6. Build response
    Ok(FormatListResponse {
        title: info.title,
        formats,
        subtitles,
        thumbnail_url: info.thumbnail,
        duration: info.duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_info_serialization() {
        let info = FormatInfo {
            format_id: "137".to_owned(),
            ext: "mp4".to_owned(),
            format_note: Some("1080p".to_owned()),
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
            tbr: Some(4500.0),
            vcodec: Some("h264".to_owned()),
            acodec: Some("aac".to_owned()),
            filesize: Some(524_288_000),
            has_video: true,
            has_audio: true,
        };

        let json = serde_json::to_value(&info).expect("serialization");
        assert_eq!(json["format_id"], "137");
        assert_eq!(json["ext"], "mp4");
        assert_eq!(json["width"], 1920);
        assert_eq!(json["height"], 1080);
        assert_eq!(json["has_video"], true);
        assert_eq!(json["has_audio"], true);
        assert_eq!(json["filesize"], 524_288_000u64);
    }

    #[test]
    fn test_format_info_optional_fields_null() {
        let info = FormatInfo {
            format_id: "hls-auto".to_owned(),
            ext: "mp4".to_owned(),
            format_note: None,
            width: None,
            height: None,
            fps: None,
            tbr: None,
            vcodec: None,
            acodec: None,
            filesize: None,
            has_video: false,
            has_audio: true,
        };

        let json = serde_json::to_value(&info).expect("serialization");
        assert!(json["format_note"].is_null());
        assert!(json["width"].is_null());
        assert!(json["filesize"].is_null());
        assert!(!json["has_video"].as_bool().unwrap());
        assert!(json["has_audio"].as_bool().unwrap());
    }

    #[test]
    fn test_subtitle_info_serialization() {
        let sub = SubtitleInfo {
            lang: "en".to_owned(),
            formats: vec!["srt".to_owned(), "vtt".to_owned()],
        };

        let json = serde_json::to_value(&sub).expect("serialization");
        assert_eq!(json["lang"], "en");
        let fmts = json["formats"].as_array().unwrap();
        assert_eq!(fmts.len(), 2);
        assert_eq!(fmts[0], "srt");
        assert_eq!(fmts[1], "vtt");
    }

    #[test]
    fn test_format_list_response_serialization() {
        let resp = FormatListResponse {
            title: "Test Video".to_owned(),
            formats: vec![FormatInfo {
                format_id: "22".to_owned(),
                ext: "mp4".to_owned(),
                format_note: Some("720p".to_owned()),
                width: Some(1280),
                height: Some(720),
                fps: Some(30.0),
                tbr: Some(2500.0),
                vcodec: Some("h264".to_owned()),
                acodec: Some("aac".to_owned()),
                filesize: Some(100_000_000),
                has_video: true,
                has_audio: true,
            }],
            subtitles: vec![SubtitleInfo {
                lang: "en".to_owned(),
                formats: vec!["vtt".to_owned()],
            }],
            thumbnail_url: Some("https://example.com/thumb.jpg".to_owned()),
            duration: Some(180.5),
        };

        let json = serde_json::to_value(&resp).expect("serialization");
        assert_eq!(json["title"], "Test Video");
        assert_eq!(json["formats"].as_array().unwrap().len(), 1);
        assert_eq!(json["subtitles"].as_array().unwrap().len(), 1);
        assert_eq!(json["thumbnail_url"], "https://example.com/thumb.jpg");
        assert_eq!(json["duration"], 180.5);
    }

    #[test]
    fn test_format_list_response_empty() {
        let resp = FormatListResponse {
            title: "Empty Video".to_owned(),
            formats: vec![],
            subtitles: vec![],
            thumbnail_url: None,
            duration: None,
        };

        let json = serde_json::to_value(&resp).expect("serialization");
        assert_eq!(json["title"], "Empty Video");
        assert!(json["formats"].as_array().unwrap().is_empty());
        assert!(json["subtitles"].as_array().unwrap().is_empty());
        assert!(json["thumbnail_url"].is_null());
        assert!(json["duration"].is_null());
    }
}
