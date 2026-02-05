//! xHamster extractor module
//!
//! This module provides extraction support for xHamster videos, embeds, and user playlists.
//!
//! # Architecture
//!
//! The extractor is split into focused submodules:
//! - `patterns` - URL patterns and regex definitions
//! - `formats` - Format extraction from various sources
//! - `decrypt` - URL decryption with 7 PRNG algorithms (including MurmurHash3 fmix32)
//! - `utils` - Error detection, metadata extraction helpers
//!
//! # Supported URLs
//!
//! - Videos: `https://xhamster.com/videos/slug-123456`
//! - Legacy: `https://xhamster.com/movies/123456/slug.html`
//! - Embed: `https://xhamster.com/xembed.php?video=123456`
//! - Users: `https://xhamster.com/users/username/videos`
//! - Creators: `https://xhamster.com/creators/username`

mod decrypt;
mod formats;
mod patterns;
mod utils;

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use log::{debug, info, warn};
use rdlp_core::{
    ExtractionContext, InfoDict, InfoExtractor, RdlpError, Result, check_http_response,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;

use crate::base::common::{BaseExtractor, MAX_PLAYLIST_SIZE};
use crate::hls::detect_format_sizes;

pub use patterns::{XHAMSTER_EMBED_PATTERN, XHAMSTER_VIDEO_PATTERN};

/// Timeout for extracting a single video in playlist mode (30 seconds)
const VIDEO_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Rate limit delay between user page fetches (500ms)
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Number of concurrent video extractions
const CONCURRENT_EXTRACTIONS: usize = 4;

/// xHamster extractor
///
/// Supports:
/// - Single videos (old and new URL schemas)
/// - Embed pages (delegates to video extraction)
/// - User/creator playlists with pagination
///
/// # Example
///
/// ```no_run
/// use rdlp_extractor::XHamsterExtractor;
/// use rdlp_core::InfoExtractor;
///
/// let extractor = XHamsterExtractor::new();
/// assert!(extractor.suitable("https://xhamster.com/videos/test-video-1509445"));
/// ```
pub struct XHamsterExtractor;

impl XHamsterExtractor {
    /// Create a new xHamster extractor
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Extract a single video from a video page URL.
    async fn extract_video(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Rewrite mobile URLs to desktop
        let url = patterns::rewrite_mobile_url(url);

        // Extract video ID and display ID
        let video_id = patterns::extract_video_id(&url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract video ID: {url}")))?;
        let display_id = patterns::extract_display_id(&url);

        // Fetch the webpage
        let webpage = BaseExtractor::fetch_webpage(&url, ctx).await?;

        // Check for video unavailability
        if let Some(error_msg) = utils::detect_video_unavailable(&webpage) {
            return Err(RdlpError::Extraction(error_msg));
        }

        // Extract age limit
        let age_limit = utils::extract_age_limit(&webpage);

        // Try modern layout: window.initials JSON
        let (mut info, formats) =
            if let Some(initials) = extract_initials_json(&webpage) {
                let video_model = initials.get("videoModel");

                let info = if let Some(vm) = video_model {
                    utils::extract_metadata_from_json(
                        vm,
                        &video_id,
                        display_id.as_deref(),
                        &url,
                        self.name(),
                        age_limit,
                    )
                } else {
                    // initials found but no videoModel — fall back to HTML metadata
                    utils::extract_metadata_from_html(
                        &webpage,
                        &video_id,
                        display_id.as_deref(),
                        &url,
                        self.name(),
                        age_limit,
                    )
                };

                let formats = formats::extract_from_initials(&initials, &url);
                (info, formats)
            } else {
                // Legacy fallback
                debug!("[XHamster] No window.initials found, using legacy extraction");
                let info = utils::extract_metadata_from_html(
                    &webpage,
                    &video_id,
                    display_id.as_deref(),
                    &url,
                    self.name(),
                    age_limit,
                );
                let formats = formats::extract_from_legacy(&webpage);
                (info, formats)
            };

        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Detect file sizes and segment counts for HLS
        let (formats_with_size, hls_flags) =
            detect_format_sizes(formats, ctx, self.name()).await;

        info.formats = formats_with_size;
        info.propagate_duration();

        if hls_flags.is_live {
            info.is_live = Some(true);
        }

        Ok(info)
    }

    /// Handle embed URL by fetching the embed page and extracting the real video URL.
    async fn extract_embed(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Try to find the real video URL in the embed page
        if let Some(caps) = patterns::EMBED_VIDEO_URL_PATTERN.captures(&webpage) {
            if let Some(video_url) = caps.get(1) {
                debug!(video_url:? = video_url.as_str(); "[XHamster] Found video URL in embed page");
                return self.extract_video(video_url.as_str(), ctx).await;
            }
        }

        // Try extracting from embed vars JSON
        if let Some(caps) = patterns::EMBED_VARS_PATTERN.captures(&webpage) {
            if let Some(json_str) = caps.get(1) {
                if let Ok(vars) = serde_json::from_str::<serde_json::Value>(json_str.as_str()) {
                    if let Some(video_url) = vars.get("downloadLink")
                        .or_else(|| vars.get("mp4File"))
                        .and_then(|v| v.as_str())
                    {
                        if !video_url.is_empty() {
                            debug!(video_url:?; "[XHamster] Found video URL in embed vars");
                            return self.extract_video(video_url, ctx).await;
                        }
                    }
                }
            }
        }

        Err(RdlpError::Extraction(format!(
            "Could not extract video URL from embed page: {url}"
        )))
    }

    /// Extract all videos from a user/creator page with pagination.
    async fn extract_user_playlist(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<Vec<InfoDict>> {
        let (user_id, _is_user) = patterns::extract_user_info(url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract user ID: {url}")))?;

        info!(user_id:?; "[XHamster] Extracting user playlist");

        let mut all_video_urls: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut page = 1;

        loop {
            let page_url = if page == 1 {
                url.to_string()
            } else {
                format!("{url}/{page}")
            };

            debug!(page, url:? = page_url; "[XHamster] Fetching user page");

            let response = ctx
                .http_client
                .get(&page_url)
                .send()
                .await
                .map_err(|e| {
                    RdlpError::Network(format!("Failed to fetch user page {page}: {e}"))
                })?;

            check_http_response(&response)?;

            let webpage = response
                .text()
                .await
                .map_err(|e| {
                    RdlpError::Network(format!("Failed to read user page {page}: {e}"))
                })?;

            // Extract video URLs from the page
            let page_urls = extract_user_video_urls(&webpage);
            if page_urls.is_empty() {
                break;
            }

            let mut found_new = false;
            for video_url in page_urls {
                if seen.insert(video_url.clone()) {
                    all_video_urls.push(video_url);
                    found_new = true;
                }
            }

            if !found_new {
                break;
            }

            // Check for next page link
            if !webpage.contains("data-page=\"next\"") {
                break;
            }

            page += 1;

            // Rate limiting
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        let total = all_video_urls.len();
        info!(total; "[XHamster] Found videos in user playlist");

        if total == 0 {
            return Err(RdlpError::Extraction(format!(
                "No videos found on user page: {url}"
            )));
        }

        if total > MAX_PLAYLIST_SIZE {
            return Err(RdlpError::Extraction(format!(
                "Playlist too large: {total} videos (max: {MAX_PLAYLIST_SIZE})"
            )));
        }

        // Extract videos in parallel
        debug!(total, concurrent = CONCURRENT_EXTRACTIONS; "[XHamster] Extracting videos");

        let completed = Arc::new(AtomicUsize::new(0));

        let extraction_futures = all_video_urls
            .into_iter()
            .enumerate()
            .map(|(index, video_url)| {
                let position = index + 1;
                let user_id = user_id.clone();
                let completed = Arc::clone(&completed);

                async move {
                    let result = timeout(
                        VIDEO_EXTRACTION_TIMEOUT,
                        self.extract_video(&video_url, ctx),
                    )
                    .await;

                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;

                    match result {
                        Ok(Ok(mut info)) => {
                            info.playlist = Some(user_id);
                            info.playlist_index = Some(position);
                            info.playlist_count = Some(total);

                            debug!(done, total; "[XHamster] Extracted video");
                            Some((position, info))
                        }
                        Ok(Err(e)) => {
                            warn!(position, total; "Failed to extract video: {e}");
                            None
                        }
                        Err(_) => {
                            warn!(position, total; "Timed out extracting video");
                            None
                        }
                    }
                }
            });

        let results: Vec<Option<(usize, InfoDict)>> = stream::iter(extraction_futures)
            .buffer_unordered(CONCURRENT_EXTRACTIONS)
            .collect()
            .await;

        let mut extracted: Vec<(usize, InfoDict)> = results.into_iter().flatten().collect();
        extracted.sort_by_key(|(pos, _)| *pos);

        let results: Vec<InfoDict> = extracted.into_iter().map(|(_, info)| info).collect();

        if results.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "Failed to extract any videos from user page: {url}"
            )));
        }

        info!(extracted = results.len(), total; "[XHamster] Successfully extracted videos");

        Ok(results)
    }
}

impl Default for XHamsterExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for XHamsterExtractor {
    fn name(&self) -> &str {
        "XHamster"
    }

    fn valid_url(&self) -> &regex::Regex {
        &XHAMSTER_VIDEO_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        if patterns::is_embed_url(url) {
            return self.extract_embed(url, ctx).await;
        }

        self.extract_video(url, ctx).await
    }

    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        if patterns::is_user_url(url) {
            return self.extract_user_playlist(url, ctx).await;
        }

        Ok(vec![self.extract(url, ctx).await?])
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Try to extract and parse `window.initials` JSON from the page source.
fn extract_initials_json(webpage: &str) -> Option<serde_json::Value> {
    // Try strict pattern first, then fallback
    let json_str = patterns::INITIALS_PATTERN
        .captures(webpage)
        .or_else(|| patterns::INITIALS_FALLBACK_PATTERN.captures(webpage))
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str())?;

    match serde_json::from_str(json_str) {
        Ok(val) => Some(val),
        Err(e) => {
            debug!("[XHamster] Failed to parse window.initials JSON: {e}");
            None
        }
    }
}

/// Extract video URLs from a user/creator page HTML.
///
/// Looks for `a.video-thumb__image-container` elements with href attributes.
fn extract_user_video_urls(webpage: &str) -> Vec<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;

    static VIDEO_THUMB_HREF: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"<a[^>]+class=[\"'][^\"']*\bvideo-thumb__image-container[^>]+href=[\"']([^\"']+)[\"']"#,
        )
        .expect("Valid video thumb href pattern")
    });

    static VIDEO_THUMB_HREF_ALT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"<a[^>]+href=[\"']([^\"']+)[\"'][^>]+class=[\"'][^\"']*\bvideo-thumb__image-container"#,
        )
        .expect("Valid video thumb href alt pattern")
    });

    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    for pattern in [&*VIDEO_THUMB_HREF, &*VIDEO_THUMB_HREF_ALT] {
        for caps in pattern.captures_iter(webpage) {
            if let Some(href) = caps.get(1) {
                let url = href.as_str().to_string();
                if patterns::XHAMSTER_VIDEO_PATTERN.is_match(&url) && seen.insert(url.clone()) {
                    urls.push(url);
                }
            }
        }
    }

    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = XHamsterExtractor::new();
        assert_eq!(extractor.name(), "XHamster");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = XHamsterExtractor::new();

        // Video URLs
        assert!(extractor.suitable(
            "https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"
        ));
        assert!(extractor.suitable(
            "http://xhamster.com/movies/1509445/femaleagent_shy_beauty.html"
        ));

        // Alt domains
        assert!(extractor
            .suitable("https://xhamster.one/videos/test-1509445"));
        assert!(extractor
            .suitable("https://xhamster2.com/videos/test-1509445"));
        assert!(extractor
            .suitable("https://xhday.com/videos/test-xhh7yVf"));

        // Embed URLs
        assert!(extractor
            .suitable("http://xhamster.com/xembed.php?video=3328539"));

        // User URLs
        assert!(extractor
            .suitable("https://xhamster.com/users/netvideogirls/videos"));
        assert!(extractor
            .suitable("https://xhamster.com/creators/squirt-orgasm-69"));

        // Invalid URLs
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://pornhub.com/view_video.php?viewkey=ph123"));
    }

    #[test]
    fn test_extract_initials_json() {
        let webpage = r#"
            <script>
            window.initials = {"videoModel": {"title": "Test", "sources": {}}};
            </script>
        "#;

        let initials = extract_initials_json(webpage);
        assert!(initials.is_some());
        let initials = initials.unwrap();
        assert_eq!(
            initials.pointer("/videoModel/title").unwrap().as_str(),
            Some("Test")
        );
    }

    #[test]
    fn test_extract_initials_json_not_found() {
        let webpage = "<html><body>No initials here</body></html>";
        assert!(extract_initials_json(webpage).is_none());
    }

    #[test]
    fn test_extract_user_video_urls() {
        let html = r#"
            <a class="video-thumb__image-container" href="https://xhamster.com/videos/test-video-123456">
            <a class="video-thumb__image-container" href="https://xhamster.com/videos/another-video-789012">
            <a class="some-other-class" href="https://xhamster.com/videos/not-this-one-111111">
        "#;

        let urls = extract_user_video_urls(html);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().any(|u| u.contains("123456")));
        assert!(urls.iter().any(|u| u.contains("789012")));
    }
}
