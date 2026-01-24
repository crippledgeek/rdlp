//! PornHub extractor module
//!
//! This module provides extraction support for PornHub videos and playlists.
//!
//! # Architecture
//!
//! The extractor is split into focused submodules:
//! - `patterns` - URL patterns and regex definitions
//! - `formats` - Format extraction from various sources
//! - `playlist` - Playlist pagination and extraction
//! - `utils` - Helper functions for parsing and validation
//!
//! # Supported URLs
//!
//! - Videos: `https://www.pornhub.com/view_video.php?viewkey=ph123`
//! - Playlists: `https://www.pornhub.com/playlist/123456`
//! - Embed: `https://www.pornhub.com/embed/ph123`
//! - Thumbzilla: `https://www.thumbzilla.com/video/ph123/title`

mod formats;
mod patterns;
mod playlist;
mod utils;

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result};
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::hls::detect_format_sizes;

pub use patterns::{PORNHUB_PLAYLIST_URL_PATTERN, PORNHUB_VIDEO_URL_PATTERN};

/// PornHub extractor
///
/// Supports:
/// - Single videos: `https://www.pornhub.com/view_video.php?viewkey=ph123`
/// - Playlists: `https://www.pornhub.com/playlist/123456`
///
/// # Example
///
/// ```no_run
/// use rdlp_extractor::PornHubExtractor;
/// use rdlp_core::InfoExtractor;
///
/// let extractor = PornHubExtractor::new();
/// assert!(extractor.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
/// ```
pub struct PornHubExtractor;

impl PornHubExtractor {
    /// Create a new PornHub extractor
    pub fn new() -> Self {
        Self
    }
}

impl Default for PornHubExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for PornHubExtractor {
    fn name(&self) -> &str {
        "PornHub"
    }

    fn valid_url(&self) -> &regex::Regex {
        &PORNHUB_VIDEO_URL_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let host = utils::extract_host(url);

        // Set age verification cookies
        utils::set_age_cookies(&host, ctx).await?;

        // Fetch the webpage using BaseExtractor (handles errors, verbose logging)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Check for video unavailability errors
        if let Some(error_msg) = utils::detect_video_unavailable(&webpage) {
            return Err(RdlpError::Extraction(error_msg));
        }

        // Get video ID
        let video_id = patterns::extract_video_id(url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract video ID: {url}")))?;

        // Extract title
        let title = {
            let html = Html::parse_document(&webpage);
            utils::extract_title(&html, &webpage)
        };

        // Extract formats with fallback strategies
        let formats = formats::extract_all_formats(&webpage, ctx).await?;

        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Detect file sizes and segment counts
        let formats_with_size = detect_format_sizes(formats, ctx, self.name()).await;

        // Build InfoDict
        let mut info = InfoDict::new(video_id, title, self.name().to_string(), url.to_string());
        info.age_limit = Some(18);
        info.formats = formats_with_size;

        Ok(info)
    }

    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        if !patterns::is_playlist_url(url) {
            return Ok(vec![self.extract(url, ctx).await?]);
        }

        playlist::extract_playlist(self, url, ctx).await
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
        let extractor = PornHubExtractor::new();
        assert_eq!(extractor.name(), "PornHub");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = PornHubExtractor::new();

        // Video URLs
        assert!(extractor.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
        assert!(extractor.suitable("https://www.pornhub.com/embed/ph456"));
        assert!(extractor.suitable("https://de.pornhub.com/view_video.php?viewkey=ph789"));

        // Playlist URLs
        assert!(extractor.suitable("https://www.pornhub.com/playlist/123456"));

        // Invalid URLs
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }
}
