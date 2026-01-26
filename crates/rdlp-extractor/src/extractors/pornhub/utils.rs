//! Utility functions for PornHub extractor
//!
//! Contains helper functions for parsing, validation, and common operations.

use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use regex::Regex;
use scraper::{Html, Selector};

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

/// Extract video title from HTML
pub fn extract_title(html: &Html, webpage: &str) -> String {
    // Strategy 1: twitter:title meta tag
    if let Some(title) = extract_meta_content(html, "meta[name='twitter:title']") {
        return title;
    }

    // Strategy 2: h1.title element
    if let Some(title) = extract_element_text(html, "h1.title") {
        return title;
    }

    // Strategy 3: shareTitle JavaScript variable
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

/// Extract video description from HTML
pub fn extract_description(html: &Html) -> Option<String> {
    extract_meta_content(html, "meta[property='og:description']")
        .or_else(|| extract_meta_content(html, "meta[name='description']"))
}

/// Extract thumbnail URL from HTML
pub fn extract_thumbnail(html: &Html) -> Option<String> {
    extract_meta_content(html, "meta[property='og:image']")
        .or_else(|| extract_meta_content(html, "meta[name='twitter:image']"))
}

/// Extract uploader name from HTML
pub fn extract_uploader(html: &Html) -> Option<String> {
    // Try multiple selectors for uploader
    extract_element_text(html, ".usernameBadgesWrapper a")
        .or_else(|| extract_element_text(html, ".usernameWrap a"))
        .or_else(|| extract_element_text(html, ".video-info-row .usernameLink"))
}

/// Extract uploader URL from HTML
pub fn extract_uploader_url(html: &Html) -> Option<String> {
    let selectors = [
        ".usernameBadgesWrapper a",
        ".usernameWrap a",
        ".video-info-row .usernameLink",
    ];

    for selector_str in selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = html.select(&selector).next() {
                if let Some(href) = element.value().attr("href") {
                    if !href.is_empty() {
                        // Make absolute if relative
                        if href.starts_with('/') {
                            return Some(format!("https://www.pornhub.com{href}"));
                        }
                        return Some(href.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Extract channel name (may differ from uploader on some videos)
pub fn extract_channel(html: &Html) -> Option<String> {
    extract_element_text(html, ".video-info-row .channel-name a")
        .or_else(|| extract_element_text(html, ".channel-link"))
}

/// Extract channel URL
pub fn extract_channel_url(html: &Html) -> Option<String> {
    let selectors = [".video-info-row .channel-name a", ".channel-link"];

    for selector_str in selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = html.select(&selector).next() {
                if let Some(href) = element.value().attr("href") {
                    if !href.is_empty() {
                        if href.starts_with('/') {
                            return Some(format!("https://www.pornhub.com{href}"));
                        }
                        return Some(href.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Extract view count from HTML
pub fn extract_view_count(html: &Html) -> Option<u64> {
    // Try to find view count element
    if let Some(text) = extract_element_text(html, ".count") {
        return parse_view_count(&text);
    }
    if let Some(text) = extract_element_text(html, ".views") {
        return parse_view_count(&text);
    }
    None
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
    // Try percent element
    if let Some(text) = extract_element_text(html, ".percent") {
        let text = text.trim().trim_end_matches('%');
        return text.parse().ok();
    }
    // Try rating-value element
    if let Some(text) = extract_element_text(html, ".rating-value") {
        let text = text.trim().trim_end_matches('%');
        return text.parse().ok();
    }
    None
}

/// Extract content from meta tag
fn extract_meta_content(html: &Html, selector_str: &str) -> Option<String> {
    let selector = Selector::parse(selector_str).ok()?;
    let element = html.select(&selector).next()?;
    let content = element.value().attr("content")?.trim();

    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

/// Extract text from element
fn extract_element_text(html: &Html, selector_str: &str) -> Option<String> {
    let selector = Selector::parse(selector_str).ok()?;
    let element = html.select(&selector).next()?;
    let text: String = element.text().collect();
    let text = text.trim();

    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Build width from height assuming 16:9 aspect ratio
pub fn width_from_height(height: u32) -> u32 {
    match height {
        240 => 426,
        360 => 640,
        480 => 854,
        720 => 1280,
        1080 => 1920,
        1440 => 2560,
        2160 => 3840,
        _ => (height as f32 * 16.0 / 9.0) as u32,
    }
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

    #[test]
    fn test_width_from_height() {
        assert_eq!(width_from_height(1080), 1920);
        assert_eq!(width_from_height(720), 1280);
        assert_eq!(width_from_height(480), 854);
    }
}
