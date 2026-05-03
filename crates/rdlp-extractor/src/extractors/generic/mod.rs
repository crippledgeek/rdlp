//! Generic fallback extractor.
//!
//! Handles unknown sites by detecting direct media URLs, structured data
//! (JSON-LD, OpenGraph, Twitter cards), JS video players (JW Player, KVS,
//! Video.js), and HTML5 `<video>` elements. Runs at lowest priority (`-1000`)
//! after all site-specific extractors.

mod detection;
mod direct;
mod html_sources;
mod js_sources;
mod json_ld;
mod meta_sources;
mod patterns;

use async_trait::async_trait;
use regex::Regex;
use scraper::Html;
use url::Url;

use rdlp_core::{ExtractionContext, InfoExtractor, Result};
use rdlp_types::{DownloadProtocol, Format, InfoDict};

use crate::base::common::BaseExtractor;

use self::detection::{
    DetectedFormat, DetectionStrategy, PageContext, ext_from_url, run_detection_pipeline,
};
use self::direct::{PrefetchResponse, prefetch, title_from_url};
use self::json_ld::JsonLdStrategy;

/// Maximum page size to parse (2 MB). Pages larger than this are truncated.
const MAX_PAGE_SIZE: usize = 2 * 1024 * 1024;

/// Generic fallback extractor that handles unknown sites.
pub struct GenericExtractor {
    strategies: Vec<Box<dyn DetectionStrategy>>,
}

impl Default for GenericExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl GenericExtractor {
    /// Create a new generic extractor with all detection strategies.
    #[must_use]
    pub fn new() -> Self {
        Self {
            strategies: vec![
                Box::new(JsonLdStrategy),
                Box::new(meta_sources::OpenGraphStrategy),
                Box::new(meta_sources::TwitterPlayerStrategy),
                Box::new(js_sources::JwPlayerStrategy),
                Box::new(js_sources::KvsPlayerStrategy),
                Box::new(js_sources::VideoJsStrategy),
                Box::new(js_sources::GenericJsParamsStrategy),
                Box::new(html_sources::Html5VideoStrategy),
                Box::new(html_sources::IframeEmbedStrategy),
                Box::new(js_sources::DirectLinkScanStrategy),
            ],
        }
    }
}

#[async_trait]
impl InfoExtractor for GenericExtractor {
    fn name(&self) -> &str {
        "Generic"
    }

    fn valid_url(&self) -> &Regex {
        &patterns::GENERIC_URL_PATTERN
    }

    fn priority(&self) -> i32 {
        -1000
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let parsed_url = Url::parse(url).map_err(|e| rdlp_core::RdlpError::Extraction {
            message: format!("Invalid URL: {e}"),
            url: Some(url.to_string()),
        })?;

        let domain = parsed_url.host_str().unwrap_or("unknown");
        log::info!(
            "No site-specific extractor for {}, trying generic extraction",
            domain
        );

        // === Phase 1: Prefetch (512 bytes) ===
        let prefetch_result = prefetch(url, ctx).await.ok();

        // Check for direct media URL
        if let Some(ref pf) = prefetch_result
            && let Some(mut info) = try_direct_media(url, pf)?
        {
            // Pre-resolve HLS variant playlists into per-variant Format rows
            // so the downloader can take the Format.fragments fast path.
            // Non-HLS rows pass through unchanged; expand failures keep the
            // original row (graceful fallback to the legacy variant-URL path).
            info.formats =
                crate::hls::expand_hls_in_place(info.formats, ctx.http_client.clone()).await;
            return Ok(info);
        }

        // === Phase 2: Fetch full page ===
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Truncate oversized pages — must walk back to a UTF-8 char boundary
        // because Generic runs against arbitrary unknown sites (highest input
        // variance in the workspace) and a multi-byte character at byte
        // MAX_PAGE_SIZE would panic the bare slice.
        let webpage = if webpage.len() > MAX_PAGE_SIZE {
            log::warn!(
                "Page exceeds size limit ({} bytes, max {}), truncating",
                webpage.len(),
                MAX_PAGE_SIZE
            );
            let mut end = MAX_PAGE_SIZE;
            while end > 0 && !webpage.is_char_boundary(end) {
                end -= 1;
            }
            webpage[..end].to_string()
        } else {
            webpage
        };

        // === Phase 3: Sync detection (Html is !Send — must not cross .await) ===
        let (formats, title, description, thumbnail, json_ld_meta) = {
            let html = Html::parse_document(&webpage);
            let base_url = extract_base_url(&html, &parsed_url);
            let page_ctx = PageContext {
                url: &parsed_url,
                base_url: &base_url,
                html: &html,
                raw_html: &webpage,
            };

            let detected = run_detection_pipeline(&self.strategies, &page_ctx);
            let json_ld_meta = JsonLdStrategy::extract_metadata(&page_ctx);

            // Metadata from HTML (JSON-LD takes priority where available)
            let title = json_ld_meta
                .title
                .clone()
                .or_else(|| BaseExtractor::extract_title_multi_strategy(&html))
                .unwrap_or_else(|| title_from_url(url));

            let description = json_ld_meta
                .description
                .clone()
                .or_else(|| BaseExtractor::extract_description_multi_strategy(&html));

            let thumbnail = json_ld_meta
                .thumbnail
                .clone()
                .or_else(|| BaseExtractor::extract_thumbnail_multi_strategy(&html));

            (detected, title, description, thumbnail, json_ld_meta)
        }; // html dropped here — safe to .await below

        if formats.is_empty() {
            return Err(rdlp_core::RdlpError::Extraction {
                message: "No media found on page. If this site uses JavaScript rendering, \
                          a dedicated extractor may be needed."
                    .to_string(),
                url: Some(url.to_string()),
            });
        }

        log::info!("Generic extractor found {} format(s)", formats.len());

        // === Phase 4: Build InfoDict ===
        let video_id = generate_video_id(url);
        let mut info = InfoDict::new(&video_id, &title, "Generic", url);
        info.description = description;
        info.thumbnail = thumbnail;

        if let Some(secs) = json_ld_meta.duration_seconds {
            info.duration = Some(secs);
        }
        if let Some(date) = json_ld_meta.upload_date {
            info.upload_date = Some(date);
        }

        info.formats = formats.into_iter().map(detected_to_format).collect();

        // Pre-resolve HLS variant playlists into per-variant Format rows so
        // the downloader can take the Format.fragments fast path. Non-HLS
        // rows pass through unchanged; expand failures keep the original row
        // (graceful fallback to the legacy variant-URL path).
        info.formats = crate::hls::expand_hls_in_place(info.formats, ctx.http_client.clone()).await;

        Ok(info)
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Try to handle a direct media URL from the prefetch response.
fn try_direct_media(url: &str, pf: &PrefetchResponse) -> Result<Option<InfoDict>> {
    // Check for DASH manifest before the generic media content-type branch, since
    // `application/dash+xml` satisfies `is_media_content_type` and would otherwise
    // produce a wrong generic-direct format instead of `HttpDashSegments`.
    if pf.is_dash_content_type() || pf.is_mpd_manifest() {
        let title = title_from_url(url);
        let video_id = generate_video_id(url);

        let body_str = std::str::from_utf8(&pf.bytes).unwrap_or("");
        let mpd_url = match Url::parse(url) {
            Ok(u) => u,
            Err(_) => {
                // Malformed input URL — fall back to legacy placeholder so the
                // download path can still try with the raw URL string.
                let mut info = InfoDict::new(&video_id, &title, "Generic", url);
                let format = Format::new("dash", url, "mpd", DownloadProtocol::HttpDashSegments);
                info.formats = vec![format];
                return Ok(Some(info));
            }
        };

        match crate::base::common::dash::expand_dash_representations(body_str, &mpd_url) {
            Ok(crate::base::common::dash::DashExpansion {
                formats,
                subtitles: _,
            }) => {
                // Subtitles deliberately dropped here — InfoDict has no subtitles field today.
                // Tracking issue (to be filed) for orchestrator-level propagation.
                let mut info = InfoDict::new(&video_id, &title, "Generic", url);
                info.formats = formats;
                return Ok(Some(info));
            }
            Err(crate::base::common::dash::DashExpandError::DynamicMpd) => {
                // Live/dynamic manifest — not supported; let other strategies try.
                log::warn!("DASH dynamic/live manifest at {url}; not yet supported");
                return Ok(None);
            }
            Err(e) => {
                log::warn!(
                    "DASH expansion failed for {url}: {e}; falling back to legacy single-Format path"
                );
                let mut info = InfoDict::new(&video_id, &title, "Generic", url);
                let format = Format::new("dash", url, "mpd", DownloadProtocol::HttpDashSegments);
                info.formats = vec![format];
                return Ok(Some(info));
            }
        }
    }

    if pf.is_media_content_type() && !pf.is_html_content_type() {
        let title = title_from_url(url);
        let ext = ext_from_url(url).or_else(|| {
            pf.content_type
                .as_deref()
                .and_then(content_type_to_ext)
                .map(|s| s.to_string())
        });

        let video_id = generate_video_id(url);
        let mut info = InfoDict::new(&video_id, &title, "Generic", url);

        let protocol = protocol_from_url(url, ext.as_deref());
        let mut format = Format::new(
            format!("generic-direct-{}", ext.as_deref().unwrap_or("video")),
            url,
            ext.as_deref().unwrap_or("mp4"),
            protocol,
        );
        if let Some(size) = pf.content_length {
            format.filesize = Some(size);
        }

        info.formats = vec![format];
        return Ok(Some(info));
    }

    // Check for HLS manifest in prefetch bytes
    if pf.is_hls_manifest() {
        let title = title_from_url(url);
        let video_id = generate_video_id(url);
        let mut info = InfoDict::new(&video_id, &title, "Generic", url);
        let format = Format::new("generic-hls", url, "m3u8", DownloadProtocol::M3u8);
        info.formats = vec![format];
        return Ok(Some(info));
    }

    Ok(None)
}

/// Extract the base URL from `<base href="...">` or fall back to the page URL.
fn extract_base_url(html: &Html, page_url: &Url) -> Url {
    use scraper::Selector;
    use std::sync::LazyLock;

    static BASE_SELECTOR: LazyLock<Selector> = crate::static_selector!("base[href]");

    html.select(&BASE_SELECTOR)
        .next()
        .and_then(|elem| elem.value().attr("href"))
        .and_then(|href| Url::parse(href).or_else(|_| page_url.join(href)).ok())
        .unwrap_or_else(|| page_url.clone())
}

/// Generate a stable video ID from a URL (hash-based).
fn generate_video_id(url: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("generic-{:016x}", hasher.finish())
}

/// Convert a `DetectedFormat` to an `rdlp_types::Format`.
fn detected_to_format(df: DetectedFormat) -> Format {
    let format_id = format!(
        "generic-{}-{}",
        df.source.replace([':', '.'], "-"),
        df.ext.as_deref().unwrap_or("video")
    );
    let ext = df.ext.as_deref().unwrap_or("mp4");

    let protocol = protocol_from_url(&df.url, df.ext.as_deref());
    let mut format = Format::new(&format_id, &df.url, ext, protocol);

    if let Some(q) = df.quality {
        format.format_note = Some(q);
    }

    format
}

/// Infer the download protocol from a URL and optional extension.
fn protocol_from_url(url: &str, ext: Option<&str>) -> DownloadProtocol {
    match ext {
        Some("m3u8") => DownloadProtocol::M3u8,
        _ if url.starts_with("https://") => DownloadProtocol::Https,
        _ if url.starts_with("http://") => DownloadProtocol::Http,
        _ => DownloadProtocol::Https,
    }
}

/// Map Content-Type to a file extension.
fn content_type_to_ext(ct: &str) -> Option<&'static str> {
    let ct = ct.split(';').next().unwrap_or(ct).trim();
    match ct {
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/x-flv" => Some("flv"),
        "video/quicktime" => Some("mov"),
        "video/x-matroska" => Some("mkv"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" => Some("m4a"),
        "audio/ogg" => Some("ogg"),
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => Some("m3u8"),
        "application/dash+xml" => Some("mpd"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_extractor_priority() {
        let ext = GenericExtractor::new();
        assert_eq!(ext.priority(), -1000);
    }

    #[test]
    fn generic_extractor_name() {
        let ext = GenericExtractor::new();
        assert_eq!(ext.name(), "Generic");
    }

    #[test]
    fn generic_url_matches_http() {
        let ext = GenericExtractor::new();
        assert!(ext.suitable("https://example.com/video"));
        assert!(ext.suitable("http://example.com/page"));
        assert!(!ext.suitable("ftp://example.com/file"));
    }

    #[test]
    fn generate_video_id_stable() {
        let id1 = generate_video_id("https://example.com/video");
        let id2 = generate_video_id("https://example.com/video");
        assert_eq!(id1, id2);
        assert!(id1.starts_with("generic-"));
    }

    #[test]
    fn generate_video_id_different_for_different_urls() {
        let id1 = generate_video_id("https://example.com/video1");
        let id2 = generate_video_id("https://example.com/video2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn extract_base_url_from_tag() {
        let html_str = r#"<html><head><base href="https://cdn.example.com/"></head></html>"#;
        let html = Html::parse_document(html_str);
        let page_url = Url::parse("https://example.com/page").unwrap();

        let base = extract_base_url(&html, &page_url);
        assert_eq!(base.as_str(), "https://cdn.example.com/");
    }

    #[test]
    fn extract_base_url_fallback_to_page() {
        let html_str = r#"<html><head></head></html>"#;
        let html = Html::parse_document(html_str);
        let page_url = Url::parse("https://example.com/page").unwrap();

        let base = extract_base_url(&html, &page_url);
        assert_eq!(base.as_str(), "https://example.com/page");
    }

    #[test]
    fn content_type_to_ext_works() {
        assert_eq!(content_type_to_ext("video/mp4"), Some("mp4"));
        assert_eq!(content_type_to_ext("video/mp4; codecs=avc1"), Some("mp4"));
        assert_eq!(
            content_type_to_ext("application/vnd.apple.mpegurl"),
            Some("m3u8")
        );
        assert_eq!(content_type_to_ext("text/html"), None);
    }

    #[test]
    fn detected_to_format_conversion() {
        let df = DetectedFormat {
            url: "https://cdn.example.com/video.mp4".to_string(),
            ext: Some("mp4".to_string()),
            quality: Some("720p".to_string()),
            confidence: detection::Confidence::High,
            source: "og:video",
        };
        let format = detected_to_format(df);
        assert_eq!(format.url, "https://cdn.example.com/video.mp4");
        assert_eq!(format.ext, "mp4");
        assert_eq!(format.format_note, Some("720p".to_string()));
    }

    #[test]
    fn try_direct_media_with_video_content_type() {
        let pf = PrefetchResponse {
            content_type: Some("video/mp4".to_string()),
            bytes: vec![0; 100],
            content_length: Some(1_000_000),
        };
        let result = try_direct_media("https://cdn.example.com/video.mp4", &pf).unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.formats.len(), 1);
        assert_eq!(info.formats[0].ext, "mp4");
        assert_eq!(info.formats[0].filesize, Some(1_000_000));
    }

    #[test]
    fn try_direct_media_with_hls_prefetch() {
        let pf = PrefetchResponse {
            content_type: Some("application/octet-stream".to_string()),
            bytes: b"#EXTM3U\n#EXT-X-VERSION:3".to_vec(),
            content_length: None,
        };
        let result = try_direct_media("https://cdn.example.com/stream.m3u8", &pf).unwrap();
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.formats.len(), 1);
        assert_eq!(info.formats[0].ext, "m3u8");
    }

    #[test]
    fn try_direct_media_html_returns_none() {
        let pf = PrefetchResponse {
            content_type: Some("text/html; charset=utf-8".to_string()),
            bytes: b"<!DOCTYPE html>".to_vec(),
            content_length: None,
        };
        let result = try_direct_media("https://example.com/page", &pf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn direct_mpd_emits_dash_format() {
        let mpd_xml = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT4S">
  <Period duration="PT4S">
    <AdaptationSet mimeType="video/mp4" codecs="avc1.4d401e">
      <Representation id="v1" bandwidth="1000000" width="640" height="360">
        <SegmentTemplate media="$Number$.m4s" duration="2" timescale="1" startNumber="1"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;

        // Content-Type: application/dash+xml — should emit per-Repr HttpDashSegments formats
        let pf_ct = PrefetchResponse {
            content_type: Some("application/dash+xml".to_string()),
            bytes: mpd_xml.as_bytes().to_vec(),
            content_length: None,
        };
        let result = try_direct_media("https://cdn.example.com/manifest.mpd", &pf_ct).unwrap();
        assert!(
            result.is_some(),
            "expected Some(InfoDict) for MPD via Content-Type"
        );
        let info = result.unwrap();
        assert!(
            !info.formats.is_empty(),
            "expansion produces at least 1 Format"
        );
        for f in &info.formats {
            assert_eq!(
                f.protocol,
                DownloadProtocol::HttpDashSegments,
                "format must use HttpDashSegments protocol"
            );
            assert!(f.fragments.is_some(), "fragments must be pre-resolved");
        }

        // Body sniff: <MPD start with no application/dash+xml Content-Type
        let pf_sniff = PrefetchResponse {
            content_type: Some("application/octet-stream".to_string()),
            bytes: mpd_xml.as_bytes().to_vec(),
            content_length: None,
        };
        let result2 = try_direct_media("https://cdn.example.com/stream.mpd", &pf_sniff).unwrap();
        assert!(
            result2.is_some(),
            "expected Some(InfoDict) for MPD via body sniff"
        );
        let info2 = result2.unwrap();
        assert!(
            !info2.formats.is_empty(),
            "body-sniff expansion produces at least 1 Format"
        );
        for f in &info2.formats {
            assert_eq!(
                f.protocol,
                DownloadProtocol::HttpDashSegments,
                "body-sniffed MPD must also use HttpDashSegments protocol"
            );
            assert!(
                f.fragments.is_some(),
                "body-sniffed fragments must be pre-resolved"
            );
        }
    }

    /// Regression guard for #258 — confirm the generic extractor's HLS
    /// emission paths (both `try_direct_media` for direct .m3u8 URLs and
    /// the strategy-detected path) feed through `expand_hls_in_place` and
    /// produce per-variant rows with `Format.fragments` populated.
    ///
    /// The wiring in `extract` is what we're locking down: a regression
    /// that removes either expand call would leave HLS rows with
    /// `fragments = None`, so this test on the helper-output shape catches
    /// both.
    ///
    /// Strategy-side: synthetic Format vec mirrors what `detected_to_format`
    /// would emit for an HLS detection (M3u8 protocol seeded from extension
    /// inference per `protocol_from_url`). Same shape as the other extractor
    /// wiring tests added for #258.
    #[tokio::test]
    async fn generic_hls_row_expanded_and_https_pass_through() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/generic-master.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let hls_url = format!("{}/generic-master.m3u8", server.url());
        let hls = Format::new("generic-hls", &hls_url, "m3u8", DownloadProtocol::M3u8);
        let mp4 = Format::new(
            "generic-mp4",
            "https://cdn.example.com/v.mp4",
            "mp4",
            DownloadProtocol::Https,
        );

        let formats = vec![hls, mp4];
        let http = std::sync::Arc::new(wreq::Client::new());
        let expanded = crate::hls::expand_hls_in_place(formats, http).await;

        assert_eq!(expanded.len(), 2);
        assert!(
            expanded[0].fragments.is_some(),
            "M3u8 row must carry pre-resolved fragments after expand"
        );
        assert_eq!(expanded[0].fragments.as_ref().unwrap().len(), 2);
        assert!(
            expanded[1].fragments.is_none(),
            "Https MP4 row must pass through untouched"
        );
        assert_eq!(expanded[1].url, "https://cdn.example.com/v.mp4");
    }
}
