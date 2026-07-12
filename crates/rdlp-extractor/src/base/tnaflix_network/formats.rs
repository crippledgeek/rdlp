//! Format building and video source parsing for TNAFlix network sites
//!
//! Provides functions for extracting video source URLs from HTML and
//! building Format objects with filesize detection.

use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_core::ExtractionContext;
use rdlp_types::Codec;
use rdlp_types::Format;
use scraper::{Html, Selector};
use std::sync::LazyLock;

use crate::base::common::BaseExtractor;

/// Video metadata extracted from HTML: (format_id, video_url, ext, height, width)
pub(crate) type VideoMetadata = (String, String, String, Option<u32>, Option<u32>);

// ============================================================================
// Static Selectors and Patterns
// ============================================================================

/// Selector for video source tags: <source src="..." type="video/mp4">
pub(crate) static SOURCE_SELECTOR: LazyLock<Selector> =
    crate::static_selector!("source[src][type='video/mp4']");

/// Regex to extract CDN URL from MovieFap JavaScript
pub(crate) static CDN_URL_REGEX: Lazy<Regex> =
    lazy_regex!(r#"url:\s*['"]([^'"]+/cdn\.php[^'"]+)['"]"#);

/// Regex to extract video items from MovieFap XML
pub(crate) static MOVIEFAP_XML_REGEX: Lazy<Regex> =
    lazy_regex!(r"(?s)<item>.*?<res>([^<]+)</res>.*?<videoLink>([^<]+)</videoLink>.*?</item>");

/// Regex patterns for extracting config URLs (multiple fallback strategies)
#[allow(dead_code)] // Used by extract_config_url which is tested
pub(crate) static CONFIG_URL_PATTERNS: [Lazy<Regex>; 3] = [
    lazy_regex!(r#"flashvars\.config\s*=\s*escape\("([^"]+)""#),
    lazy_regex!(r#"<input[^>]+name="config\d?"[^>]+value="([^"]+)""#),
    lazy_regex!(r#"config\s*=\s*["']([^"']+)["']"#),
];

// ============================================================================
// Video Source Parsing
// ============================================================================

/// Parse video source tags from HTML and extract format metadata
///
/// Looks for: `<source src="..." type="video/mp4" size="720">`
pub(crate) fn parse_video_sources(html: &Html) -> Vec<VideoMetadata> {
    html.select(&SOURCE_SELECTOR)
        .filter_map(|source_elem| {
            let video_url = source_elem.value().attr("src")?;

            let quality_str = source_elem.value().attr("size").unwrap_or("unknown");
            let height = quality_str.parse::<u32>().ok();
            let width = height.map(|h| (h * 16) / 9);
            let ext = extract_extension_from_url(video_url);

            let format_id = if quality_str != "unknown" {
                format!("http-{quality_str}")
            } else {
                "http-default".into()
            };

            Some((
                format_id,
                video_url.to_owned(),
                ext.to_owned(),
                height,
                width,
            ))
        })
        .collect()
}

/// Extract config URL from HTML using multiple fallback patterns
#[allow(dead_code)] // Tested but not used by current extractors
pub(crate) fn extract_config_url(html_text: &str) -> Option<String> {
    CONFIG_URL_PATTERNS.iter().find_map(|pattern| {
        pattern
            .captures(html_text)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    })
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
    MOVIEFAP_XML_REGEX
        .captures_iter(xml_text)
        .filter_map(|cap| {
            let quality_str = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("unknown");
            let video_url = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");

            if video_url.is_empty() {
                return None;
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

            Some((format_id, video_url, ext.to_owned(), height, width))
        })
        .collect()
}

// ============================================================================
// Format Building
// ============================================================================

// Common codec strings to avoid repeated allocations
const CODEC_H264: &str = "h264";
const CODEC_AAC: &str = "aac";
use rdlp_types::DownloadProtocol;

/// Build format list from video metadata and fetch filesizes.
///
/// Filesize detection uses `detect_format_sizes` (the shared HLS module
/// helper) which runs all HEAD requests in parallel via `join_all`.
pub(crate) async fn build_formats(
    video_data: Vec<VideoMetadata>,
    ctx: &ExtractionContext,
) -> Vec<Format> {
    // Build format structs (sync, no I/O)
    let formats: Vec<Format> = video_data
        .into_iter()
        .map(|(format_id, video_url, ext, height, width)| {
            let mut format = Format::new(&format_id, &video_url, &ext, DownloadProtocol::Https);
            format.height = height;
            format.width = width;
            format.format_note = height.map(|h| format!("{h}p"));
            if ext == "mp4" {
                format.vcodec = Codec::from(CODEC_H264.to_owned());
                format.acodec = Codec::from(CODEC_AAC.to_owned());
            }
            format
        })
        .collect();

    // Parallel filesize detection — reuses the shared HLS module helper
    // which runs HEAD requests concurrently. HLS flags are discarded
    // since TNAFlix network formats are all direct HTTPS.
    let (mut formats, _hls_flags) = crate::hls::detect_format_sizes(formats, ctx, "TNAFlix").await;

    BaseExtractor::dedup_format_ids(&mut formats);
    formats
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract file extension from URL path
///
/// Returns a static string reference to avoid allocation.
fn extract_extension_from_url(url: &str) -> &'static str {
    if let Ok(parsed_url) = url::Url::parse(url)
        && let Some(mut path_segments) = parsed_url.path_segments()
        && let Some(last_segment) = path_segments.next_back()
        && let Some(ext_start) = last_segment.rfind('.')
    {
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
    "unknown"
}
