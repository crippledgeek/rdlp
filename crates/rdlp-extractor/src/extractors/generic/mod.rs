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
    run_detection_pipeline, DetectedFormat, DetectionStrategy, PageContext, ext_from_url,
};
use self::direct::{prefetch, title_from_url, PrefetchResponse};
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
        log::info!("No site-specific extractor for {}, trying generic extraction", domain);

        // === Phase 1: Prefetch (512 bytes) ===
        let prefetch_result = prefetch(url, ctx).await.ok();

        // Check for direct media URL
        if let Some(ref pf) = prefetch_result
            && let Some(info) = try_direct_media(url, pf)? {
                return Ok(info);
            }

        // === Phase 2: Fetch full page ===
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Truncate oversized pages
        let webpage = if webpage.len() > MAX_PAGE_SIZE {
            log::warn!(
                "Page exceeds size limit ({} bytes, max {}), truncating",
                webpage.len(),
                MAX_PAGE_SIZE
            );
            webpage[..MAX_PAGE_SIZE].to_string()
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

        Ok(info)
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Try to handle a direct media URL from the prefetch response.
fn try_direct_media(url: &str, pf: &PrefetchResponse) -> Result<Option<InfoDict>> {
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

    static BASE_SELECTOR: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("base[href]").expect("valid base selector"));

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
        assert_eq!(
            content_type_to_ext("video/mp4; codecs=avc1"),
            Some("mp4")
        );
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
}
