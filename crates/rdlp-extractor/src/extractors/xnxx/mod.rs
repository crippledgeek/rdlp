//! XNXX extractor
//!
//! XNXX is a WGCZ Holding site (same inline-JS format as XVideos).
//! Inline `html5player.setXxx` calls provide HLS and MP4 format URLs.
//!
//! Supports:
//! - Video pages: `https://www.xnxx.com/video-14cco143/slug`
//! - Video pages (no hyphen): `https://www.xnxx.com/video14cco143/slug`
//! - xnxx3.com variant: `https://www.xnxx3.com/video-14cco143/slug`
//! - Embed pages: `https://www.xnxx.com/embedframe/14cco143`
//!
//! ## Module structure
//!
//! - `patterns` - URL regex patterns
//! - `search`   - SearchExtractor implementation

pub mod patterns;
pub mod search;

use async_trait::async_trait;
use lazy_regex::{Lazy, Regex, lazy_regex};
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::Codec;
use rdlp_types::{DownloadProtocol, Format, InfoDict};

use crate::base::common::BaseExtractor;
use crate::base::wgcz_network::WgczNetworkBase;

const XNXX_NAME: &str = "XNXX";
const XNXX_PRIORITY: i32 = 100;

/// XNXX site extractor.
#[derive(Default)]
pub struct XNXXExtractor;

impl XNXXExtractor {
    /// Create a new XNXX extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// Parse height (in pixels) from an XNXX MP4 URL.
///
/// XNXX MP4 URLs commonly include `_240p`, `_360p`, `_720p` etc. in the
/// filename (e.g. `video_360p.mp4`). Falls back to `None` when the URL
/// uses a symbolic label like `mp4_sd.mp4`.
fn parse_mp4_height(url: &str) -> Option<u32> {
    static RE: Lazy<Regex> = lazy_regex!(r"[_-](\d{3,4})p");
    RE.captures(url)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Build `Format` entries from `WgczFormatUrls`.
///
/// XNXX (WGCZ Holding) always serves muxed H.264 + AAC. Setting
/// vcodec/acodec/container explicitly prevents the UI from misclassifying
/// these as audio-only formats.
fn build_formats(format_urls: &crate::base::wgcz_network::WgczFormatUrls) -> Vec<Format> {
    let mut formats = Vec::new();

    if let Some(hls_url) = &format_urls.hls {
        let mut f = Format::new("hls", hls_url, "m3u8", DownloadProtocol::M3u8);
        f.vcodec = Codec::from("h264".to_string());
        f.acodec = Codec::from("aac".to_string());
        f.container = Some("m3u8".to_string());
        f.format_note = Some("HLS".to_string());
        formats.push(f);
    }
    if let Some(low_url) = &format_urls.mp4_low {
        let height = parse_mp4_height(low_url).or(Some(360));
        let mut f = Format::new("mp4_low", low_url, "mp4", DownloadProtocol::Https);
        f.vcodec = Codec::from("h264".to_string());
        f.acodec = Codec::from("aac".to_string());
        f.container = Some("mp4".to_string());
        f.format_note = Some("SD".to_string());
        f.height = height;
        f.quality = Some(-2);
        formats.push(f);
    }
    if let Some(high_url) = &format_urls.mp4_high {
        let height = parse_mp4_height(high_url).or(Some(720));
        let mut f = Format::new("mp4_high", high_url, "mp4", DownloadProtocol::Https);
        f.vcodec = Codec::from("h264".to_string());
        f.acodec = Codec::from("aac".to_string());
        f.container = Some("mp4".to_string());
        f.format_note = Some("HD".to_string());
        f.height = height;
        formats.push(f);
    }

    formats
}

#[async_trait]
impl InfoExtractor for XNXXExtractor {
    fn name(&self) -> &str {
        XNXX_NAME
    }

    fn valid_url(&self) -> &Regex {
        &patterns::VIDEO_URL
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        XNXX_PRIORITY
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let video_id = patterns::extract_video_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!("xnxx: could not extract video ID from URL: {url}"),
            url: Some(url.to_string()),
        })?;

        debug!("[xnxx] Fetching video page for id={video_id}");
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // --- formats ---
        let format_urls = WgczNetworkBase::extract_format_urls(&webpage);
        let formats = build_formats(&format_urls);

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!(
                    "xnxx: no video formats found (html5player calls missing). URL: {url}"
                ),
                url: Some(url.to_string()),
            });
        }

        // --- metadata (sync — no await inside this block) ---
        let inline = WgczNetworkBase::extract_inline_meta(&webpage);
        let ld = WgczNetworkBase::extract_json_ld(&webpage);

        let title = inline
            .title
            .or_else(|| ld.name.clone())
            .unwrap_or_else(|| "Untitled".to_string());

        let thumbnail = inline.thumbnail_url.or_else(|| ld.thumbnail_url.clone());

        let uploader = inline.uploader.clone();

        let description = ld.description.clone();
        let duration = ld
            .duration_iso
            .as_deref()
            .and_then(BaseExtractor::parse_iso8601_duration);
        let view_count = ld.view_count;
        let upload_date = ld.upload_date.clone();

        // --- build InfoDict ---
        let mut info = InfoDict::new(&video_id, &title, XNXX_NAME, url);
        info.extractor = XNXX_NAME.to_string();
        info.description = description;
        info.thumbnail = thumbnail;
        info.uploader = uploader;
        info.view_count = view_count;
        info.upload_date = upload_date;
        info.duration = duration;
        info.age_limit = Some(18);
        // XNXX has no per-video performer taxonomy
        info.actors = Vec::new();
        info.formats = formats;
        info.propagate_duration();

        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_matching_smoke() {
        let ext = XNXXExtractor::new();
        assert!(ext.suitable("https://www.xnxx.com/video-14cco143/slug"));
        assert!(ext.suitable("https://www.xnxx3.com/video-14cco143/"));
        assert!(ext.suitable("https://www.xnxx.com/embedframe/14cco143"));
        assert!(!ext.suitable("https://www.xvideos.com/video.abc/slug"));
        assert!(!ext.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
    }

    #[test]
    fn build_info_extracts_title_and_formats() {
        const FIXTURE: &str = include_str!("tests/xnxx_video_page.html");

        let format_urls = WgczNetworkBase::extract_format_urls(FIXTURE);
        let inline = WgczNetworkBase::extract_inline_meta(FIXTURE);
        let ld = WgczNetworkBase::extract_json_ld(FIXTURE);

        // At least one format URL must be present in the live fixture
        let has_formats = format_urls.hls.is_some()
            || format_urls.mp4_low.is_some()
            || format_urls.mp4_high.is_some();
        assert!(
            has_formats,
            "live fixture must contain at least one format URL"
        );

        // Title must be extractable
        let title = inline.title.or_else(|| ld.name.clone()).unwrap_or_default();
        assert!(!title.is_empty(), "title must not be empty");

        // Formats produced by build_formats
        let formats = build_formats(&format_urls);
        assert!(!formats.is_empty(), "must produce at least one Format");

        // extractor field and actors invariant
        let video_id = "14cco143";
        let mut info = InfoDict::new(
            video_id,
            &title,
            XNXX_NAME,
            "https://www.xnxx.com/video-14cco143/slug",
        );
        info.extractor = XNXX_NAME.to_string();
        info.actors = Vec::new();
        info.formats = formats;

        assert_eq!(info.extractor, "XNXX");
        assert!(info.actors.is_empty(), "XNXX has no performer taxonomy");
    }

    #[test]
    fn extract_round_trip_via_fixture() {
        const FIXTURE: &str = include_str!("tests/xnxx_video_page.html");

        let format_urls = WgczNetworkBase::extract_format_urls(FIXTURE);
        let inline = WgczNetworkBase::extract_inline_meta(FIXTURE);
        let ld = WgczNetworkBase::extract_json_ld(FIXTURE);

        // Verify inline meta
        assert!(
            inline.title.is_some() || ld.name.is_some(),
            "should extract a title from the live fixture"
        );

        // Verify at least one format
        let has_formats = format_urls.hls.is_some()
            || format_urls.mp4_low.is_some()
            || format_urls.mp4_high.is_some();
        assert!(has_formats);

        // HLS url should end with .m3u8 if present
        if let Some(hls) = &format_urls.hls {
            assert!(hls.contains(".m3u8"), "HLS URL should contain .m3u8: {hls}");
        }
    }
}
