//! Format extraction for SpankBang.
//!
//! Two paths in priority order:
//!
//! 1. **Inline `stream_data`**: the video page embeds a Python-dict literal
//!    with every resolution and HLS variant. Single GET, no extra round-trip.
//! 2. **Formats API fallback**: when `stream_data` is absent (legacy pages),
//!    regex-extract `data-streamkey`, POST to `/api/videos/stream`, parse
//!    the same JSON shape.

use rdlp_types::{DownloadProtocol, Format};
use serde_json::Value;

use super::patterns;

/// Map a SpankBang JSON-key like "1080p" to a height in pixels.
fn height_for_label(label: &str) -> Option<u32> {
    match label {
        "240p" => Some(240),
        "320p" => Some(320),
        "480p" => Some(480),
        "720p" => Some(720),
        "1080p" => Some(1080),
        "4k" => Some(2160),
        _ => None,
    }
}

/// Extract the inline `stream_data` JSON value from a SpankBang video page.
/// Returns `None` if the page does not embed it (caller should fall back to
/// the formats-API path).
pub(super) fn parse_inline_stream_data(html: &str) -> Option<Value> {
    let caps = patterns::STREAM_DATA_INLINE.captures(html)?;
    let body = caps.get(1)?.as_str();
    let json = patterns::pydict_to_json(body);
    serde_json::from_str(&json).ok()
}

/// Extract the `data-streamkey` token from a video-page HTML for the
/// formats-API fallback. Returns `None` when the anchor is missing.
pub(super) fn parse_streamkey(html: &str) -> Option<String> {
    patterns::STREAMKEY
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Convert a SpankBang `stream_data` / API JSON object to `Vec<Format>`.
/// Skips empty arrays. Sets `vcodec=h264`, `acodec=aac`, `container=mp4`
/// for direct MP4s; HLS rows get `protocol = M3u8`.
pub(super) fn build_formats(data: &Value) -> Vec<Format> {
    let mut formats = Vec::new();
    let Value::Object(map) = data else {
        return formats;
    };

    // Direct MP4 progressive.
    for label in ["240p", "320p", "480p", "720p", "1080p", "4k"] {
        if let Some(arr) = map.get(label).and_then(Value::as_array)
            && let Some(url) = arr.first().and_then(Value::as_str)
        {
            let mut f = Format::new(label, url, "mp4", DownloadProtocol::Https);
            f.vcodec = Some("h264".to_string());
            f.acodec = Some("aac".to_string());
            f.container = Some("mp4".to_string());
            f.height = height_for_label(label);
            f.format_note = Some(label.to_uppercase());
            formats.push(f);
        }
    }

    // Multi-rendition master HLS playlist.
    if let Some(arr) = map.get("m3u8").and_then(Value::as_array)
        && let Some(url) = arr.first().and_then(Value::as_str)
    {
        let mut f = Format::new("hls", url, "m3u8", DownloadProtocol::M3u8);
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("aac".to_string());
        f.container = Some("m3u8".to_string());
        f.format_note = Some("HLS".to_string());
        formats.push(f);
    }

    // Single-rendition HLS variants (one per resolution).
    // 320p is intentionally omitted — SpankBang does not serve a stable
    // m3u8_320p stream; matches yt-dlp's reference behaviour.
    for label in ["240p", "480p", "720p", "1080p", "4k"] {
        let key = format!("m3u8_{label}");
        if let Some(arr) = map.get(&key).and_then(Value::as_array)
            && let Some(url) = arr.first().and_then(Value::as_str)
        {
            let mut f = Format::new(&key, url, "m3u8", DownloadProtocol::M3u8);
            f.vcodec = Some("h264".to_string());
            f.acodec = Some("aac".to_string());
            f.container = Some("m3u8".to_string());
            f.height = height_for_label(label);
            f.format_note = Some(format!("HLS {}", label.to_uppercase()));
            formats.push(f);
        }
    }

    formats
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("tests/spankbang_video_page.html");
    const API: &str = include_str!("tests/spankbang_formats_api.json");

    #[test]
    fn inline_stream_data_present_in_fixture() {
        let data = parse_inline_stream_data(PAGE).expect("inline stream_data must parse");
        assert!(data.is_object());
        assert!(data.get("1080p").is_some(), "1080p key present");
        assert!(data.get("m3u8").is_some(), "m3u8 master present");
        assert_eq!(data.get("length").and_then(Value::as_u64), Some(910));
    }

    #[test]
    fn streamkey_extractable_from_fixture() {
        let key = parse_streamkey(PAGE).expect("data-streamkey must be present");
        // Exact value from the recorded fixture — deterministic, locks the
        // shape <base>.<sig> contract.
        assert_eq!(key, "MTcwMDE4NDU.XTPJk92TYuF3gscEzR5UhXY4ttk");
    }

    #[test]
    fn build_formats_from_inline_data() {
        let data = parse_inline_stream_data(PAGE).expect("inline parse");
        let formats = build_formats(&data);
        assert!(!formats.is_empty(), "at least one format");
        // 1080p MP4 must be present + carry the right shape.
        let f1080 = formats
            .iter()
            .find(|f| f.format_id == "1080p")
            .expect("1080p MP4 row");
        assert_eq!(f1080.ext, "mp4");
        assert_eq!(f1080.height, Some(1080));
        assert_eq!(f1080.vcodec.as_deref(), Some("h264"));
        // Multi-rendition HLS master row.
        let hls = formats
            .iter()
            .find(|f| f.format_id == "hls")
            .expect("hls master row");
        assert_eq!(hls.ext, "m3u8");

        // Empty arrays in the fixture (`'320p': []`, `'4k': []`) must NOT
        // produce format rows.
        assert!(
            formats.iter().all(|f| f.format_id != "320p"),
            "320p is empty in fixture; must not produce a row"
        );
        assert!(
            formats.iter().all(|f| f.format_id != "4k"),
            "4k is empty in fixture; must not produce a row"
        );
    }

    #[test]
    fn build_formats_from_api_json() {
        let data: Value = serde_json::from_str(API).expect("API JSON parse");
        let formats = build_formats(&data);
        assert!(!formats.is_empty(), "API path also produces formats");
        assert!(formats.iter().any(|f| f.format_id == "1080p"));
        assert!(formats.iter().any(|f| f.format_id == "hls"));
    }
}
