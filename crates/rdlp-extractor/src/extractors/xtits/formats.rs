//! Format extraction for XTits (KVS player)
//!
//! XTits uses Kernel Video Sharing (KVS) which embeds video URLs directly
//! in a `flashvars` JavaScript object. Videos are direct MP4 downloads
//! (no HLS). Typical fields:
//!
//! - `video_url` + `video_url_text` — primary quality (e.g. "480p")
//! - `video_alt_url` + `video_alt_url_text` — alternate quality (e.g. "720p")

use log::debug;
use rdlp_core::Format;

use crate::base::common::BaseExtractor;
use crate::base::kvs::{KvsFlashvars, parse_kvs_flashvars};

/// Extract video formats from KVS flashvars block content.
///
/// KVS uses numbered URL fields:
/// - `video_url` / `video_url_text` — primary format
/// - `video_alt_url` / `video_alt_url_text` — first alternate
/// - `video_alt_url2` / `video_alt_url2_text` — second alternate (rare)
pub fn extract_formats(flashvars_content: &str) -> Vec<Format> {
    let flashvars = parse_kvs_flashvars(flashvars_content);
    let mut formats = Vec::new();

    // Primary format: video_url
    if flashvars.has("video_url") {
        let url = flashvars.get("video_url").unwrap();
        let quality = flashvars.get("video_url_text").unwrap_or("default");
        let format = build_kvs_format(quality, url);
        debug!(format_id = format.format_id.as_str(), url; "[XTits] Primary format");
        formats.push(format);
    }

    // Alternate format: video_alt_url
    if flashvars.has("video_alt_url") {
        let url = flashvars.get("video_alt_url").unwrap();
        let quality = flashvars.get("video_alt_url_text").unwrap_or("alt");
        let format = build_kvs_format(quality, url);
        debug!(format_id = format.format_id.as_str(), url; "[XTits] Alt format");
        formats.push(format);
    }

    // Second alternate (rare): video_alt_url2
    if flashvars.has("video_alt_url2") {
        let url = flashvars.get("video_alt_url2").unwrap();
        let quality = flashvars.get("video_alt_url2_text").unwrap_or("alt2");
        let format = build_kvs_format(quality, url);
        debug!(format_id = format.format_id.as_str(), url; "[XTits] Alt2 format");
        formats.push(format);
    }

    formats
}

/// Extract thumbnail URL from flashvars
pub fn extract_thumbnail(flashvars: &KvsFlashvars) -> Option<String> {
    flashvars.get("preview_url").map(|s| s.to_string())
}

/// Extract duration from flashvars (KVS stores as seconds string)
pub fn extract_duration(flashvars: &KvsFlashvars) -> Option<f64> {
    flashvars.get_f64("video_duration")
}

/// Parse flashvars content and return structured KvsFlashvars (public for mod.rs)
pub fn parse_flashvars_public(flashvars_content: &str) -> KvsFlashvars {
    parse_kvs_flashvars(flashvars_content)
}

/// Build a Format from a KVS quality string and URL.
fn build_kvs_format(quality_str: &str, url: &str) -> Format {
    let height = BaseExtractor::parse_quality_height(quality_str);

    let mut format = BaseExtractor::build_format(
        quality_str.to_owned(),
        url.to_owned(),
        "mp4".to_owned(),
        height,
    );

    format.container = Some("mp4".to_owned());

    // KVS quality labels without height info get stored as format_note
    if height.is_none() {
        format.format_note = Some(quality_str.to_owned());
    }

    format
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_flashvars() {
        let content = r#"
            video_id: '183207',
            video_url: 'https://example.com/video.mp4/',
            video_url_text: '480p',
            video_alt_url: 'https://example.com/video_720p.mp4/',
            video_alt_url_text: '720p',
            preview_url: 'https://example.com/preview.jpg',
        "#;

        let flashvars = parse_kvs_flashvars(content);
        assert_eq!(flashvars.get("video_id"), Some("183207"));
        assert_eq!(flashvars.get("video_url_text"), Some("480p"));
        assert_eq!(flashvars.get("video_alt_url_text"), Some("720p"));
    }

    #[test]
    fn test_extract_formats() {
        let content = r#"
            video_url: 'https://example.com/video.mp4/',
            video_url_text: '480p',
            video_alt_url: 'https://example.com/video_720p.mp4/',
            video_alt_url_text: '720p',
        "#;

        let formats = extract_formats(content);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].format_id, "480p");
        assert_eq!(formats[0].height, Some(480));
        assert_eq!(formats[1].format_id, "720p");
        assert_eq!(formats[1].height, Some(720));
    }

    #[test]
    fn test_extract_formats_single() {
        let content = r#"
            video_url: 'https://example.com/video.mp4/',
            video_url_text: '480p',
        "#;

        let formats = extract_formats(content);
        assert_eq!(formats.len(), 1);
    }

    #[test]
    fn test_extract_formats_empty_url_skipped() {
        let content = r#"
            video_url: 'https://example.com/video.mp4/',
            video_url_text: '480p',
            video_alt_url: '',
            video_alt_url_text: '720p',
        "#;

        let formats = extract_formats(content);
        assert_eq!(formats.len(), 1);
    }

    #[test]
    fn test_build_kvs_format() {
        let format = build_kvs_format("720p", "https://example.com/video.mp4");
        assert_eq!(format.format_id, "720p");
        assert_eq!(format.height, Some(720));
        assert_eq!(format.width, Some(1280));
        assert_eq!(format.ext, "mp4");
        assert_eq!(format.container, Some("mp4".to_string()));
    }

    #[test]
    fn test_build_kvs_format_unknown_quality() {
        let format = build_kvs_format("default", "https://example.com/video.mp4");
        assert_eq!(format.format_id, "default");
        assert_eq!(format.height, None);
        assert_eq!(format.format_note, Some("default".to_string()));
    }

    #[test]
    fn test_parse_flashvars_single_line() {
        // Real KVS pages often emit all flashvars on a single line
        let content = "video_id: '183207',                                                                                     video_url: 'https://www.xtits.xxx/get_file/7/abc/183207.mp4/',                                                                                     video_url_text: '480p',                                                                                     video_alt_url: 'https://www.xtits.xxx/get_file/7/def/183207_720p.mp4/',                                                                                     video_alt_url_text: '720p',                                                                                     video_alt_url_hd: '1',                                                                                     preview_url: 'https://www.xtits.xxx/contents/videos_screenshots/183000/183207/preview.jpg'";

        let flashvars = parse_kvs_flashvars(content);
        assert_eq!(flashvars.get("video_id"), Some("183207"));
        assert_eq!(flashvars.get("video_url_text"), Some("480p"));
        assert_eq!(flashvars.get("video_alt_url_text"), Some("720p"));
        assert!(flashvars.get("video_url").unwrap().contains("183207.mp4"));
        assert!(flashvars.get("video_alt_url").unwrap().contains("720p.mp4"));

        let formats = extract_formats(content);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].height, Some(480));
        assert_eq!(formats[1].height, Some(720));
    }

    #[test]
    fn test_extract_thumbnail() {
        let flashvars = parse_kvs_flashvars("preview_url: 'https://example.com/thumb.jpg',");
        assert_eq!(
            extract_thumbnail(&flashvars),
            Some("https://example.com/thumb.jpg".to_string())
        );
    }

    #[test]
    fn test_extract_duration() {
        let flashvars = parse_kvs_flashvars("video_duration: '1698',");
        assert_eq!(extract_duration(&flashvars), Some(1698.0));
    }
}
