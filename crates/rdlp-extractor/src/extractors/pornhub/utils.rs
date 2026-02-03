//! Utility functions for PornHub extractor
//!
//! Contains helper functions for parsing, validation, and common operations.

use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use regex::Regex;
use scraper::Html;

use super::patterns::FLASHVARS_PATTERN;
use crate::base::common::BaseExtractor;

/// Age verification cookies required by PornHub
const AGE_COOKIES: &[&str] = &[
    "age_verified=1",
    "accessAgeDisclaimerPH=1",
    "accessAgeDisclaimerUK=1",
    "accessPH=1",
];

// Pre-compiled regexes for error detection (avoid repeated compilation)
static REMOVED_VIDEO_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?s)<div[^>]+class=["'](?:[^"']*\s)?(?:removeduserMessageSection|removed)(?:\s[^"']*)?["'][^>]*>(?P<error>.+?)</div>"#,
    )
    .expect("Valid removed video pattern")
});

static NO_VIDEO_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<section[^>]+class=["']noVideo["'][^>]*>(?P<error>.+?)</section>"#)
        .expect("Valid no video pattern")
});

static GEO_BLOCKED_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"class=["']geoBlocked["']"#).expect("Valid geo blocked pattern"));

static LOCKED_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"<[^>]+\bid=["']lockedPlayer"#).expect("Valid locked pattern"));

static HTML_TAG_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<[^>]+>").expect("Valid HTML tag pattern"));

static SHARE_TITLE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"shareTitle\s*[:=]\s*["']([^"']+)["']"#).expect("Valid share title pattern")
});

/// Extract host from URL
///
/// Returns the host portion of the URL (e.g., "de.pornhub.com" from a German URL).
/// If URL parsing fails or host is missing, defaults to "www.pornhub.com".
///
/// # Fallback Behavior
///
/// This function intentionally uses a silent fallback because:
/// - Malformed URLs should still attempt extraction with the default host
/// - If the host is wrong, subsequent HTTP requests will fail with descriptive errors
/// - The default host covers the common case (99%+ of PornHub URLs)
pub fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "www.pornhub.com".to_string())
}

/// Set age verification cookies
pub async fn set_age_cookies(host: &str, ctx: &ExtractionContext) -> Result<()> {
    let base_url = format!("https://{host}");

    for cookie in AGE_COOKIES {
        ctx.cookie_jar
            .add_cookie(&base_url, cookie)
            .await
            .map_err(|e| RdlpError::Extraction(format!("Failed to set age cookie: {e}")))?;
    }

    Ok(())
}

/// Detect if video is unavailable
///
/// Returns error message if video is unavailable, None otherwise
pub fn detect_video_unavailable(webpage: &str) -> Option<String> {
    // Check for removed/flagged videos
    if let Some(caps) = REMOVED_VIDEO_PATTERN.captures(webpage) {
        if let Some(error) = caps.name("error") {
            let cleaned = clean_html(error.as_str());
            return Some(format!("Video unavailable: {cleaned}"));
        }
    }

    // Check for noVideo section
    if let Some(caps) = NO_VIDEO_PATTERN.captures(webpage) {
        if let Some(error) = caps.name("error") {
            let cleaned = clean_html(error.as_str());
            return Some(format!("Video unavailable: {cleaned}"));
        }
    }

    // Check for geo-blocked
    if GEO_BLOCKED_PATTERN.is_match(webpage)
        || webpage.contains("This content is unavailable in your country")
    {
        return Some("Video is geo-blocked in your country".to_string());
    }

    // Check for locked/premium videos
    if LOCKED_PATTERN.is_match(webpage) {
        return Some("Video is locked (premium only or private)".to_string());
    }

    None
}

/// Clean HTML tags from text and truncate
fn clean_html(text: &str) -> String {
    let cleaned = HTML_TAG_PATTERN.replace_all(text.trim(), "");
    cleaned.trim().chars().take(200).collect()
}

/// PornHub base URL for making relative hrefs absolute.
const PORNHUB_BASE_URL: &str = "https://www.pornhub.com";

/// Extract video title from HTML.
///
/// Uses PornHub-specific strategies first (`h1.title`, `shareTitle` JS var),
/// then falls back to `BaseExtractor::extract_title_multi_strategy`.
pub fn extract_title(html: &Html, webpage: &str) -> String {
    // Strategy 1: PornHub-specific h1.title element
    if let Some(title) = BaseExtractor::extract_element_text_str(html, "h1.title") {
        return title;
    }

    // Strategy 2: generic multi-strategy (og:title, twitter:title, <title>, h1)
    if let Some(title) = BaseExtractor::extract_title_multi_strategy(html) {
        return title;
    }

    // Strategy 3: PornHub-specific shareTitle JavaScript variable
    if let Some(caps) = SHARE_TITLE_PATTERN.captures(webpage) {
        if let Some(m) = caps.get(1) {
            let title = m.as_str().trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }

    "Untitled".to_string()
}

/// Extract video description from HTML.
///
/// Delegates to `BaseExtractor::extract_description_multi_strategy`.
pub fn extract_description(html: &Html) -> Option<String> {
    BaseExtractor::extract_description_multi_strategy(html)
}

/// Extract thumbnail URL from flashvars (primary) or HTML meta tags (fallback).
///
/// yt-dlp uses `flashvars.image_url` which is the authoritative CDN URL.
/// Meta tags (`og:image`, `twitter:image`) may use different CDN paths that
/// require different auth tokens or return lower quality images.
pub fn extract_thumbnail(html: &Html, webpage: &str) -> Option<String> {
    // Strategy 1: flashvars.image_url (matches yt-dlp behavior)
    if let Some(url) = extract_flashvar_string(webpage, "image_url") {
        return Some(url);
    }

    // Strategy 2: og:image / twitter:image meta tags
    BaseExtractor::extract_thumbnail_multi_strategy(html)
}

/// Extract video duration from flashvars.
///
/// yt-dlp uses `flashvars.video_duration` for accurate duration.
pub fn extract_duration(webpage: &str) -> Option<f64> {
    extract_flashvar_string(webpage, "video_duration")
        .and_then(|s| s.parse::<f64>().ok())
}

/// Extract a string value from the flashvars JSON object.
fn extract_flashvar_string(webpage: &str, key: &str) -> Option<String> {
    let json_str = FLASHVARS_PATTERN
        .captures(webpage)?
        .get(1)?
        .as_str();
    let flashvars: serde_json::Value = serde_json::from_str(json_str).ok()?;
    flashvars.get(key)?.as_str().map(|s| s.to_string())
}

/// Extract uploader name from HTML
pub fn extract_uploader(html: &Html) -> Option<String> {
    BaseExtractor::extract_element_text_str(html, ".usernameBadgesWrapper a")
        .or_else(|| BaseExtractor::extract_element_text_str(html, ".usernameWrap a"))
        .or_else(|| BaseExtractor::extract_element_text_str(html, ".video-info-row .usernameLink"))
}

/// Extract uploader URL from HTML
pub fn extract_uploader_url(html: &Html) -> Option<String> {
    BaseExtractor::extract_first_href(
        html,
        &[
            ".usernameBadgesWrapper a",
            ".usernameWrap a",
            ".video-info-row .usernameLink",
        ],
        PORNHUB_BASE_URL,
    )
}

/// Extract channel name (may differ from uploader on some videos)
pub fn extract_channel(html: &Html) -> Option<String> {
    BaseExtractor::extract_element_text_str(html, ".video-info-row .channel-name a")
        .or_else(|| BaseExtractor::extract_element_text_str(html, ".channel-link"))
}

/// Extract channel URL
pub fn extract_channel_url(html: &Html) -> Option<String> {
    BaseExtractor::extract_first_href(
        html,
        &[".video-info-row .channel-name a", ".channel-link"],
        PORNHUB_BASE_URL,
    )
}

/// Extract view count from HTML
pub fn extract_view_count(html: &Html) -> Option<u64> {
    BaseExtractor::extract_element_text_str(html, ".count")
        .and_then(|t| parse_view_count(&t))
        .or_else(|| {
            BaseExtractor::extract_element_text_str(html, ".views")
                .and_then(|t| parse_view_count(&t))
        })
}

/// Parse view count string (handles "1.5M", "500K", etc.)
fn parse_view_count(text: &str) -> Option<u64> {
    let text = text.trim().to_uppercase();
    let text = text.replace(',', "").replace(" ", "");

    if text.ends_with('K') {
        let num: f64 = text.trim_end_matches('K').parse().ok()?;
        Some((num * 1000.0) as u64)
    } else if text.ends_with('M') {
        let num: f64 = text.trim_end_matches('M').parse().ok()?;
        Some((num * 1_000_000.0) as u64)
    } else if text.ends_with('B') {
        let num: f64 = text.trim_end_matches('B').parse().ok()?;
        Some((num * 1_000_000_000.0) as u64)
    } else {
        text.parse().ok()
    }
}

/// Extract rating percentage from HTML
pub fn extract_rating(html: &Html) -> Option<f64> {
    BaseExtractor::extract_element_text_str(html, ".percent")
        .or_else(|| BaseExtractor::extract_element_text_str(html, ".rating-value"))
        .and_then(|text| text.trim().trim_end_matches('%').parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://www.pornhub.com/view_video.php?viewkey=ph123"),
            "www.pornhub.com"
        );
        assert_eq!(
            extract_host("https://de.pornhub.com/view_video.php?viewkey=ph123"),
            "de.pornhub.com"
        );
    }

    #[test]
    fn test_detect_video_unavailable() {
        // Removed video
        let html = r#"<div class="removed userMessageSection"><p>Video removed</p></div>"#;
        assert!(detect_video_unavailable(html).is_some());

        // Geo-blocked
        let html = r#"<div class="geoBlocked">Content blocked</div>"#;
        assert!(
            detect_video_unavailable(html)
                .unwrap()
                .contains("geo-blocked")
        );

        // Normal video
        let html = r#"<div class="video">Normal content</div>"#;
        assert!(detect_video_unavailable(html).is_none());
    }
}
