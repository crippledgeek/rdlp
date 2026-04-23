//! Shared base for WGCZ Holding sites (XVideos, XNXX).

pub mod patterns;

use serde_json::Value;

/// Marker struct carrying shared parse helpers for WGCZ sites.
pub struct WgczNetworkBase;

/// Format URLs extracted from inline `html5player.setXxx` calls.
#[derive(Debug, Default, Clone)]
pub struct WgczFormatUrls {
    /// HLS manifest URL (`html5player.setVideoHLS`).
    pub hls: Option<String>,
    /// Low-quality MP4 URL (`html5player.setVideoUrlLow`).
    pub mp4_low: Option<String>,
    /// High-quality MP4 URL (`html5player.setVideoUrlHigh`).
    pub mp4_high: Option<String>,
}

/// Metadata extracted from inline `html5player.setXxx` calls.
#[derive(Debug, Default, Clone)]
pub struct WgczInlineMeta {
    /// Video title (`html5player.setVideoTitle`).
    pub title: Option<String>,
    /// Thumbnail URL (`html5player.setThumbUrl`).
    pub thumbnail_url: Option<String>,
    /// Uploader name (`html5player.setUploaderName`).
    pub uploader: Option<String>,
}

/// JSON-LD `VideoObject` fields we care about.
#[derive(Debug, Default, Clone)]
pub struct WgczJsonLd {
    /// Video title from `name`.
    pub name: Option<String>,
    /// Description from `description`.
    pub description: Option<String>,
    /// ISO 8601 duration string from `duration`.
    pub duration_iso: Option<String>,
    /// Upload date string from `uploadDate`.
    pub upload_date: Option<String>,
    /// Thumbnail URL (first entry if array) from `thumbnailUrl`.
    pub thumbnail_url: Option<String>,
    /// Content URL from `contentUrl`.
    pub content_url: Option<String>,
    /// View count from `interactionStatistic.userInteractionCount`.
    pub view_count: Option<u64>,
}

impl WgczNetworkBase {
    /// Extract HLS and MP4 format URLs from inline `html5player` calls.
    #[must_use]
    pub fn extract_format_urls(html: &str) -> WgczFormatUrls {
        WgczFormatUrls {
            hls: patterns::VIDEO_HLS.captures(html).map(|c| c[1].to_string()),
            mp4_low: patterns::VIDEO_URL_LOW.captures(html).map(|c| c[1].to_string()),
            mp4_high: patterns::VIDEO_URL_HIGH.captures(html).map(|c| c[1].to_string()),
        }
    }

    /// Extract title, thumbnail, and uploader from inline `html5player` calls.
    #[must_use]
    pub fn extract_inline_meta(html: &str) -> WgczInlineMeta {
        WgczInlineMeta {
            title: patterns::VIDEO_TITLE.captures(html).map(|c| c[1].to_string()),
            thumbnail_url: patterns::THUMB_URL.captures(html).map(|c| c[1].to_string()),
            uploader: patterns::UPLOADER_NAME.captures(html).map(|c| c[1].to_string()),
        }
    }

    /// Extract the first `VideoObject` JSON-LD block from the page.
    ///
    /// Handles both `"thumbnailUrl": "url"` and `"thumbnailUrl": ["url1", ...]`.
    #[must_use]
    pub fn extract_json_ld(html: &str) -> WgczJsonLd {
        let doc = scraper::Html::parse_document(html);
        let selector = scraper::Selector::parse(r#"script[type="application/ld+json"]"#)
            .expect("static selector");
        for el in doc.select(&selector) {
            let raw = el.text().collect::<String>();
            let v: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("@type").and_then(Value::as_str) != Some("VideoObject") {
                continue;
            }
            return WgczJsonLd {
                name: v.get("name").and_then(Value::as_str).map(String::from),
                description: v.get("description").and_then(Value::as_str).map(String::from),
                duration_iso: v.get("duration").and_then(Value::as_str).map(String::from),
                upload_date: v.get("uploadDate").and_then(Value::as_str).map(String::from),
                thumbnail_url: match v.get("thumbnailUrl") {
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(Value::Array(arr)) => arr.first().and_then(Value::as_str).map(String::from),
                    _ => None,
                },
                content_url: v.get("contentUrl").and_then(Value::as_str).map(String::from),
                view_count: v
                    .get("interactionStatistic")
                    .and_then(|s| s.get("userInteractionCount"))
                    .and_then(Value::as_u64),
            };
        }
        WgczJsonLd::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("tests/wgcz_sample.html");

    #[test]
    fn wgcz_base_is_constructible() {
        let _b = WgczNetworkBase;
    }

    #[test]
    fn extracts_hls_and_mp4_from_sample() {
        let f = WgczNetworkBase::extract_format_urls(FIXTURE);
        assert!(f.hls.is_some());
        assert!(f.mp4_low.is_some());
        assert!(f.mp4_high.is_some());
        assert!(f.hls.as_deref().unwrap().ends_with(".m3u8"));
    }

    #[test]
    fn extracts_title_and_uploader() {
        let m = WgczNetworkBase::extract_inline_meta(FIXTURE);
        assert_eq!(m.title.as_deref(), Some("Example Video Title"));
        assert_eq!(m.uploader.as_deref(), Some("TestUploader"));
    }

    #[test]
    fn extracts_jsonld_videoobject() {
        let ld = WgczNetworkBase::extract_json_ld(FIXTURE);
        assert_eq!(ld.name.as_deref(), Some("Example Video Title"));
        assert_eq!(ld.duration_iso.as_deref(), Some("PT00H05M00S"));
        assert_eq!(ld.view_count, Some(12345));
    }
}
