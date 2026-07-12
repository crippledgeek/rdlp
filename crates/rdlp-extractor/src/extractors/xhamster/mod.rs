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

mod formats;
mod js_extract;
mod patterns;
mod playlist;
mod search;
mod search_patterns;
mod utils;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, SearchExtractor};
use rdlp_types::{InfoDict, SearchPageResponse};
use std::time::Duration;

use crate::base::common::{BaseExtractor, PagedSearch, SearchPage, Termination};
use crate::hls::detect_format_sizes_lazy;

pub use patterns::{XHAMSTER_EMBED_PATTERN, XHAMSTER_VIDEO_PATTERN};

/// Timeout for extracting a single video in playlist mode (30 seconds)
const VIDEO_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Rate limit delay between playlist page fetches (500ms). Search pagination
/// uses the shared `PagedSearch` default instead.
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
        let video_id = patterns::extract_video_id(&url).ok_or_else(|| RdlpError::Extraction {
            message: format!(
                "Could not extract video ID: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        })?;
        let display_id = patterns::extract_display_id(&url);

        // Fetch the webpage
        let webpage = BaseExtractor::fetch_webpage(&url, ctx).await?;

        // Check for video unavailability
        if let Some(error_msg) = utils::detect_video_unavailable(&webpage) {
            return Err(RdlpError::Extraction {
                message: error_msg,
                url: Some(url.to_string().into()),
            });
        }

        // Extract age limit
        let age_limit = utils::extract_age_limit(&webpage);

        // Try boa-based initials extraction first, fall back to regex
        let boa_initials =
            js_extract::extract_initials_via_boa(&webpage, ctx.js_engine.as_ref()).await;
        let initials = match boa_initials {
            Some(val) => Some(val),
            None => {
                debug!("[XHamster] Boa initials extraction failed, trying regex fallback");
                parse_initials_json(&webpage)
            }
        };

        // Discover and fetch player JS for decryption
        let player_script_urls = js_extract::find_player_script_urls(&webpage);
        let player_js =
            js_extract::fetch_player_js(&player_script_urls, &ctx.http_client, &url).await;

        // Try modern layout: window.initials JSON
        let (mut info, formats) = if let Some(initials) = initials {
            let video_model = initials.get("videoModel");

            let info = if let Some(vm) = video_model {
                utils::extract_metadata_from_json(
                    vm,
                    &video_id,
                    display_id.as_deref(),
                    &url,
                    InfoExtractor::name(self),
                    age_limit,
                )
            } else {
                // initials found but no videoModel — fall back to HTML metadata
                utils::extract_metadata_from_html(
                    &webpage,
                    &video_id,
                    display_id.as_deref(),
                    &url,
                    InfoExtractor::name(self),
                    age_limit,
                )
            };

            let formats = formats::extract_from_initials(
                &initials,
                &url,
                ctx.js_engine.as_ref(),
                player_js.as_deref(),
            )
            .await;
            (info, formats)
        } else {
            // Legacy fallback
            debug!("[XHamster] No window.initials found, using legacy extraction");
            let info = utils::extract_metadata_from_html(
                &webpage,
                &video_id,
                display_id.as_deref(),
                &url,
                InfoExtractor::name(self),
                age_limit,
            );
            let formats = formats::extract_from_legacy(&webpage);
            (info, formats)
        };

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!(
                    "No video formats found for URL: {}",
                    rdlp_redact::RedactedUrl::new(&url)
                ),
                url: Some(url.to_string().into()),
            });
        }

        // Pre-resolve HLS variant playlists into per-variant Format rows so the
        // downloader can take the Format.fragments fast path. Non-HLS rows
        // pass through unchanged; expand failures keep the original row
        // (graceful fallback to the legacy variant-URL path). Each xhamster
        // Format carries `http_headers = Some(referer_headers(page_url))`, so
        // the helper's same-origin header forwarding (added in #263) lets the
        // master + variant playlist fetches reach the CDN with the page-URL
        // Referer that xhamster requires.
        let formats = crate::hls::expand_hls_in_place(formats, ctx.http_client.clone()).await;

        // Detect file sizes and segment counts for HLS
        let (formats_with_size, hls_flags) =
            detect_format_sizes_lazy(formats, ctx, InfoExtractor::name(self)).await;

        info.formats = formats_with_size;
        info.actors = utils::extract_actors(&webpage);

        // Channel / studio (e.g. "Julia Reaves world" under /channels/…).
        // Complements `uploader` which carries the per-user submitter name
        // from videoModel.author. Only set when the page actually exposes
        // a channel chip; otherwise leave untouched.
        if let Some((name, url)) = utils::extract_channel(&webpage) {
            info.channel = Some(name);
            info.channel_url = Some(url);
        }

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
        if let Some(video_url) = patterns::EMBED_VIDEO_URL_PATTERN
            .captures(&webpage)
            .and_then(|caps| caps.get(1))
        {
            debug!(video_url:? = rdlp_redact::RedactedUrl::new(video_url.as_str()); "[XHamster] Found video URL in embed page");
            return self.extract_video(video_url.as_str(), ctx).await;
        }

        // Try extracting from embed vars JSON
        if let Some(video_url) = patterns::EMBED_VARS_PATTERN
            .captures(&webpage)
            .and_then(|caps| caps.get(1))
            .and_then(|json_str| serde_json::from_str::<serde_json::Value>(json_str.as_str()).ok())
            .and_then(|vars| {
                vars.get("downloadLink")
                    .or_else(|| vars.get("mp4File"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
        {
            debug!(video_url:?; "[XHamster] Found video URL in embed vars");
            return self.extract_video(&video_url, ctx).await;
        }

        Err(RdlpError::Extraction {
            message: format!(
                "Could not extract video URL from embed page: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        })
    }
}

impl PagedSearch for XHamsterExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[XHamster]"
    }

    fn validate_search_filters(&self, filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        search::validate_search_filters(filters)
    }

    /// Fetch + parse ONE search page. `has_more` is computed here from the
    /// site's reported page count (the `Termination` helper), so the shared
    /// loop stays conditional-free.
    async fn fetch_page(
        &self,
        query: &rdlp_types::SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let page_url = if page == 1 {
            patterns::build_search_url(query)
        } else {
            patterns::build_search_url_page(query, page as usize)
        };

        debug!(page, url:? = rdlp_security::sanitize_for_logging(&page_url); "[XHamster] Fetching search page");

        let webpage = BaseExtractor::fetch_webpage(&page_url, ctx).await?;
        let initials = search::parse_initials_json(&webpage)?;
        let page_results = search::parse_search_results_json(&initials)?;
        let max_pages = search::parse_max_pages(&initials).unwrap_or(1);

        let has_more =
            !page_results.is_empty() && Termination::Pages(max_pages).has_more(page as usize);
        Ok(SearchPage {
            results: page_results,
            has_more,
            total_estimate: None,
        })
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

#[async_trait]
impl SearchExtractor for XHamsterExtractor {
    fn name(&self) -> &str {
        "XHamster"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        patterns::search_filter_descriptors()
    }

    async fn search(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<rdlp_types::SearchResultPreview>> {
        self.search_all_pages(query, ctx).await
    }

    async fn search_page(
        &self,
        query: &rdlp_types::SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        self.search_page_response(query, ctx).await
    }
}

/// Try to extract and parse `window.initials` JSON from the page source.
fn parse_initials_json(webpage: &str) -> Option<serde_json::Value> {
    // Try strict pattern first, then fallback
    let json_str = [
        &*patterns::INITIALS_PATTERN,
        &*patterns::INITIALS_FALLBACK_PATTERN,
    ]
    .iter()
    .find_map(|pat| pat.captures(webpage))
    .and_then(|caps| caps.get(1))
    .map(|m| m.as_str())?;

    serde_json::from_str(json_str)
        .inspect_err(|e| debug!("[XHamster] Failed to parse window.initials JSON: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = XHamsterExtractor::new();
        assert_eq!(InfoExtractor::name(&extractor), "XHamster");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = XHamsterExtractor::new();

        // Video URLs
        assert!(
            extractor.suitable(
                "https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"
            )
        );
        assert!(
            extractor.suitable("http://xhamster.com/movies/1509445/femaleagent_shy_beauty.html")
        );

        // Alt domains
        assert!(extractor.suitable("https://xhamster.one/videos/test-1509445"));
        assert!(extractor.suitable("https://xhamster2.com/videos/test-1509445"));
        assert!(extractor.suitable("https://xhday.com/videos/test-xhh7yVf"));

        // Embed URLs
        assert!(extractor.suitable("http://xhamster.com/xembed.php?video=3328539"));

        // User URLs
        assert!(extractor.suitable("https://xhamster.com/users/netvideogirls/videos"));
        assert!(extractor.suitable("https://xhamster.com/creators/squirt-orgasm-69"));

        // Invalid URLs
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://pornhub.com/view_video.php?viewkey=ph123"));
    }

    #[test]
    fn test_parse_initials_json() {
        let webpage = r#"
            <script>
            window.initials = {"videoModel": {"title": "Test", "sources": {}}};
            </script>
        "#;

        let initials = parse_initials_json(webpage);
        assert!(initials.is_some());
        let initials = initials.unwrap();
        assert_eq!(
            initials.pointer("/videoModel/title").unwrap().as_str(),
            Some("Test")
        );
    }

    #[test]
    fn test_parse_initials_json_not_found() {
        let webpage = "<html><body>No initials here</body></html>";
        assert!(parse_initials_json(webpage).is_none());
    }

    #[test]
    fn test_xhamster_implements_search_extractor() {
        let extractor = XHamsterExtractor::new();
        let filters =
            <XHamsterExtractor as rdlp_core::SearchExtractor>::supported_filters(&extractor);
        assert!(!filters.is_empty());
        assert_eq!(
            <XHamsterExtractor as rdlp_core::SearchExtractor>::name(&extractor),
            "XHamster"
        );
    }

    #[test]
    fn test_extract_user_video_urls() {
        let html = r#"
            <a class="video-thumb__image-container" href="https://xhamster.com/videos/test-video-123456">
            <a class="video-thumb__image-container" href="https://xhamster.com/videos/another-video-789012">
            <a class="some-other-class" href="https://xhamster.com/videos/not-this-one-111111">
        "#;

        let urls = playlist::extract_user_video_urls(html);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().any(|u| u.contains("123456")));
        assert!(urls.iter().any(|u| u.contains("789012")));
    }

    /// Regression guard for #258 — confirm an `M3u8Native` HLS row produced
    /// by xhamster's format builder is expanded into per-variant fragments
    /// by `expand_hls_in_place`, that the per-format Referer header is
    /// preserved on every expanded row (xhamster requires the page-URL
    /// Referer for CDN segment fetches), and that non-HLS rows pass through
    /// unchanged. Catches a wiring break where the helper call is removed
    /// from `extract_video` (the M3u8Native row would arrive at the
    /// downloader without pre-resolved fragments) AND a regression in
    /// `expand_media_playlist`'s seed-clone behaviour that would drop the
    /// Referer.
    #[tokio::test]
    async fn xhamster_hls_row_expanded_with_referer_preserved() {
        use rdlp_types::{DownloadProtocol, Format};

        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/xh-master.m3u8")
            .match_header("Referer", "https://xhamster.com/videos/some-video-123")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let hls_url = format!("{}/xh-master.m3u8", server.url());
        let mut hls = Format::new(
            "hls-h264-url",
            &hls_url,
            "mp4",
            DownloadProtocol::M3u8Native,
        );
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://xhamster.com/videos/some-video-123".to_string(),
        );
        hls.http_headers = Some(headers);

        let mut mp4 = Format::new(
            "standard-720p",
            "https://cdn.example.com/v_720p.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        mp4.height = Some(720);

        let formats = vec![hls, mp4];
        let http = std::sync::Arc::new(wreq::Client::new());
        let expanded = crate::hls::expand_hls_in_place(formats, http).await;

        assert_eq!(expanded.len(), 2);
        assert!(
            expanded[0].fragments.is_some(),
            "M3u8Native row must carry pre-resolved fragments after expand"
        );
        assert_eq!(expanded[0].fragments.as_ref().unwrap().len(), 2);
        let preserved = expanded[0]
            .http_headers
            .as_ref()
            .and_then(|h| h.get("Referer"))
            .map(String::as_str);
        assert_eq!(
            preserved,
            Some("https://xhamster.com/videos/some-video-123"),
            "Referer header must survive expand (xhamster CDN rejects without it)"
        );

        assert!(
            expanded[1].fragments.is_none(),
            "Https MP4 row must pass through untouched"
        );
        assert_eq!(expanded[1].url, "https://cdn.example.com/v_720p.mp4");
        assert_eq!(expanded[1].height, Some(720));
    }
}
