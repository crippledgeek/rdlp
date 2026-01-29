//! RedTube extractor
//!
//! Supports URLs like:
//! - https://www.redtube.com/123456
//! - https://www.redtube.com.br/123456
//! - https://embed.redtube.com/?id=123456
//!
//! RedTube embeds video sources in JavaScript objects rather than HTML `<source>` tags,
//! so this extractor uses regex to extract JSON from the page source.
//!
//! ## Module Structure
//!
//! - `patterns` - URL and extraction regex patterns
//! - `formats` - Format extraction from JavaScript sources and mediaDefinition

mod formats;
mod patterns;

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result};
use regex::Regex;
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::base::tnaflix_network::TnaFlixNetworkBase;
use crate::hls::detect_format_sizes;
use crate::utils::make_absolute_url;
use patterns::REDTUBE_URL_PATTERN;

/// RedTube extractor
pub struct RedTubeExtractor {
    base: TnaFlixNetworkBase,
}

impl RedTubeExtractor {
    /// Create a new RedTube extractor
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Extract video ID from URL using BaseExtractor utility
    fn extract_id(&self, url: &str) -> Option<String> {
        BaseExtractor::extract_id_from_url(url, &REDTUBE_URL_PATTERN, "id")
    }
}

impl Default for RedTubeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for RedTubeExtractor {
    fn name(&self) -> &str {
        "RedTube"
    }

    fn valid_url(&self) -> &Regex {
        &REDTUBE_URL_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Fetch the webpage using BaseExtractor (handles errors, verbose logging)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Get video ID using BaseExtractor
        let video_id = self.extract_id(url).ok_or_else(|| {
            RdlpError::Extraction(format!("Could not extract video ID from URL: {url}"))
        })?;

        // Extract all data from HTML before any async operations
        let metadata = {
            let html = Html::parse_document(&webpage);
            self.base.extract_metadata(&html)?
        }; // html is dropped here

        // Try to extract video formats from JavaScript sources
        let mut formats = formats::extract_from_sources(&webpage);

        // If sources didn't work, try mediaDefinition
        if formats.is_empty() {
            formats = formats::extract_from_media_definition(&webpage, ctx).await;
        }

        // If both JavaScript methods failed, fall back to HTML <source> tags
        if formats.is_empty() {
            let video_data = {
                let html = Html::parse_document(&webpage);
                self.base.parse_video_sources(&html)
            };

            if !video_data.is_empty() {
                formats = self.base.build_formats(video_data, ctx).await;
            }
        }

        // Return error if still no sources found
        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video sources found in JavaScript or HTML. Video may be unavailable. URL: {url}"
            )));
        }

        // Convert relative URLs to absolute using utility
        for format in &mut formats {
            if !format.url.starts_with("http://") && !format.url.starts_with("https://") {
                format.url = make_absolute_url(url, &format.url);
            }
        }

        // Fetch sizes/segments for all formats in parallel
        let (formats, hls_flags) = detect_format_sizes(formats, ctx, self.name()).await;

        // Build InfoDict with all extracted metadata
        let mut info = InfoDict::new(
            video_id,
            metadata.title,
            self.name().to_string(),
            url.to_string(),
        );
        info.description = metadata.description;
        info.uploader = metadata.uploader;
        info.uploader_id = metadata.uploader_id;
        info.uploader_url = metadata.uploader_url;
        info.channel = metadata.channel;
        info.channel_id = metadata.channel_id;
        info.channel_url = metadata.channel_url;
        info.thumbnail = metadata.thumbnail;
        info.thumbnails = metadata.thumbnails;
        info.duration = metadata.duration;
        info.upload_date = metadata.upload_date;
        info.view_count = metadata.view_count;
        info.like_count = metadata.like_count;
        info.average_rating = metadata.average_rating;
        info.tags = metadata.tags;
        info.categories = metadata.categories;
        info.age_limit = Some(18); // RedTube is adult content
        info.formats = formats;

        // Set stream-level flags from HLS detection
        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        Ok(info)
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use once_cell::sync::Lazy;

    /// Shared test fixture (compiled once, reused across all tests)
    static TEST_REDTUBE: Lazy<RedTubeExtractor> = Lazy::new(RedTubeExtractor::new);

    #[test]
    fn test_redtube_url_pattern() {
        let extractor = &*TEST_REDTUBE;
        assert!(extractor.suitable("https://www.redtube.com/123456"));
        assert!(extractor.suitable("https://redtube.com/12345678"));
        assert!(extractor.suitable("https://www.redtube.com.br/987654"));
        assert!(extractor.suitable("https://embed.redtube.com/?id=123456"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://www.tnaflix.com/video/123"));
    }

    #[test]
    fn test_extract_id() {
        let extractor = &*TEST_REDTUBE;

        let id1 = extractor.extract_id("https://www.redtube.com/123456");
        assert_eq!(id1, Some("123456".to_string()));

        let id2 = extractor.extract_id("https://redtube.com/12345678");
        assert_eq!(id2, Some("12345678".to_string()));

        let id3 = extractor.extract_id("https://www.redtube.com.br/987654");
        assert_eq!(id3, Some("987654".to_string()));

        let id4 = extractor.extract_id("https://embed.redtube.com/?id=555555");
        assert_eq!(id4, Some("555555".to_string()));
    }

    #[test]
    fn test_extractor_name() {
        let extractor = &*TEST_REDTUBE;
        assert_eq!(extractor.name(), "RedTube");
    }

    #[test]
    fn test_extractor_priority() {
        let extractor = &*TEST_REDTUBE;
        assert_eq!(extractor.priority(), 0);
    }
}
