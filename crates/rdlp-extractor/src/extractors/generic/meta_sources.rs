//! OpenGraph and Twitter meta tag detection strategies.

use scraper::Selector;
use std::sync::LazyLock;

use super::detection::{Confidence, DetectedFormat, DetectionStrategy, PageContext, resolve_url};

// ============================================================================
// Selectors
// ============================================================================

static OG_VIDEO_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[property="og:video"], meta[property="og:video:url"], meta[property="og:video:secure_url"]"#)
        .expect("valid og:video selector")
});

static OG_AUDIO_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[property="og:audio"], meta[property="og:audio:url"], meta[property="og:audio:secure_url"]"#)
        .expect("valid og:audio selector")
});

static OG_VIDEO_TYPE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[property="og:video:type"]"#).expect("valid og:video:type selector")
});

static TWITTER_PLAYER_STREAM_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[name="twitter:player:stream"], meta[property="twitter:player:stream"]"#)
        .expect("valid twitter:player:stream selector")
});

// ============================================================================
// OpenGraph Strategy
// ============================================================================

pub(crate) struct OpenGraphStrategy;

impl DetectionStrategy for OpenGraphStrategy {
    fn name(&self) -> &'static str {
        "OpenGraph"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();

        // og:video:secure_url takes priority (listed first in selector)
        for elem in ctx.html.select(&OG_VIDEO_SELECTOR) {
            if let Some(content) = elem.value().attr("content")
                && let Some(url) = resolve_url(ctx.base_url, content) {
                    let ext = super::detection::ext_from_url(&url);
                    formats.push(DetectedFormat {
                        url,
                        ext,
                        quality: None,
                        confidence: Confidence::High,
                        source: "og:video",
                    });
                }
        }

        // og:audio
        for elem in ctx.html.select(&OG_AUDIO_SELECTOR) {
            if let Some(content) = elem.value().attr("content")
                && let Some(url) = resolve_url(ctx.base_url, content) {
                    let ext = super::detection::ext_from_url(&url);
                    formats.push(DetectedFormat {
                        url,
                        ext,
                        quality: None,
                        confidence: Confidence::High,
                        source: "og:audio",
                    });
                }
        }

        // Check og:video:type for MIME hint
        if let Some(type_elem) = ctx.html.select(&OG_VIDEO_TYPE_SELECTOR).next()
            && let Some(mime) = type_elem.value().attr("content") {
                // If type indicates a Flash embed (not direct media), lower confidence
                if mime.contains("flash") || mime.contains("shockwave") {
                    for f in &mut formats {
                        if f.source == "og:video" {
                            f.confidence = Confidence::Low;
                        }
                    }
                }
            }

        formats
    }
}

// ============================================================================
// Twitter Player Strategy
// ============================================================================

pub(crate) struct TwitterPlayerStrategy;

impl DetectionStrategy for TwitterPlayerStrategy {
    fn name(&self) -> &'static str {
        "TwitterPlayer"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();

        for elem in ctx.html.select(&TWITTER_PLAYER_STREAM_SELECTOR) {
            if let Some(content) = elem.value().attr("content")
                && let Some(url) = resolve_url(ctx.base_url, content) {
                    let ext = super::detection::ext_from_url(&url);
                    formats.push(DetectedFormat {
                        url,
                        ext,
                        quality: None,
                        confidence: Confidence::Medium,
                        source: "twitter:player:stream",
                    });
                }
        }

        formats
    }
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
    fn og_video_url_extracted() {
        let raw = r#"<html><head>
            <meta property="og:video:url" content="https://cdn.example.com/video.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
        assert_eq!(formats[0].confidence, Confidence::High);
    }

    #[test]
    fn og_video_secure_url_extracted() {
        let raw = r#"<html><head>
            <meta property="og:video:secure_url" content="https://cdn.example.com/secure.mp4">
            <meta property="og:video:url" content="http://cdn.example.com/video.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 2);
        // Both are extracted; dedup happens at pipeline level
        assert!(formats
            .iter()
            .any(|f| f.url == "https://cdn.example.com/secure.mp4"));
    }

    #[test]
    fn og_flash_type_lowers_confidence() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://example.com/player.swf">
            <meta property="og:video:type" content="application/x-shockwave-flash">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].confidence, Confidence::Low);
    }

    #[test]
    fn og_audio_extracted() {
        let raw = r#"<html><head>
            <meta property="og:audio" content="https://cdn.example.com/audio.mp3">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/audio.mp3");
        assert_eq!(formats[0].source, "og:audio");
    }

    #[test]
    fn twitter_player_stream_extracted() {
        let raw = r#"<html><head>
            <meta name="twitter:player:stream" content="https://cdn.example.com/video.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = TwitterPlayerStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
        assert_eq!(formats[0].confidence, Confidence::Medium);
    }

    #[test]
    fn no_og_video_returns_empty() {
        let raw = r#"<html><head>
            <meta property="og:title" content="A Page">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(OpenGraphStrategy.detect(&ctx).is_empty());
    }
}
