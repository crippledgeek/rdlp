//! SpankBang extractor.
//!
//! Single-request extraction: fetches the video page with the `country=US`
//! cookie, parses the inline `stream_data = {...}` Python-dict for formats
//! and metadata. Falls back to the formats API (POST `/api/videos/stream`)
//! when the inline dict is absent (rare on current pages).
//!
//! Cloudflare-fronted; the wreq+BoringSSL browser-emulation HTTP client
//! shipped by `rdlp-http` is required (default in the production stack).
//!
//! Reference: yt-dlp `yt_dlp/extractor/spankbang.py` — uses `impersonate=True`
//! for the equivalent calls.

mod formats;
mod metadata;
mod patterns;
mod search;

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::{Format, InfoDict};
use regex::Regex;
use serde_json::Value;

use crate::base::common::BaseExtractor;

const SPANKBANG_NAME: &str = "SpankBang";
const SPANKBANG_PRIORITY: i32 = 100;
const FORMATS_API_URL: &str = "https://spankbang.com/api/videos/stream";

/// Post-process a `Vec<Format>`, replacing M3u8 entries with the per-variant
/// rows produced by [`crate::hls::expand_hls_url`]. On expand failure the
/// original Format row is kept so the legacy variant-URL path still handles
/// risky playlists (encrypted, live, byte-range init, multi-init, etc.).
async fn expand_hls_in_place(
    formats: Vec<rdlp_types::Format>,
    http: std::sync::Arc<wreq::Client>,
) -> Vec<rdlp_types::Format> {
    use rdlp_types::DownloadProtocol;
    let mut expanded = Vec::with_capacity(formats.len());
    for f in formats {
        if matches!(f.protocol, DownloadProtocol::M3u8) {
            match crate::hls::expand_hls_url(&f, std::sync::Arc::clone(&http)).await {
                Ok(rows) => expanded.extend(rows),
                Err(e) => {
                    log::warn!(
                        "HLS expand failed for {} ({e}) — falling back to legacy variant-URL path",
                        f.url
                    );
                    expanded.push(f);
                }
            }
        } else {
            expanded.push(f);
        }
    }
    expanded
}

/// SpankBang site extractor.
#[derive(Default)]
pub struct SpankBangExtractor;

impl SpankBangExtractor {
    /// Construct a new extractor instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Fetch the formats-API JSON when the inline `stream_data` is absent.
    /// Caller passes the streamkey already extracted from the page HTML.
    async fn fetch_formats_api(
        ctx: &ExtractionContext,
        streamkey: &str,
        page_url: &str,
    ) -> Result<Value> {
        let body = format!("id={}&data=0", urlencoding::encode(streamkey));
        let resp = ctx
            .http_client
            .post(FORMATS_API_URL)
            .header("Referer", page_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Origin", "https://spankbang.com")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| RdlpError::Network {
                message: format!("SpankBang formats API request failed: {e:#}"),
                url: Some(FORMATS_API_URL.to_string()),
            })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(RdlpError::Http {
                status: status.as_u16(),
                reason: format!("SpankBang formats API returned {status}"),
            });
        }

        resp.json::<Value>()
            .await
            .map_err(|e| RdlpError::Extraction {
                message: format!("SpankBang formats API JSON parse failed: {e:#}"),
                url: Some(FORMATS_API_URL.to_string()),
            })
    }
}

#[async_trait]
impl InfoExtractor for SpankBangExtractor {
    fn name(&self) -> &str {
        SPANKBANG_NAME
    }

    fn valid_url(&self) -> &Regex {
        &patterns::VIDEO_URL
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        SPANKBANG_PRIORITY
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let video_id = patterns::extract_video_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!("SpankBang: could not extract video ID from URL: {url}"),
            url: Some(url.to_string()),
        })?;

        // Playlist URLs match VIDEO_URL but require a different fetch path
        // (anchor scrape, not stream_data parse). Surface a clear error
        // until extract_playlist is implemented as a follow-up.
        if url.contains("/playlist/") {
            return Err(RdlpError::Extraction {
                message: "SpankBang playlist extraction is not yet implemented".to_string(),
                url: Some(url.to_string()),
            });
        }

        // yt-dlp parity: rewrite /<id>/embed → /<id>/video before fetching.
        let canonical = url.replace(&format!("/{video_id}/embed"), &format!("/{video_id}/video"));
        debug!("[spankbang] fetching video page id={video_id}");

        // SpankBang gates some content by country; matches yt-dlp's default.
        let webpage =
            BaseExtractor::fetch_webpage_with_headers(&canonical, &[("Cookie", "country=US")], ctx)
                .await?;

        if metadata::is_removed(&webpage) {
            return Err(RdlpError::Extraction {
                message: format!("SpankBang: video {video_id} is not available"),
                url: Some(url.to_string()),
            });
        }

        // --- formats: inline stream_data first, API fallback second ---
        let mut formats: Vec<Format> = Vec::new();
        if let Some(data) = formats::parse_inline_stream_data(&webpage) {
            formats = formats::build_formats(&data);
            debug!(
                "[spankbang] inline stream_data produced {} formats",
                formats.len()
            );
            formats = expand_hls_in_place(formats, std::sync::Arc::clone(&ctx.http_client)).await;
        }

        if formats.is_empty() {
            warn!("[spankbang] inline stream_data missing, falling back to formats API");
            let key = formats::parse_streamkey(&webpage).ok_or_else(|| RdlpError::Extraction {
                message: format!(
                    "SpankBang: neither inline stream_data nor data-streamkey present \
                     on page {url}"
                ),
                url: Some(url.to_string()),
            })?;
            let data = Self::fetch_formats_api(ctx, &key, &canonical).await?;
            formats = formats::build_formats(&data);
            formats = expand_hls_in_place(formats, std::sync::Arc::clone(&ctx.http_client)).await;
        }

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!("SpankBang: no playable formats found for {url}"),
                url: Some(url.to_string()),
            });
        }

        // Validate every URL through the SSRF gate before returning.
        for f in &formats {
            BaseExtractor::validate_url_security(&f.url)?;
        }

        // --- metadata ---
        let meta = metadata::parse(&webpage);
        let title = meta.title.clone().unwrap_or_else(|| video_id.clone());

        let mut info = InfoDict::new(&video_id, &title, SPANKBANG_NAME, url);
        info.description = meta.description;
        info.thumbnail = meta.thumbnail;
        info.uploader = meta.uploader;
        info.uploader_id = meta.uploader_id;
        info.duration = meta.duration_secs.map(|s| s as f64);
        info.age_limit = Some(18);
        info.actors = meta.actors;
        if !meta.tags.is_empty() {
            info.tags = Some(meta.tags);
        }
        info.formats = formats;
        info.propagate_duration();

        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_routing_smoke() {
        let ext = SpankBangExtractor::new();
        assert!(ext.suitable("https://spankbang.com/56b3d/video/x"));
        assert!(ext.suitable("https://m.spankbang.com/3vvn/play"));
        assert!(!ext.suitable("https://www.xnxx.com/video-14cco143/y"));
    }

    #[test]
    fn priority_above_generic_below_explicit_default() {
        let ext = SpankBangExtractor::new();
        assert_eq!(ext.priority(), 100);
    }

    #[test]
    fn name_matches_constant() {
        let ext = SpankBangExtractor::new();
        assert_eq!(ext.name(), "SpankBang");
    }

    #[tokio::test]
    async fn expand_hls_in_place_replaces_m3u8_rows_with_fragments() {
        use rdlp_types::{DownloadProtocol, Format};

        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/v.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let url = format!("{}/v.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = super::expand_hls_in_place(vec![f], http).await;
        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_some());
        assert_eq!(out[0].fragments.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn expand_hls_in_place_preserves_non_m3u8_rows() {
        use rdlp_types::{DownloadProtocol, Format};

        let mp4 = Format::new(
            "1080p",
            "https://h.com/x.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        let http = std::sync::Arc::new(wreq::Client::new());
        let out = super::expand_hls_in_place(vec![mp4.clone()], http).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].format_id, "1080p");
        assert!(out[0].fragments.is_none(), "MP4 row untouched");
    }

    #[tokio::test]
    async fn expand_hls_in_place_keeps_original_on_encrypted() {
        use rdlp_types::{DownloadProtocol, Format};

        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/enc.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXT-X-KEY:METHOD=AES-128,URI=\"https://h.com/key\"\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let url = format!("{}/enc.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = super::expand_hls_in_place(vec![f.clone()], http).await;
        assert_eq!(out.len(), 1, "graceful fallback keeps original");
        assert_eq!(out[0].url, f.url);
        assert!(out[0].fragments.is_none(), "no fragments on fallback");
    }

    #[tokio::test]
    async fn expand_hls_in_place_keeps_original_on_live() {
        use rdlp_types::{DownloadProtocol, Format};

        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/live.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n", // no #EXT-X-ENDLIST
            )
            .create_async()
            .await;

        let url = format!("{}/live.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = super::expand_hls_in_place(vec![f.clone()], http).await;
        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_none());
    }
}
