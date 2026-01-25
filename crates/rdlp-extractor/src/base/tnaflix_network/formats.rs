//! Format building and video source parsing for TNAFlix network sites
//!
//! Provides functions for extracting video source URLs from HTML and
//! building Format objects with filesize detection.

use log::{debug, warn};
use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, Format};
use regex::Regex;
use scraper::{Html, Selector};

/// Video metadata extracted from HTML: (format_id, video_url, ext, height, width)
pub(crate) type VideoMetadata = (String, String, String, Option<u32>, Option<u32>);

// ============================================================================
// Static Selectors and Patterns
// ============================================================================

/// Selector for video source tags: <source src="..." type="video/mp4">
pub(crate) static SOURCE_SELECTOR: Lazy<Selector> =
    Lazy::new(|| Selector::parse("source[src][type='video/mp4']").expect("Valid CSS selector"));

/// Regex to extract CDN URL from MovieFap JavaScript
pub(crate) static CDN_URL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"url:\s*['"]([^'"]+/cdn\.php[^'"]+)['"]"#).expect("Valid CDN URL regex")
});

/// Regex to extract video items from MovieFap XML
pub(crate) static MOVIEFAP_XML_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<item>.*?<res>([^<]+)</res>.*?<videoLink>([^<]+)</videoLink>.*?</item>")
        .expect("Valid MovieFap XML regex")
});

/// Regex patterns for extracting config URLs (multiple fallback strategies)
#[allow(dead_code)] // Used by extract_config_url which is tested
pub(crate) static CONFIG_URL_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r#"flashvars\.config\s*=\s*escape\("([^"]+)""#).expect("Valid config pattern 1"),
        Regex::new(r#"<input[^>]+name="config\d?"[^>]+value="([^"]+)""#)
            .expect("Valid config pattern 2"),
        Regex::new(r#"config\s*=\s*["']([^"']+)["']"#).expect("Valid config pattern 3"),
    ]
});

// ============================================================================
// Video Source Parsing
// ============================================================================

/// Parse video source tags from HTML and extract format metadata
///
/// Looks for: `<source src="..." type="video/mp4" size="720">`
pub(crate) fn parse_video_sources(html: &Html) -> Vec<VideoMetadata> {
    let mut video_data = Vec::new();

    for source_elem in html.select(&SOURCE_SELECTOR) {
        let video_url = match source_elem.value().attr("src") {
            Some(url) => url,
            None => continue,
        };

        let quality_str = source_elem.value().attr("size").unwrap_or("unknown");
        let height = quality_str.parse::<u32>().ok();
        let width = height.map(|h| (h * 16) / 9);
        let ext = extract_extension_from_url(video_url);

        let format_id = if quality_str != "unknown" {
            format!("http-{quality_str}")
        } else {
            "http-default".into()
        };

        video_data.push((format_id, video_url.to_owned(), ext.to_owned(), height, width));
    }

    video_data
}

/// Extract config URL from HTML using multiple fallback patterns
#[allow(dead_code)] // Tested but not used by current extractors
pub(crate) fn extract_config_url(html_text: &str) -> Option<String> {
    for pattern in CONFIG_URL_PATTERNS.iter() {
        if let Some(caps) = pattern.captures(html_text) {
            if let Some(url_match) = caps.get(1) {
                return Some(url_match.as_str().to_string());
            }
        }
    }
    None
}

/// Extract cdn.php URL from MovieFap JavaScript
pub(crate) fn extract_cdn_url(webpage: &str) -> Option<String> {
    CDN_URL_REGEX
        .captures(webpage)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

/// Parse MovieFap XML response to extract video sources
pub(crate) fn parse_moviefap_xml(xml_text: &str) -> Vec<VideoMetadata> {
    let mut video_data = Vec::new();

    for cap in MOVIEFAP_XML_REGEX.captures_iter(xml_text) {
        let quality_str = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("unknown");
        let video_url = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");

        if video_url.is_empty() {
            continue;
        }

        let video_url = video_url.replace("&amp;", "&");
        let height = quality_str.trim_end_matches('p').parse::<u32>().ok();
        let width = height.map(|h| (h * 16) / 9);
        let ext = extract_extension_from_url(&video_url);

        let format_id = if let Some(h) = height {
            format!("http-{h}")
        } else {
            "http-default".into()
        };

        video_data.push((format_id, video_url, ext.to_owned(), height, width));
    }

    video_data
}

// ============================================================================
// Format Building
// ============================================================================

// Common codec strings to avoid repeated allocations
const CODEC_H264: &str = "h264";
const CODEC_AAC: &str = "aac";
const PROTOCOL_HTTPS: &str = "https";

/// Build format list from video metadata and fetch filesizes
pub(crate) async fn build_formats(
    video_data: Vec<VideoMetadata>,
    ctx: &ExtractionContext,
) -> Vec<Format> {
    let mut formats = Vec::new();

    for (format_id, video_url, ext, height, width) in video_data {
        let mut format = Format::new(
            format_id,
            video_url.clone(),
            ext.clone(),
            PROTOCOL_HTTPS.to_owned(),
        );

        format.height = height;
        format.width = width;
        format.format_note = height.map(|h| format!("{h}p"));

        if ext == "mp4" {
            format.vcodec = Some(CODEC_H264.to_owned());
            format.acodec = Some(CODEC_AAC.to_owned());
        }

        // Fetch filesize via HEAD request
        match ctx.http_client.head(&video_url).send().await {
            Ok(response) => {
                debug!(status:? = response.status(), content_length:? = response.content_length(); "HEAD response");

                format.filesize = response.content_length();

                // Fallback: If HEAD didn't give us content-length, try Range request
                if format.filesize.is_none() || format.filesize == Some(0) {
                    debug!("HEAD request returned no size, trying Range request");

                    if let Ok(range_response) = ctx
                        .http_client
                        .get(&video_url)
                        .header("Range", "bytes=0-0")
                        .send()
                        .await
                    {
                        debug!(status:? = range_response.status(); "Range response");

                        if let Some(content_range) = range_response.headers().get("content-range") {
                            if let Ok(range_str) = content_range.to_str() {
                                if let Some(total) = range_str.split('/').nth(1) {
                                    format.filesize = total.parse::<u64>().ok();
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(url:? = video_url; "HEAD request failed: {e}");
            }
        }

        formats.push(format);
    }

    formats
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract file extension from URL path
///
/// Returns a static string reference to avoid allocation.
fn extract_extension_from_url(url: &str) -> &'static str {
    if let Ok(parsed_url) = url::Url::parse(url) {
        if let Some(mut path_segments) = parsed_url.path_segments() {
            if let Some(last_segment) = path_segments.next_back() {
                if let Some(ext_start) = last_segment.rfind('.') {
                    let extension = &last_segment[ext_start + 1..];
                    return match extension {
                        "mp4" => "mp4",
                        "flv" => "flv",
                        "m3u8" => "hls",
                        "webm" => "webm",
                        "mkv" => "mkv",
                        _ => "unknown",
                    };
                }
            }
        }
    }
    "unknown"
}
