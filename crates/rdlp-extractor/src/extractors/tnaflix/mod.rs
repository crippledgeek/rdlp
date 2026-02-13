//! TNAFlix network extractor
//!
//! Supports sites in the TNAFlix network:
//! - TNAFlix: `https://www.tnaflix.com/category/title/video123456`
//! - EMPFlix: `https://www.empflix.com/videos/title-123`
//! - MovieFap: `https://www.moviefap.com/videos/abc123/title.html`
//!
//! ## Module Structure
//!
//! - `patterns` - URL regex patterns for each site
//! - `ajax` - AJAX/XML data fetching for EMPFlix and MovieFap

mod ajax;
mod patterns;

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result};
use regex::Regex;
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::base::tnaflix_network::TnaFlixNetworkBase;
use patterns::{EMPFLIX_URL_PATTERN, MOVIEFAP_URL_PATTERN, TNAFLIX_URL_PATTERN};

/// TNAFlix network extractor (supports TNAFlix, EMPFlix, MovieFap)
///
/// Uses [`TnaFlixNetworkBase`] for shared extraction logic.
pub struct TNAFlixExtractor {
    name: &'static str,
    url_pattern: &'static Regex,
    base: TnaFlixNetworkBase,
}

impl TNAFlixExtractor {
    /// Create extractor for TNAFlix
    #[must_use]
    pub fn tnaflix() -> Self {
        Self {
            name: "TNAFlix",
            url_pattern: &TNAFLIX_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Create extractor for EMPFlix
    #[must_use]
    pub fn empflix() -> Self {
        Self {
            name: "EMPFlix",
            url_pattern: &EMPFLIX_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Create extractor for MovieFap
    #[must_use]
    pub fn moviefap() -> Self {
        Self {
            name: "MovieFap",
            url_pattern: &MOVIEFAP_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Extract video ID from URL using BaseExtractor utility
    fn extract_id(&self, url: &str) -> Option<String> {
        // Try each capture group in order (different URL patterns)
        BaseExtractor::extract_id_positional(url, self.url_pattern, &[1, 2, 3])
    }
}

#[async_trait]
impl InfoExtractor for TNAFlixExtractor {
    fn name(&self) -> &str {
        self.name
    }

    fn valid_url(&self) -> &Regex {
        self.url_pattern
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Fetch the webpage using BaseExtractor (handles errors, verbose logging)
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Get video ID using BaseExtractor
        let video_id = self.extract_id(url).ok_or_else(|| {
            RdlpError::Extraction(format!("Could not extract video ID from URL: {url}"))
        })?;

        // Check if this is MovieFap (uses different video loading mechanism)
        let is_moviefap = url.contains("moviefap.com");

        // Extract all data from HTML before any async operations
        let (metadata, cdn_url_opt) = {
            let html = Html::parse_document(&webpage);

            // Extract metadata using base (includes title, description, uploader, thumbnail, and enhanced JSON-LD fields)
            let metadata = self.base.extract_metadata(&html)?;

            // For MovieFap, extract cdn.php URL using base
            let cdn_url_opt = if is_moviefap {
                self.base.extract_cdn_url(&webpage)
            } else {
                None
            };

            (metadata, cdn_url_opt)
        }; // html is dropped here

        // Parse video data based on site type
        let video_data = if is_moviefap {
            // MovieFap: fetch XML from cdn.php
            let cdn_url = cdn_url_opt.ok_or_else(|| {
                RdlpError::Extraction(format!(
                    "Could not find cdn.php URL in MovieFap page: {url}"
                ))
            })?;

            BaseExtractor::log_if_verbose(ctx, "MovieFap", &format!("cdn.php URL: {cdn_url}"));

            ajax::parse_moviefap_xml(&self.base, &cdn_url, ctx).await?
        } else {
            // TNAFlix/EMPFlix: try HTML <source> tags first, fallback to AJAX
            let video_data = {
                let html = Html::parse_document(&webpage);
                self.base.parse_video_sources(&html)
            }; // html is dropped here

            // EMPFlix fallback: if no sources found, try AJAX endpoint
            let video_data = if video_data.is_empty() && url.contains("empflix.com") {
                BaseExtractor::log_if_verbose(
                    ctx,
                    "EMPFlix",
                    "No sources in HTML, trying AJAX endpoint...",
                );
                ajax::parse_empflix_ajax(&self.base, &video_id, url, ctx).await?
            } else {
                video_data
            };

            // Return error if still no sources found
            if video_data.is_empty() {
                return Err(RdlpError::Extraction(format!(
                    "No video source tags found in HTML. Video may be unavailable. URL: {url}"
                )));
            }

            video_data
        };

        // Build formats and fetch filesizes using base (asynchronous)
        let formats = self.base.build_formats(video_data, ctx).await;

        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Build InfoDict with all extracted metadata
        let mut info = InfoDict::new(video_id, metadata.title, self.name, url);
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
        info.formats = formats;

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

    /// Shared test fixtures (compiled once, reused across all tests)
    ///
    /// Performance: Prevents unnecessary regex compilation in tests:
    /// - Without lazy: ~50μs × 5 test instances = 250μs wasted
    /// - With lazy: ~0.01μs access after first initialization
    static TEST_TNAFLIX: Lazy<TNAFlixExtractor> = Lazy::new(TNAFlixExtractor::tnaflix);
    static TEST_EMPFLIX: Lazy<TNAFlixExtractor> = Lazy::new(TNAFlixExtractor::empflix);
    static TEST_MOVIEFAP: Lazy<TNAFlixExtractor> = Lazy::new(TNAFlixExtractor::moviefap);

    #[test]
    fn test_tnaflix_url_suitable() {
        let extractor = &*TEST_TNAFLIX;
        assert!(extractor.suitable("https://www.tnaflix.com/hd-videos/test/video123456"));
        assert!(extractor.suitable("https://tnaflix.com/amateur-porn/title/video999"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_empflix_url_suitable() {
        let extractor = &*TEST_EMPFLIX;
        assert!(extractor.suitable("https://www.empflix.com/videos/title-123"));
        assert!(extractor.suitable("https://empflix.com/view/123"));
        assert!(extractor.suitable(
            "https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093"
        ));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_empflix_extract_id() {
        let extractor = &*TEST_EMPFLIX;

        // Test /videos/title-ID format
        let id1 = extractor.extract_id("https://www.empflix.com/videos/title-123");
        assert_eq!(id1, Some("123".to_string()));

        // Test /category/ID format
        let id2 = extractor.extract_id("https://empflix.com/view/456");
        assert_eq!(id2, Some("456".to_string()));

        // Test /category/title/videoID format
        let id3 = extractor.extract_id(
            "https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093",
        );
        assert_eq!(id3, Some("3715093".to_string()));
    }

    #[test]
    fn test_moviefap_url_suitable() {
        let extractor = &*TEST_MOVIEFAP;
        assert!(extractor.suitable("https://www.moviefap.com/videos/abc123def/title.html"));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_tnaflix_extract_id() {
        let extractor = &*TEST_TNAFLIX;
        let id = extractor.extract_id("https://www.tnaflix.com/hd-videos/test/video123456");
        assert_eq!(id, Some("123456".to_string()));
    }

    #[test]
    fn test_extractor_names() {
        assert_eq!(TEST_TNAFLIX.name(), "TNAFlix");
        assert_eq!(TEST_EMPFLIX.name(), "EMPFlix");
        assert_eq!(TEST_MOVIEFAP.name(), "MovieFap");
    }

    #[test]
    fn test_extractor_priority() {
        assert_eq!(TEST_TNAFLIX.priority(), 0);
        assert_eq!(TEST_EMPFLIX.priority(), 0);
        assert_eq!(TEST_MOVIEFAP.priority(), 0);
    }
}
