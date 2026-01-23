//! Format extraction for RedTube
//!
//! Extracts video formats from JavaScript sources and mediaDefinition arrays.

use rdlp_core::{ExtractionContext, Format};
use serde_json::Value;

use crate::base::common::BaseExtractor;
use crate::utils::{extract_extension_from_url, make_absolute_url};

use super::patterns::{MEDIA_DEF_PATTERN, SOURCES_PATTERN};

/// Extract quality string from JSON value (handles both string and number types)
pub fn parse_quality(item: &Value) -> String {
    item.get("quality")
        .and_then(|q| {
            q.as_str()
                .map(String::from)
                .or_else(|| q.as_i64().map(|i| i.to_string()))
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Build Format from quality string and URL using BaseExtractor utilities
pub fn build_format(quality_str: &str, url: String, format_type: &str) -> Format {
    let height = BaseExtractor::parse_quality_height(quality_str);

    let mut format = Format::new(
        quality_str.to_string(),
        url,
        format_type.to_string(),
        "https".to_string(),
    );

    if let Some(h) = height {
        format.height = Some(h);
        format.width = Some(BaseExtractor::width_from_height(h));
        format.quality = Some((h / 100) as i32);
        format.format_note = Some(format!("{h}p"));
    } else {
        format.format_note = Some(quality_str.to_string());
    }

    format.vcodec = Some("h264".to_string());
    format.acodec = Some("aac".to_string());

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
pub fn extract_from_sources(webpage: &str, verbose: bool) -> Vec<Format> {
    let mut formats = Vec::new();

    if let Some(caps) = SOURCES_PATTERN.captures(webpage) {
        if let Some(sources_str) = caps.get(1) {
            if verbose {
                eprintln!("[RedTube] Found sources object: {}", sources_str.as_str());
            }

            // Try to parse as JSON
            match serde_json::from_str::<Value>(sources_str.as_str()) {
                Ok(sources) => {
                    if let Some(obj) = sources.as_object() {
                        for (quality, url) in obj {
                            if let Some(url_str) = url.as_str() {
                                let format_type = get_format_type_from_url(url_str);
                                let format = build_format(quality, url_str.to_string(), format_type);

                                if verbose {
                                    eprintln!(
                                        "[RedTube] Extracted format: {} ({})",
                                        format.format_id,
                                        format.format_note.as_deref().unwrap_or("unknown")
                                    );
                                }

                                formats.push(format);
                            }
                        }
                    }
                }
                Err(e) => {
                    if verbose {
                        eprintln!(
                            "[RedTube] Failed to parse sources JSON at {}:{}: {}",
                            e.line(),
                            e.column(),
                            e
                        );
                    }
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
                        BaseExtractor::log_if_verbose(ctx, "RedTube", "mediaDefinition is not an array");
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
                            let format_type = item
                                .get("format")
                                .and_then(|v| v.as_str())
                                .unwrap_or("mp4");

                            let has_quality = item.get("quality").is_some();

                            // If format is mp4/hls without quality, fetch JSON to get actual formats
                            if (format_type == "mp4" || format_type == "hls") && !has_quality {
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
            if ctx.config.verbose {
                if e.is_timeout() {
                    eprintln!("[RedTube] Request timed out for URL: {absolute_url}");
                } else if e.is_connect() {
                    eprintln!("[RedTube] Connection failed for URL: {absolute_url}: {e}");
                } else {
                    eprintln!("[RedTube] Request failed: {e}");
                }
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

        let formats = extract_from_sources(webpage, false);
        assert_eq!(formats.len(), 2);

        // Check that we got both formats
        assert!(formats.iter().any(|f| f.format_id == "720"));
        assert!(formats.iter().any(|f| f.format_id == "1080"));

        // Check format_note is set
        assert!(formats
            .iter()
            .any(|f| f.format_note == Some("720p".to_string())));
        assert!(formats
            .iter()
            .any(|f| f.format_note == Some("1080p".to_string())));
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
        assert_eq!(get_format_type_from_url("https://example.com/video.mp4"), "mp4");
        assert_eq!(get_format_type_from_url("https://example.com/playlist.m3u8"), "hls");
        assert_eq!(get_format_type_from_url("https://example.com/video.webm"), "webm");
        assert_eq!(get_format_type_from_url("https://example.com/video"), "unknown");
    }
}
