//! JavaScript-based player detection strategies.
//!
//! Scans page source for JW Player, KVS Player, Video.js, and generic
//! `file=`/`source=` parameters in inline JavaScript.

use regex::Regex;
use std::sync::LazyLock;

use super::detection::{
    Confidence, DetectedFormat, DetectionStrategy, PageContext, ext_from_url, resolve_url,
};

// ============================================================================
// Regex Patterns
// ============================================================================

// JW Player patterns
static JW_SETUP_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"jwplayer\s*\(\s*["']?\w+["']?\s*\)\s*\.\s*setup\s*\(\s*\{[\s\S]*?["']?file["']?\s*:\s*["']([^"']+)["']"#)
        .expect("valid JW Player setup regex")
});

static JW_PLAYER_OPTIONS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:JWPlayerOptions|jwConfig|playerInstance)\s*=\s*\{[\s\S]*?["']?file["']?\s*:\s*["']([^"']+)["']"#,
    )
    .expect("valid JW Player options regex")
});

static JW_FLASHVARS_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"flashvars\s*[=:]\s*[{"][\s\S]*?["']?file["']?\s*[=:]\s*["']([^"']+)["']"#)
        .expect("valid flashvars file regex")
});

// Video.js patterns
static VIDEOJS_SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"videojs\s*\(\s*["']?\w+["']?[\s\S]*?sources\s*:\s*\[\s*\{[\s\S]*?src\s*:\s*["']([^"']+)["']"#)
        .expect("valid Video.js source regex")
});

static VIDEOJS_DATA_SETUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-setup\s*=\s*'[\s\S]*?"src"\s*:\s*"([^"]+)""#)
        .expect("valid Video.js data-setup regex")
});

// Generic JS parameters
static GENERIC_FILE_PARAM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:file|source|video_url|videoUrl|videoSrc|mp4|hls_url)\s*[:=]\s*["'](https?://[^"'\s]+\.(?:mp4|m3u8|webm|flv|mkv|mov)(?:\?[^"'\s]*)?)["']"#,
    )
    .expect("valid generic file param regex")
});

// ============================================================================
// JW Player Strategy
// ============================================================================

pub(crate) struct JwPlayerStrategy;

impl DetectionStrategy for JwPlayerStrategy {
    fn name(&self) -> &'static str {
        "JWPlayer"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // jwplayer().setup({file: "..."})
        for caps in JW_SETUP_FILE.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                add_format(
                    &mut formats,
                    &mut seen,
                    url_match.as_str(),
                    ctx,
                    "jwplayer.setup",
                );
            }
        }

        // JWPlayerOptions/jwConfig = {file: "..."}
        for caps in JW_PLAYER_OPTIONS.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                add_format(
                    &mut formats,
                    &mut seen,
                    url_match.as_str(),
                    ctx,
                    "jwplayer.options",
                );
            }
        }

        // flashvars with file key
        for caps in JW_FLASHVARS_FILE.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                add_format(
                    &mut formats,
                    &mut seen,
                    url_match.as_str(),
                    ctx,
                    "jwplayer.flashvars",
                );
            }
        }

        formats
    }
}

// ============================================================================
// KVS Player Strategy (delegates to shared base::kvs module)
// ============================================================================

pub(crate) struct KvsPlayerStrategy;

impl DetectionStrategy for KvsPlayerStrategy {
    fn name(&self) -> &'static str {
        "KVSPlayer"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        use crate::base::kvs;

        // Only activate if kt_player.js is found on the page
        if !kvs::is_kvs_page(ctx.raw_html) {
            return Vec::new();
        }

        kvs::extract_kvs_formats(ctx.raw_html)
            .into_iter()
            .filter_map(|kf| {
                resolve_url(ctx.base_url, &kf.url).map(|url| DetectedFormat {
                    ext: ext_from_url(&url),
                    url,
                    quality: kf.quality,
                    confidence: Confidence::Medium,
                    source: if kf.is_primary {
                        "kvs.video_url"
                    } else {
                        "kvs.video_alt_url"
                    },
                })
            })
            .collect()
    }
}

// ============================================================================
// Video.js Strategy
// ============================================================================

pub(crate) struct VideoJsStrategy;

impl DetectionStrategy for VideoJsStrategy {
    fn name(&self) -> &'static str {
        "VideoJs"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // videojs() init with sources
        for caps in VIDEOJS_SOURCE.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                add_format(&mut formats, &mut seen, url_match.as_str(), ctx, "videojs");
            }
        }

        // data-setup attribute
        for caps in VIDEOJS_DATA_SETUP.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                add_format(
                    &mut formats,
                    &mut seen,
                    url_match.as_str(),
                    ctx,
                    "videojs.data-setup",
                );
            }
        }

        formats
    }
}

// ============================================================================
// Generic JS Params Strategy
// ============================================================================

pub(crate) struct GenericJsParamsStrategy;

impl DetectionStrategy for GenericJsParamsStrategy {
    fn name(&self) -> &'static str {
        "GenericJsParams"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for caps in GENERIC_FILE_PARAM.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                let raw = url_match.as_str();
                if let Some(url) = resolve_url(ctx.base_url, raw)
                    && seen.insert(url.clone())
                {
                    formats.push(DetectedFormat {
                        ext: ext_from_url(&url),
                        url,
                        quality: None,
                        confidence: Confidence::Low,
                        source: "js.param",
                    });
                }
            }
        }

        formats
    }
}

// ============================================================================
// Direct Link Scan Strategy
// ============================================================================

/// Last-resort scan: regex for media URLs anywhere in page source.
static DIRECT_LINK_SCAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"["'](https?://[^"'\s]+\.(?:mp4|m3u8|webm)(?:\?[^"'\s]*)?)["']"#)
        .expect("valid direct link scan regex")
});

pub(crate) struct DirectLinkScanStrategy;

impl DetectionStrategy for DirectLinkScanStrategy {
    fn name(&self) -> &'static str {
        "DirectLinkScan"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for caps in DIRECT_LINK_SCAN.captures_iter(ctx.raw_html) {
            if let Some(url_match) = caps.get(1) {
                let url = url_match.as_str().to_string();
                // Filter out obvious non-media URLs (thumbnails, tracking, etc.)
                if is_likely_media_url(&url) && seen.insert(url.clone()) {
                    formats.push(DetectedFormat {
                        ext: ext_from_url(&url),
                        url,
                        quality: None,
                        confidence: Confidence::Low,
                        source: "link_scan",
                    });
                }
            }
        }

        formats
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn add_format(
    formats: &mut Vec<DetectedFormat>,
    seen: &mut std::collections::HashSet<String>,
    raw_url: &str,
    ctx: &PageContext<'_>,
    source: &'static str,
) {
    if let Some(url) = resolve_url(ctx.base_url, raw_url)
        && seen.insert(url.clone())
    {
        formats.push(DetectedFormat {
            ext: ext_from_url(&url),
            url,
            quality: None,
            confidence: Confidence::Medium,
            source,
        });
    }
}

/// Filter out URLs that are unlikely to be actual media content.
fn is_likely_media_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    // Reject thumbnail/poster image patterns
    if lower.contains("/thumb") || lower.contains("/poster") || lower.contains("/preview") {
        return false;
    }
    // Reject tracking/analytics pixels
    if lower.contains("/pixel") || lower.contains("/beacon") || lower.contains("/track") {
        return false;
    }
    // Reject very short URLs (likely not real media)
    url.len() > 30
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::Html;
    use url::Url;

    fn make_ctx<'a>(html: &'a Html, raw: &'a str, url: &'a Url) -> PageContext<'a> {
        PageContext {
            url,
            base_url: url,
            html,
            raw_html: raw,
        }
    }

    #[test]
    fn jw_player_setup_detected() {
        let raw = r#"<html><body><script>
            jwplayer("player").setup({
                file: "https://cdn.example.com/video.mp4",
                width: 640
            });
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = JwPlayerStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
        assert_eq!(formats[0].confidence, Confidence::Medium);
    }

    #[test]
    fn jw_player_options_detected() {
        let raw = r#"<html><body><script>
            var JWPlayerOptions = {
                file: "https://cdn.example.com/stream.m3u8",
                autostart: true
            };
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = JwPlayerStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/stream.m3u8");
    }

    #[test]
    fn jw_flashvars_detected() {
        let raw = r#"<html><body><script>
            var flashvars = {
                file: "https://cdn.example.com/video.flv"
            };
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = JwPlayerStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.flv");
    }

    #[test]
    fn kvs_player_detected_only_with_script() {
        // KVS flashvars use single-quoted values (key: 'value' syntax)
        let raw = r#"<html><body>
            <script src="/js/kt_player.js"></script>
            <script>
                var flashvars = {
                    video_url: 'https://cdn.example.com/kvs_video.mp4'
                };
            </script>
        </body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = KvsPlayerStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/kvs_video.mp4");
    }

    #[test]
    fn kvs_player_not_detected_without_script() {
        let raw = r#"<html><body><script>
            var video_url = "https://cdn.example.com/video.mp4";
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        // Without kt_player.js, KVS strategy should not activate
        assert!(KvsPlayerStrategy.detect(&ctx).is_empty());
    }

    #[test]
    fn videojs_source_detected() {
        let raw = r#"<html><body><script>
            videojs("my-video", {
                sources: [{ src: "https://cdn.example.com/video.mp4", type: "video/mp4" }]
            });
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = VideoJsStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
    }

    #[test]
    fn generic_js_file_param_detected() {
        let raw = r#"<html><body><script>
            var config = {
                file: "https://cdn.example.com/content/video.mp4"
            };
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = GenericJsParamsStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/content/video.mp4");
        assert_eq!(formats[0].confidence, Confidence::Low);
    }

    #[test]
    fn direct_link_scan_finds_urls() {
        let raw = r#"<html><body><script>
            var data = {"url": "https://cdn.example.com/media/full-video.mp4"};
        </script></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = DirectLinkScanStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].confidence, Confidence::Low);
    }

    #[test]
    fn direct_link_scan_filters_thumbnails() {
        let raw = r#"<html><body>
            "https://cdn.example.com/thumbs/thumbnail.mp4"
        </body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(DirectLinkScanStrategy.detect(&ctx).is_empty());
    }

    #[test]
    fn no_js_returns_empty() {
        let raw = r#"<html><body><p>Just text</p></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(JwPlayerStrategy.detect(&ctx).is_empty());
        assert!(KvsPlayerStrategy.detect(&ctx).is_empty());
        assert!(VideoJsStrategy.detect(&ctx).is_empty());
        assert!(GenericJsParamsStrategy.detect(&ctx).is_empty());
    }
}
