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
use rdlp_core::{ExtractionContext, Format, Fragment, InfoDict, InfoExtractor, RdlpError, Result};
use scraper::Html;
use std::time::Duration;
use tokio::time::timeout;

use crate::base::common::BaseExtractor;
use crate::hls::HlsSizeDetector;

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

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        0
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

        // Detect file sizes
        let formats_with_size = detect_sizes(formats, ctx).await;

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
}

/// Detect file sizes and segment counts for formats using parallel requests
///
/// All formats are detected concurrently using `join_all` for maximum speed.
/// HLS formats use fast segment counting (no size fetching - just parses m3u8).
async fn detect_sizes(formats: Vec<Format>, ctx: &ExtractionContext) -> Vec<Format> {
    use futures::future::join_all;

    let verbose = ctx.config.verbose;
    let hls_detector = HlsSizeDetector::new(ctx.http_client.clone(), verbose);
    let http_client = ctx.http_client.clone();

    // Create detection tasks for all formats in parallel
    let detection_futures: Vec<_> = formats
        .into_iter()
        .map(|format| {
            let hls_detector = hls_detector.clone();
            let http_client = http_client.clone();

            async move {
                let mut format = format;
                let url = format.url.clone();
                let is_hls = url.contains(".m3u8") || url.contains("/hls/");

                if is_hls {
                    // Fast segment count only (no size fetching) - just parses m3u8
                    let result = timeout(
                        Duration::from_secs(5), // Fast - only 1-2 HTTP requests
                        hls_detector.count_segments(&url),
                    )
                    .await;

                    match result {
                        Ok(Ok(Some(segment_count))) => {
                            // Store segment count in fragments field
                            format.fragments = Some(
                                (0..segment_count)
                                    .map(|_| Fragment {
                                        url: String::new(),
                                        duration: None,
                                        filesize: None,
                                    })
                                    .collect(),
                            );
                            if verbose {
                                eprintln!(
                                    "[PornHub] HLS {}: {} segments",
                                    format.format_note.as_deref().unwrap_or("unknown"),
                                    segment_count
                                );
                            }
                        }
                        Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                            // Fallback - no segment count available
                            if verbose {
                                eprintln!("[PornHub] Could not count segments for: {url}");
                            }
                        }
                    }
                } else {
                    // Use BaseExtractor size detection with timeout for non-HLS
                    let result = timeout(
                        Duration::from_secs(5),
                        BaseExtractor::detect_file_size_with_client(&url, &http_client),
                    )
                    .await;

                    if let Ok(Some(size)) = result {
                        format.filesize = Some(size);
                    }
                }

                format
            }
        })
        .collect();

    // Execute all detection tasks in parallel
    join_all(detection_futures).await
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
