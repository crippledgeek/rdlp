//! Format extraction for PornHub
//!
//! Extracts video formats from various sources in the page.

use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use rdlp_types::Format;
use serde_json::Value;

use super::patterns::{
    DOWNLOAD_BTN_PATTERN, FLASHVARS_PATTERN, MEDIA_VAR_PATTERN, QUALITY_FROM_URL_PATTERN,
    QUALITY_ITEMS_PATTERN,
};
use crate::base::common::BaseExtractor;
use std::collections::HashSet;

/// Extend `dest` with `formats`, skipping URLs already in `seen`.
fn extend_deduped(dest: &mut Vec<Format>, seen: &mut HashSet<String>, formats: Vec<Format>) {
    for format in formats {
        if seen.insert(format.url.clone()) {
            dest.push(format);
        }
    }
}

/// Extract all formats using multiple strategies
///
/// Tries strategies in order:
/// 1. flashvars mediaDefinitions (primary)
/// 2. JavaScript variables (qualityItems_*, media_*, quality_*)
/// 3. Download buttons
pub async fn extract_all_formats(webpage: &str, ctx: &ExtractionContext) -> Result<Vec<Format>> {
    let mut all_formats = Vec::new();
    let mut seen_urls = HashSet::new();

    // Strategy 1: flashvars (primary)
    if let Ok(formats) = extract_from_flashvars(webpage, ctx).await {
        extend_deduped(&mut all_formats, &mut seen_urls, formats);
        if !all_formats.is_empty() {
            debug!(count = all_formats.len(); "[PornHub] Extracted formats from flashvars");
        }
    }

    // Strategy 2: JavaScript variables
    extend_deduped(
        &mut all_formats,
        &mut seen_urls,
        extract_from_js_vars(webpage),
    );

    // Strategy 3: Download buttons
    extend_deduped(
        &mut all_formats,
        &mut seen_urls,
        extract_from_download_buttons(webpage),
    );

    if all_formats.is_empty() {
        return Err(RdlpError::Extraction {
            message: "No video formats found with any strategy".to_string(),
            url: None,
        });
    }

    dedup_format_ids(&mut all_formats);

    debug!(count = all_formats.len(); "[PornHub] Total unique formats");

    Ok(all_formats)
}

/// Ensure format_ids are unique by appending "-2", "-3", etc. for duplicates.
///
/// Delegates to [`BaseExtractor::dedup_format_ids`].
fn dedup_format_ids(formats: &mut [Format]) {
    BaseExtractor::dedup_format_ids(formats);
}

/// Extract formats from flashvars JavaScript object
async fn extract_from_flashvars(webpage: &str, ctx: &ExtractionContext) -> Result<Vec<Format>> {
    let flashvars_json = FLASHVARS_PATTERN
        .captures(webpage)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str())
        .ok_or_else(|| RdlpError::Extraction {
            message: "No flashvars found".to_string(),
            url: None,
        })?;

    let flashvars: Value = serde_json::from_str(flashvars_json).map_err(|e| RdlpError::Extraction {
        message: format!("Failed to parse flashvars: {e}"),
        url: None,
    })?;

    let mut formats = Vec::new();

    let media_definitions = flashvars
        .get("mediaDefinitions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| RdlpError::Extraction {
            message: "No mediaDefinitions in flashvars".to_string(),
            url: None,
        })?;

    // Separate get_media endpoints (need async fetch) from direct URLs (sync)
    let mut media_futures = Vec::new();
    for (idx, definition) in media_definitions.iter().enumerate() {
        let video_url = match definition.get("videoUrl").and_then(|v| v.as_str()) {
            Some(url) if !url.is_empty() => url,
            _ => continue,
        };

        if video_url.contains("/video/get_media") {
            media_futures.push(fetch_media_formats(video_url, idx, ctx));
        } else if let Some(format) = build_format_from_definition(definition, video_url, idx) {
            formats.push(format);
        }
    }

    // Fetch all get_media endpoints in parallel
    let media_results = futures::future::join_all(media_futures).await;
    for result in media_results.into_iter().flatten() {
        formats.extend(result);
    }

    if formats.is_empty() {
        return Err(RdlpError::Extraction {
            message: "No formats in mediaDefinitions".to_string(),
            url: None,
        });
    }

    Ok(formats)
}

/// Fetch formats from get_media endpoint
async fn fetch_media_formats(
    url: &str,
    idx: usize,
    ctx: &ExtractionContext,
) -> Option<Vec<Format>> {
    debug!("[PornHub] Fetching formats from get_media endpoint...");

    let response = ctx.http_client.get(url).send().await.ok()?;

    if !response.status().is_success() {
        debug!(status:? = response.status(); "[PornHub] get_media returned non-success");
        return None;
    }

    let json_text = response.text().await.ok()?;
    let media_array: Value = serde_json::from_str(&json_text).ok()?;

    let mut formats = Vec::new();

    for item in media_array.as_array()? {
        let real_url = item.get("videoUrl").and_then(|v| v.as_str())?;
        let quality = item.get("quality").and_then(parse_quality);

        let format = build_format(real_url, quality, idx);

        debug!(format_id:? = format.format_id; "[PornHub] Found format");

        formats.push(format);
    }

    Some(formats)
}

/// Build format from mediaDefinition entry
fn build_format_from_definition(definition: &Value, url: &str, idx: usize) -> Option<Format> {
    let quality = definition
        .get("quality")
        .or_else(|| definition.get("defaultQuality"))
        .and_then(parse_quality)
        .or_else(|| parse_quality_from_url(url));

    Some(build_format(url, quality, idx))
}

/// Build a Format struct with common settings.
///
/// Delegates to `BaseExtractor::build_format()` for the shared logic
/// (height/width, format_note, codec defaults, quality score).
fn build_format(url: &str, quality: Option<u64>, idx: usize) -> Format {
    let height = quality.map(|q| q as u32);
    let format_id = quality
        .map(|q| format!("{q}p"))
        .unwrap_or_else(|| format!("format_{idx}"));

    BaseExtractor::build_format(format_id, url.to_string(), "mp4".to_string(), height)
}

/// Parse quality from JSON value (handles string or number)
fn parse_quality(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// Parse quality from URL pattern (e.g., "720P_1200K")
fn parse_quality_from_url(url: &str) -> Option<u64> {
    QUALITY_FROM_URL_PATTERN
        .captures(url)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Extract formats from JavaScript variables
fn extract_from_js_vars(webpage: &str) -> Vec<Format> {
    let mut formats = Vec::new();

    // Strategy 1: qualityItems_* JSON arrays
    for caps in QUALITY_ITEMS_PATTERN.captures_iter(webpage) {
        if let Some(json_str) = caps.get(1)
            && let Ok(items) = serde_json::from_str::<Value>(json_str.as_str())
            && let Some(array) = items.as_array()
        {
            for item in array {
                if let (Some(url), Some(quality)) = (
                    item.get("url").and_then(|v| v.as_str()),
                    item.get("quality").and_then(|v| v.as_str()),
                ) {
                    let format = build_format(url, quality.parse().ok(), 0);
                    formats.push(format);

                    debug!(quality; "[PornHub] Found format from qualityItems");
                }
            }
        }
    }

    // Strategy 2: media_*/quality_* direct URLs
    for caps in MEDIA_VAR_PATTERN.captures_iter(webpage) {
        if let (Some(quality_str), Some(url)) = (caps.get(2), caps.get(3)) {
            let url_str = url.as_str();
            if !url_str.starts_with("http") {
                continue;
            }

            let quality_name = quality_str.as_str();
            let quality: Option<u64> = quality_name.parse().ok();

            let format = build_format(url_str, quality, 0);

            debug!(quality:? = quality_name; "[PornHub] Found format from JS var");

            formats.push(format);
        }
    }

    formats
}

/// Extract formats from download buttons
fn extract_from_download_buttons(webpage: &str) -> Vec<Format> {
    DOWNLOAD_BTN_PATTERN
        .captures_iter(webpage)
        .filter_map(|caps| {
            let url = caps.get(1)?.as_str();
            let quality = parse_quality_from_url(url);
            let format_id = quality
                .map(|q| format!("{q}p"))
                .unwrap_or_else(|| "download".to_string());
            let height = quality.map(|q| q as u32);

            debug!(format_id:?; "[PornHub] Found format from download button");

            Some(BaseExtractor::build_format(
                format_id,
                url.to_string(),
                "mp4".to_string(),
                height,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_from_js_vars() {
        let webpage = r#"
            var quality_720 = "https://example.com/720.mp4";
            var media_1080 = "https://example.com/1080.mp4";
        "#;

        let formats = extract_from_js_vars(webpage);
        assert!(formats.len() >= 2);
        assert!(formats.iter().any(|f| f.url.contains("720")));
        assert!(formats.iter().any(|f| f.url.contains("1080")));
    }

    #[test]
    fn test_extract_from_download_buttons() {
        let webpage = r#"
            <a class="downloadBtn" href="https://example.com/720p_1000k.mp4">720p</a>
            <a class="downloadBtn" href="https://example.com/1080p_2000k.mp4">1080p</a>
        "#;

        let formats = extract_from_download_buttons(webpage);
        assert_eq!(formats.len(), 2);
    }

    #[test]
    fn test_parse_quality_from_url() {
        assert_eq!(
            parse_quality_from_url("https://example.com/720P_1200K.mp4"),
            Some(720)
        );
        assert_eq!(
            parse_quality_from_url("https://example.com/1080p_4000k.mp4"),
            Some(1080)
        );
        assert_eq!(
            parse_quality_from_url("https://example.com/video.mp4"),
            None
        );
    }

    #[test]
    fn test_build_format() {
        let format = build_format("https://example.com/video.mp4", Some(1080), 0);

        assert_eq!(format.format_id, "1080p");
        assert_eq!(format.height, Some(1080));
        assert_eq!(format.width, Some(1920));
        assert_eq!(format.vcodec, Some("h264".to_string()));
    }

    #[test]
    fn test_duplicate_format_ids_get_suffixed() {
        let mut formats = vec![
            build_format("https://cdn1.example.com/1080.mp4", Some(1080), 0),
            build_format("https://cdn2.example.com/1080.mp4", Some(1080), 1),
            build_format("https://cdn1.example.com/720.mp4", Some(720), 2),
            build_format("https://cdn2.example.com/720.mp4", Some(720), 3),
            build_format("https://cdn3.example.com/720.mp4", Some(720), 4),
        ];

        dedup_format_ids(&mut formats);

        assert_eq!(formats[0].format_id, "1080p");
        assert_eq!(formats[1].format_id, "1080p-2");
        assert_eq!(formats[2].format_id, "720p");
        assert_eq!(formats[3].format_id, "720p-2");
        assert_eq!(formats[4].format_id, "720p-3");
    }
}
