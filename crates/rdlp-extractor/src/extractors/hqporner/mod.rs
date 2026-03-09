//! HQPorner extractor module.
//!
//! This module provides extraction support for HQPorner videos, categories,
//! actress listings, and keyword search.
//!
//! # Architecture
//!
//! HQPorner is a two-layer iframe-based site:
//! - `patterns` - URL patterns and regex definitions
//! - `mydaddy` - mydaddy.cc embed resolver (iframe → direct MP4 URLs)
//! - `search` - Search result HTML parsing
//! - `search_patterns` - Search URL builders
//!
//! The extractor fetches the HQPorner page for metadata, extracts the
//! mydaddy.cc iframe URL, then resolves it to direct bigcdn.cc MP4 URLs.
//!
//! # Supported URLs
//!
//! - Videos: `https://hqporner.com/hdporn/81203-full_body_massage.html`
//! - Categories: `https://hqporner.com/category/amateur`
//! - Actresses: `https://hqporner.com/actress/emily-bloom`
//! - Search: `https://hqporner.com/?q=massage`

mod mydaddy;
mod patterns;
mod search;
mod search_patterns;

use std::time::Duration;

use async_trait::async_trait;
use log::warn;
use rdlp_core::{ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result};
use regex::Regex;
use scraper::Html;
use std::sync::LazyLock;

use crate::base::common::{BaseExtractor, MAX_PLAYLIST_SIZE};
use crate::hls::detect_format_sizes;

pub use patterns::HQPORNER_VIDEO_PATTERN;

/// Rate limit between listing/search page fetches (milliseconds).
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Pattern to extract duration from text like "26m 52s" or "1h 6m 39s".
static DURATION_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:(\d+)h\s*)?(\d+)m\s*(\d+)s").expect("Valid duration pattern"));

/// HQPorner extractor.
///
/// Supports single videos, category listings, actress listings, and search.
///
/// # Example
///
/// ```no_run
/// use rdlp_extractor::HQPornerExtractor;
/// use rdlp_core::InfoExtractor;
///
/// let extractor = HQPornerExtractor::new();
/// assert!(extractor.suitable("https://hqporner.com/hdporn/81203-full_body_massage.html"));
/// ```
pub struct HQPornerExtractor;

impl HQPornerExtractor {
    /// Create a new HQPorner extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for HQPornerExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a duration string like "26m 52s" or "1h 6m 39s" into total seconds.
pub(super) fn parse_duration(text: &str) -> Option<f64> {
    DURATION_PATTERN.captures(text).map(|caps| {
        let hours: f64 = caps
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);
        let minutes: f64 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);
        let seconds: f64 = caps
            .get(3)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0.0);
        hours * 3600.0 + minutes * 60.0 + seconds
    })
}

/// Extract the video title from an HQPorner page.
fn extract_title(html: &Html) -> String {
    html.select(&scraper::Selector::parse("h1.main-h1").expect("Valid selector"))
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

/// Extract the video duration from an HQPorner page.
fn extract_duration_from_html(html: &Html) -> Option<f64> {
    html.select(&scraper::Selector::parse("li.icon.fa-clock-o").expect("Valid selector"))
        .next()
        .and_then(|el| {
            let text = el.text().collect::<String>();
            parse_duration(&text)
        })
}

/// Extract the actress name from an HQPorner page.
fn extract_actress(html: &Html) -> Option<String> {
    html.select(&scraper::Selector::parse("a[href^='/actress/']").expect("Valid selector"))
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
}

/// Extract the actress profile URL from an HQPorner page.
fn extract_actress_url(html: &Html) -> Option<String> {
    html.select(&scraper::Selector::parse("a[href^='/actress/']").expect("Valid selector"))
        .next()
        .and_then(|el| el.value().attr("href"))
        .map(|href| format!("https://hqporner.com{href}"))
}

/// Extract category tags from an HQPorner page.
fn extract_categories(html: &Html) -> Vec<String> {
    html.select(
        &scraper::Selector::parse("a.tag-link[href^='/category/']").expect("Valid selector"),
    )
    .map(|el| el.text().collect::<String>().trim().to_string())
    .filter(|s| !s.is_empty())
    .collect()
}

/// Extract the meta description from the raw HTML.
fn extract_description(webpage: &str) -> Option<String> {
    let html = Html::parse_document(webpage);
    html.select(&scraper::Selector::parse("meta[name='description']").expect("Valid selector"))
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.to_string())
}

/// Extract the mydaddy.cc iframe URL from the raw HTML.
fn extract_iframe_url(webpage: &str) -> Option<String> {
    patterns::IFRAME_PATTERN
        .captures(webpage)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

#[async_trait]
impl InfoExtractor for HQPornerExtractor {
    fn name(&self) -> &str {
        "HQPorner"
    }

    fn valid_url(&self) -> &Regex {
        &HQPORNER_VIDEO_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let video_id = patterns::extract_video_id(url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract video ID: {url}")))?;

        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Extract iframe URL before parsing HTML (avoids borrow issues)
        let iframe_url = extract_iframe_url(&webpage).ok_or_else(|| {
            RdlpError::Extraction("No mydaddy.cc iframe found on page".to_string())
        })?;

        let description = extract_description(&webpage);

        // Parse HTML for metadata
        let (title, duration, actress, actress_url, categories) = {
            let html = Html::parse_document(&webpage);
            (
                extract_title(&html),
                extract_duration_from_html(&html),
                extract_actress(&html),
                extract_actress_url(&html),
                extract_categories(&html),
            )
        };

        // Resolve formats from mydaddy.cc embed
        let mydaddy_result = mydaddy::resolve_formats(&iframe_url, ctx).await?;

        if mydaddy_result.formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Detect file sizes
        let extractor_name = InfoExtractor::name(self);
        let (formats_with_size, hls_flags) =
            detect_format_sizes(mydaddy_result.formats, ctx, extractor_name).await;

        let mut info = InfoDict::new(&video_id, &title, extractor_name, url);
        info.description = description;
        info.thumbnail = mydaddy_result.thumbnail;
        info.uploader = actress;
        info.uploader_url = actress_url;
        info.duration = duration;
        info.age_limit = Some(18);
        info.formats = formats_with_size;
        info.tags = if categories.is_empty() {
            None
        } else {
            Some(categories)
        };
        info.propagate_duration();

        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        Ok(info)
    }

    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        if !patterns::is_category_url(url) && !patterns::is_actress_url(url) {
            return Ok(vec![self.extract(url, ctx).await?]);
        }

        // Extract listing pages
        let mut all_results = Vec::new();
        let mut page_url = url.to_string();

        loop {
            let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;

            let video_urls: Vec<String> = patterns::VIDEO_LINK_PATTERN
                .captures_iter(&webpage)
                .filter_map(|c| c.get(1))
                .map(|m| format!("https://hqporner.com{}", m.as_str()))
                .collect();

            if video_urls.is_empty() {
                break;
            }

            for video_url in &video_urls {
                match self.extract(video_url, ctx).await {
                    Ok(info) => all_results.push(info),
                    Err(e) => {
                        warn!("[HQPorner] Failed to extract {video_url}: {e}");
                    }
                }

                if all_results.len() >= MAX_PLAYLIST_SIZE {
                    break;
                }
            }

            if all_results.len() >= MAX_PLAYLIST_SIZE {
                break;
            }

            // Check for "Next" pagination link
            if !webpage.contains("pagi-btn\">Next") {
                break;
            }

            // Build next page URL
            page_url = search_patterns::next_listing_page_url(url, &webpage);
            if page_url.is_empty() {
                break;
            }

            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        Ok(all_results)
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = HQPornerExtractor::new();
        assert_eq!(InfoExtractor::name(&extractor), "HQPorner");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = HQPornerExtractor::new();
        assert!(extractor.suitable("https://hqporner.com/hdporn/81203-full_body_massage.html"));
        assert!(extractor.suitable("https://hqporner.com/category/amateur"));
        assert!(extractor.suitable("https://hqporner.com/actress/emily-bloom"));
        assert!(extractor.suitable("https://hqporner.com/?q=test"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_parse_duration_minutes_seconds() {
        assert_eq!(parse_duration("26m 52s"), Some(1612.0));
    }

    #[test]
    fn test_parse_duration_hours_minutes_seconds() {
        assert_eq!(parse_duration("1h 6m 39s"), Some(3999.0));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert_eq!(parse_duration("invalid"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn test_extract_title_from_fixture() {
        let html_str = r#"<html><body><h1 class="main-h1" style="line-height: 1em;">
full body massage</h1></body></html>"#;
        let html = Html::parse_document(html_str);
        assert_eq!(extract_title(&html), "full body massage");
    }

    #[test]
    fn test_extract_duration_from_fixture() {
        let html_str = r#"<html><body><li class="icon fa-clock-o">26m 52s</li></body></html>"#;
        let html = Html::parse_document(html_str);
        assert_eq!(extract_duration_from_html(&html), Some(1612.0));
    }

    #[test]
    fn test_extract_actress_from_fixture() {
        let html_str = r#"<html><body><a href="/actress/emily-bloom" title="See all Emily Bloom videos" class="click-trigger">Emily Bloom</a></body></html>"#;
        let html = Html::parse_document(html_str);
        assert_eq!(extract_actress(&html), Some("Emily Bloom".to_string()));
    }

    #[test]
    fn test_extract_categories_from_fixture() {
        let html_str = r#"<html><body><a href="/category/1080p-porn" class="tag-link click-trigger">1080p</a><a href="/category/fingering" class="tag-link click-trigger">fingering</a><a href="/category/porn-massage" class="tag-link click-trigger">massage</a></body></html>"#;
        let html = Html::parse_document(html_str);
        let cats = extract_categories(&html);
        assert_eq!(cats, vec!["1080p", "fingering", "massage"]);
    }

    #[test]
    fn test_extract_iframe_url() {
        let html = r#"<iframe width="560" height="350" src="//mydaddy.cc/video/97d0145823aeb8edca/" frameborder="0" allowfullscreen></iframe>"#;
        assert_eq!(
            extract_iframe_url(html),
            Some("//mydaddy.cc/video/97d0145823aeb8edca/".to_string())
        );
    }

    #[test]
    fn test_extract_iframe_url_missing() {
        assert_eq!(extract_iframe_url("<html>no iframe here</html>"), None);
    }
}
