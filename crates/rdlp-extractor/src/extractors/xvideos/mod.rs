//! XVideos extractor
//!
//! XVideos is a WGCZ Holding tube site serving HLS and MP4 downloads.
//!
//! Supports:
//! - Video pages: `https://www.xvideos.com/video.ooumovia9b7/slug`
//! - Embed pages: `https://www.xvideos.com/embedframe/ooumovia9b7`
//! - Language subdomains: `fr.xvideos.com`, `de.xvideos.es`
//! - `xvideos2.com` and `.es` TLD variants
//!
//! ## Module Structure
//!
//! - `patterns` - URL regex patterns
//! - `search` - Search extractor implementation

pub mod patterns;
pub mod search;

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::{DownloadProtocol, Format, InfoDict};
use regex::Regex;

use crate::base::common::BaseExtractor;
use crate::base::wgcz_network::WgczNetworkBase;

/// Parse height (in pixels) from an XVideos MP4 URL.
///
/// XVideos/XNXX (WGCZ) MP4 URLs tend to follow one of these patterns:
///   - `.../video_240p.mp4`, `.../video_360p.mp4` — height is encoded directly
///   - `.../mp4_sd.mp4`, `.../mp4_hd.mp4`         — symbolic labels, no explicit height
///
/// Returns `Some(height)` when a `_(\d+)p` fragment is present, otherwise `None`.
fn parse_mp4_height(url: &str) -> Option<u32> {
    static RE: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"[_-](\d{3,4})p").unwrap());
    RE.captures(url)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// XVideos extractor
#[derive(Default)]
pub struct XVideosExtractor;

impl XVideosExtractor {
    /// Create a new XVideos extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Build an `InfoDict` from raw HTML bytes.
    ///
    /// Separated from `extract()` so tests can call it with fixture bytes
    /// without needing a matching URL.
    pub(crate) fn build_info(html: &str, eid: &str, url: &str) -> Result<InfoDict> {
        // Extract format URLs from inline html5player calls
        let fmt_urls = WgczNetworkBase::extract_format_urls(html);
        let inline_meta = WgczNetworkBase::extract_inline_meta(html);
        let json_ld = WgczNetworkBase::extract_json_ld(html);

        // Title: prefer inline JS, fall back to JSON-LD name
        let title = inline_meta
            .title
            .or(json_ld.name.clone())
            .unwrap_or_else(|| "Untitled".to_string());

        // Build formats. XVideos (WGCZ Holding) always serves muxed H.264 +
        // AAC video. Setting vcodec/acodec/container explicitly is what lets
        // the UI classify these as video formats — otherwise they fall into
        // the "Audio Only" bucket by default.
        let mut formats: Vec<Format> = Vec::new();

        if let Some(hls_url) = fmt_urls.hls {
            let mut f = Format::new("hls-0", hls_url, "m3u8", DownloadProtocol::M3u8Native);
            f.format_note = Some("HLS".to_string());
            f.vcodec = Some("h264".to_string());
            f.acodec = Some("aac".to_string());
            f.container = Some("m3u8".to_string());
            formats.push(f);
        }

        if let Some(high_url) = fmt_urls.mp4_high {
            let height = parse_mp4_height(&high_url).or(Some(720));
            let mut f = Format::new("mp4-hd", high_url, "mp4", DownloadProtocol::Https);
            f.format_note = Some("HD".to_string());
            f.vcodec = Some("h264".to_string());
            f.acodec = Some("aac".to_string());
            f.container = Some("mp4".to_string());
            f.height = height;
            formats.push(f);
        }

        if let Some(low_url) = fmt_urls.mp4_low {
            let height = parse_mp4_height(&low_url).or(Some(360));
            let mut f = Format::new("mp4-sd", low_url, "mp4", DownloadProtocol::Https);
            f.format_note = Some("SD".to_string());
            f.vcodec = Some("h264".to_string());
            f.acodec = Some("aac".to_string());
            f.container = Some("mp4".to_string());
            f.height = height;
            f.quality = Some(-2);
            formats.push(f);
        }

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!("No video formats found on page. URL: {url}"),
                url: Some(url.to_string()),
            });
        }

        // Duration: prefer JSON-LD ISO 8601, no text fallback on WGCZ pages
        let duration = json_ld
            .duration_iso
            .as_deref()
            .and_then(BaseExtractor::parse_iso8601_duration);

        // Upload date: normalise from ISO 8601 to YYYYMMDD
        let upload_date = json_ld
            .upload_date
            .as_deref()
            .and_then(BaseExtractor::parse_iso8601_date);

        // Thumbnail: prefer inline JS, fall back to JSON-LD
        let thumbnail = inline_meta.thumbnail_url.or(json_ld.thumbnail_url.clone());

        let mut info = InfoDict::new(eid, title, "XVideos", url);
        info.thumbnail = thumbnail;
        info.description = json_ld.description;
        info.duration = duration;
        info.upload_date = upload_date;
        info.view_count = json_ld.view_count;
        info.uploader = inline_meta.uploader;
        info.age_limit = Some(18);
        // XVideos has no per-video performer taxonomy
        info.actors = Vec::new();
        info.formats = formats;
        info.propagate_duration();

        Ok(info)
    }
}

#[async_trait]
impl InfoExtractor for XVideosExtractor {
    fn name(&self) -> &str {
        "XVideos"
    }

    fn valid_url(&self) -> &Regex {
        &patterns::VIDEO_URL
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::VIDEO_URL.is_match(url) || patterns::EMBED_URL.is_match(url)
    }

    fn priority(&self) -> i32 {
        0
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let eid = patterns::extract_eid(url).ok_or_else(|| RdlpError::Extraction {
            message: format!("Could not extract video EID from URL: {url}"),
            url: Some(url.to_string()),
        })?;

        let html = BaseExtractor::fetch_webpage(url, ctx).await?;
        Self::build_info(&html, &eid, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("tests/xvideos_video_page.html");

    #[test]
    fn url_matching_smoke() {
        let ext = XVideosExtractor::new();
        assert!(ext.suitable("https://www.xvideos.com/video.ooumovia9b7/some-slug"));
        assert!(!ext.suitable("https://www.xnxx.com/video12345/title"));
    }

    #[test]
    fn build_info_extracts_title_and_formats() {
        let info = XVideosExtractor::build_info(
            FIXTURE,
            "ooumovia9b7",
            "https://www.xvideos.com/video.ooumovia9b7/slug",
        )
        .expect("build_info should succeed");

        assert!(!info.title.is_empty(), "title should not be empty");
        assert_eq!(info.extractor, "XVideos", "extractor should be 'XVideos'");
        assert!(info.actors.is_empty(), "actors should be empty for XVideos");

        let has_m3u8 = info.formats.iter().any(|f| {
            f.protocol == DownloadProtocol::M3u8Native
                || f.ext == "m3u8"
                || f.url.ends_with(".m3u8")
        });
        assert!(has_m3u8, "should have at least one m3u8 format");

        let has_mp4 = info
            .formats
            .iter()
            .any(|f| f.ext == "mp4" && f.protocol == DownloadProtocol::Https);
        assert!(has_mp4, "should have at least one mp4 format");
    }

    #[tokio::test]
    async fn extract_round_trip_via_mockito() {
        use async_trait::async_trait;
        use mockito::Server;
        use rdlp_core::{CookieJar, ExtractionContext, JsEngine};
        use rdlp_types::Config;
        use std::sync::Arc;

        struct NoOpJs;
        #[async_trait]
        impl JsEngine for NoOpJs {
            async fn eval(&self, _: &str) -> Result<serde_json::Value> {
                Ok(serde_json::Value::Null)
            }
            async fn eval_with_context(
                &self,
                _: &str,
                _: &serde_json::Value,
            ) -> Result<serde_json::Value> {
                Ok(serde_json::Value::Null)
            }
            async fn call_function(
                &self,
                _: &str,
                _: &[serde_json::Value],
            ) -> Result<serde_json::Value> {
                Ok(serde_json::Value::Null)
            }
        }

        struct NoOpCookies;
        #[async_trait]
        impl CookieJar for NoOpCookies {
            async fn get_cookies(&self, _: &str) -> Result<Vec<String>> {
                Ok(vec![])
            }
            async fn add_cookie(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            async fn load_from_browser(&self, _: rdlp_types::BrowserType) -> Result<usize> {
                Ok(0)
            }
            async fn load_from_file(&self, _: &std::path::Path) -> Result<usize> {
                Ok(0)
            }
        }

        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_body(FIXTURE)
            .create_async()
            .await;

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let ctx = ExtractionContext::new(
            Arc::new(client),
            Arc::new(NoOpJs),
            Arc::new(NoOpCookies),
            Arc::new(Config::default()),
        );

        // VIDEO_URL regex does not accept 127.0.0.1, so fetch HTML via mockito
        // then call build_info directly — proves the full parse pipeline works.
        let html = ctx
            .http_client
            .get(format!("{}/video.ooumovia9b7/slug", server.url()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        let info = XVideosExtractor::build_info(
            &html,
            "ooumovia9b7",
            "https://www.xvideos.com/video.ooumovia9b7/slug",
        )
        .expect("build_info should succeed on fixture from mockito");

        assert!(!info.title.is_empty());
        assert_eq!(info.id, "ooumovia9b7");
        assert!(!info.formats.is_empty());
    }
}
