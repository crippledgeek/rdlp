//! TNAFlixExtractor — single-video info extractor for the TNAFlix network.
//!
//! Supports TNAFlix, EMPFlix, and MovieFap through a shared extraction
//! pipeline backed by [`TnaFlixNetworkBase`].

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::InfoDict;
use regex::Regex;
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::base::tnaflix_network::TnaFlixNetworkBase;

use super::ajax;
use super::patterns::{EMPFLIX_URL_PATTERN, MOVIEFAP_URL_PATTERN, TNAFLIX_URL_PATTERN};

/// TNAFlix network extractor (supports TNAFlix, EMPFlix, MovieFap)
///
/// Uses [`TnaFlixNetworkBase`] for shared extraction logic.
pub struct TNAFlixExtractor {
    pub(super) name: &'static str,
    pub(super) url_pattern: &'static Regex,
    pub(super) base: TnaFlixNetworkBase,
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
    pub(super) fn extract_id(&self, url: &str) -> Option<String> {
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
        let video_id = self.extract_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!(
                "Could not extract video ID from URL: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
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
            let cdn_url = cdn_url_opt.ok_or_else(|| RdlpError::Extraction {
                message: format!(
                    "Could not find cdn.php URL in MovieFap page: {}",
                    rdlp_redact::RedactedUrl::new(&url)
                ),
                url: Some(url.to_string().into()),
            })?;

            BaseExtractor::log_if_verbose(ctx, "MovieFap", &format!("cdn.php URL: {cdn_url}"));

            ajax::parse_moviefap_xml(&self.base, &cdn_url, url, ctx).await?
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
                return Err(RdlpError::Extraction {
                    message: format!(
                        "No video source tags found in HTML. Video may be unavailable. URL: {}",
                        rdlp_redact::RedactedUrl::new(url)
                    ),
                    url: Some(url.to_string().into()),
                });
            }

            video_data
        };

        // Build formats and fetch filesizes using base (asynchronous)
        let mut formats = self.base.build_formats(video_data, ctx).await;

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!(
                    "No video formats found for URL: {}",
                    rdlp_redact::RedactedUrl::new(&url)
                ),
                url: Some(url.to_string().into()),
            });
        }

        // Set Referer header on all formats so the CDN receives it during download.
        // MovieFap's CDN may throttle requests without a proper Referer.
        if is_moviefap {
            let mut headers = std::collections::HashMap::new();
            headers.insert("Referer".to_string(), url.to_string());
            for format in &mut formats {
                format.http_headers = Some(headers.clone());
            }
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
