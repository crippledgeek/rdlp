//! mydaddy.cc embed resolver.
//!
//! Fetches the mydaddy.cc embed page (with Referer header) and extracts
//! direct MP4 URLs from the inline JavaScript. No JS execution needed —
//! URLs are static string literals in the HTML.

use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use rdlp_types::Format;
use regex::Regex;
use std::sync::LazyLock;

use crate::base::common::BaseExtractor;

/// Pattern to extract MP4 source URLs from the embed page.
///
/// Matches: `//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/1080.mp4`
static CDN_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(//s\d+\.bigcdn\.cc/pubs/[^"'\\]+\.mp4)"#).expect("Valid CDN URL pattern")
});

/// Pattern to extract the poster/thumbnail URL.
static POSTER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(//s\d+\.bigcdn\.cc/pubs/[^"'\\]+/main\.jpg)"#).expect("Valid poster URL pattern")
});

/// Pattern to extract quality from CDN filename (e.g., "1080" from "/1080.mp4").
static QUALITY_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(\d+)\.mp4$").expect("Valid quality pattern"));

/// Resolved formats and metadata from a mydaddy.cc embed.
pub(crate) struct MyDaddyResult {
    pub formats: Vec<Format>,
    pub thumbnail: Option<String>,
}

/// Resolve a mydaddy.cc embed URL to direct video format URLs.
///
/// Fetches the embed page with a `Referer: hqporner.com` header and parses
/// the inline JS for bigcdn.cc MP4 URLs.
///
/// # Arguments
/// * `embed_url` - The mydaddy.cc iframe URL (e.g., `//mydaddy.cc/video/{hash}/`)
/// * `ctx` - Extraction context with HTTP client
///
/// # Returns
/// Resolved formats and optional thumbnail URL.
pub(crate) async fn resolve_formats(
    embed_url: &str,
    ctx: &ExtractionContext,
) -> Result<MyDaddyResult> {
    let full_url = if embed_url.starts_with("//") {
        format!("https:{embed_url}")
    } else {
        embed_url.to_string()
    };

    let sanitized = rdlp_security::sanitize_for_logging(&full_url);
    debug!("[HQPorner] Resolving mydaddy.cc embed: {sanitized}");

    // Build alt URL upfront for fallback on both fetch failure and empty formats
    let alt_url = if full_url.ends_with('/') {
        format!("{full_url}&alt")
    } else {
        format!("{full_url}/&alt")
    };

    // Try primary embed, fall back to alt on fetch failure
    let html = match fetch_embed(&full_url, ctx).await {
        Ok(h) => h,
        Err(e) => {
            debug!(
                "[HQPorner] Primary embed fetch failed ({e}), trying alt player: {}",
                rdlp_security::sanitize_for_logging(&alt_url)
            );
            return resolve_from_html(&fetch_embed(&alt_url, ctx).await?);
        }
    };

    // Check for blocked response
    if html.contains("This domain has been blocked") {
        return Err(RdlpError::Extraction {
            message: "mydaddy.cc embed blocked — Referer header may not have been accepted"
                .to_string(),
            url: Some(full_url),
        });
    }

    let formats = parse_formats(&html);

    if formats.is_empty() {
        debug!(
            "[HQPorner] No formats found, trying alt player: {}",
            rdlp_security::sanitize_for_logging(&alt_url)
        );

        let alt_html = fetch_embed(&alt_url, ctx).await?;
        return resolve_from_html(&alt_html);
    }

    let thumbnail = extract_thumbnail(&html);

    Ok(MyDaddyResult { formats, thumbnail })
}

/// Parse formats and thumbnail from already-fetched embed HTML.
fn resolve_from_html(html: &str) -> Result<MyDaddyResult> {
    let formats = parse_formats(html);

    if formats.is_empty() {
        return Err(RdlpError::Extraction {
            message: "No video formats found in mydaddy.cc embed or alt player".to_string(),
            url: None,
        });
    }

    let thumbnail = extract_thumbnail(html);
    Ok(MyDaddyResult { formats, thumbnail })
}

/// Fetch the embed page with the required Referer header.
async fn fetch_embed(url: &str, ctx: &ExtractionContext) -> Result<String> {
    let response = ctx
        .http_client
        .get(url)
        .header("Referer", "https://hqporner.com/")
        .send()
        .await
        .map_err(|e| RdlpError::Network {
            message: format!("Failed to fetch mydaddy.cc embed: {e}"),
            url: Some(url.to_string()),
        })?;

    rdlp_core::check_http_response(&response)?;

    response.text().await.map_err(|e| RdlpError::Network {
        message: format!("Failed to read mydaddy.cc response: {e}"),
        url: Some(url.to_string()),
    })
}

/// Parse MP4 format URLs from the embed HTML.
fn parse_formats(html: &str) -> Vec<Format> {
    let mut seen = std::collections::HashSet::new();
    let mut formats = Vec::new();

    for caps in CDN_URL_PATTERN.captures_iter(html) {
        let url = caps.get(1).unwrap().as_str();
        if !seen.insert(url.to_string()) {
            continue;
        }

        let full_url = format!("https:{url}");
        let quality = QUALITY_PATTERN
            .captures(url)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok());

        let format_id = quality
            .map(|q| format!("{q}p"))
            .unwrap_or_else(|| "unknown".to_string());

        let format = BaseExtractor::build_format(format_id, full_url, "mp4", quality);

        formats.push(format);
    }

    // Sort by quality ascending (360, 720, 1080)
    formats.sort_by_key(|f| f.height.unwrap_or(0));

    formats
}

/// Extract the poster thumbnail URL from embed HTML.
fn extract_thumbnail(html: &str) -> Option<String> {
    POSTER_PATTERN
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| format!("https:{}", m.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample mydaddy.cc embed HTML (non-adblock path, 3 qualities).
    fn sample_embed_html() -> &'static str {
        r##"<script>function do_pl(){ console.log("ab:"+hasAdblock);if(!oldIE&&hasMP4Video){if(hasAdblock){$("#jw").html("<video id=\"flvv\" preload=\"none\" poster=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/main.jpg\" controls style=\"width:100%; height:100%;\" src=\"\"><source src=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/360.mp4\" title=\"360p\" type=\"video/mp4\" /></video>"); }else{ $("#jw").html("<video id=\"flvv\" preload=\"none\" poster=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/main.jpg\" controls style=\"width:100%; height:100%;\" src=\"\"><source src=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/360.mp4\" title=\"360p\" type=\"video/mp4\" /><source src=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/720.mp4\" title=\"720p HD\" type=\"video/mp4\" /><source src=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/1080.mp4\" title=\"1080p Full HD\" type=\"video/mp4\" /></video>");} }}</script>"##
    }

    /// Sample mydaddy.cc embed HTML (adblock path, 360p only).
    fn sample_embed_adblock_html() -> &'static str {
        r##"<script>function do_pl(){ if(hasAdblock){$("#jw").html("<video id=\"flvv\" poster=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/main.jpg\" src=\"\"><source src=\"//s57.bigcdn.cc/pubs/69af3debebaf77.86372927/360.mp4\" title=\"360p\" type=\"video/mp4\" /></video>"); }}</script>"##
    }

    #[test]
    fn test_parse_formats_three_qualities() {
        let formats = parse_formats(sample_embed_html());
        assert_eq!(formats.len(), 3);
        assert_eq!(formats[0].format_id, "360p");
        assert_eq!(formats[0].height, Some(360));
        assert!(formats[0].url.contains("360.mp4"));
        assert_eq!(formats[1].format_id, "720p");
        assert_eq!(formats[1].height, Some(720));
        assert_eq!(formats[2].format_id, "1080p");
        assert_eq!(formats[2].height, Some(1080));
    }

    #[test]
    fn test_parse_formats_adblock_single_quality() {
        let formats = parse_formats(sample_embed_adblock_html());
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format_id, "360p");
    }

    #[test]
    fn test_parse_formats_deduplicates() {
        // The embed HTML contains each URL twice (adblock + non-adblock branch)
        let formats = parse_formats(sample_embed_html());
        assert_eq!(formats.len(), 3); // Not 6
    }

    #[test]
    fn test_parse_formats_empty_html() {
        let formats = parse_formats("<html></html>");
        assert!(formats.is_empty());
    }

    #[test]
    fn test_extract_thumbnail() {
        let thumb = extract_thumbnail(sample_embed_html());
        assert_eq!(
            thumb,
            Some("https://s57.bigcdn.cc/pubs/69af3debebaf77.86372927/main.jpg".to_string())
        );
    }

    #[test]
    fn test_extract_thumbnail_missing() {
        assert_eq!(extract_thumbnail("<html></html>"), None);
    }

    #[test]
    fn test_formats_urls_are_https() {
        let formats = parse_formats(sample_embed_html());
        for f in &formats {
            assert!(
                f.url.starts_with("https://"),
                "URL should be https: {}",
                f.url
            );
        }
    }

    #[test]
    fn test_formats_sorted_by_quality() {
        let formats = parse_formats(sample_embed_html());
        let heights: Vec<u32> = formats.iter().filter_map(|f| f.height).collect();
        assert_eq!(heights, vec![360, 720, 1080]);
    }
}
