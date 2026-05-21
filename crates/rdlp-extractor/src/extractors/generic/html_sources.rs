//! HTML5 `<video>`, `<source>`, `<embed>`, and `<object>` element extraction.

use scraper::Selector;
use std::sync::LazyLock;

use super::detection::{
    Confidence, DetectedFormat, DetectionStrategy, PageContext, ext_from_url, resolve_url,
};

// ============================================================================
// Selectors
// ============================================================================

static VIDEO_SELECTOR: LazyLock<Selector> = crate::static_selector!("video");

static SOURCE_SELECTOR: LazyLock<Selector> = crate::static_selector!("source");

static IFRAME_SELECTOR: LazyLock<Selector> = crate::static_selector!("iframe");

// ============================================================================
// HTML5 Video/Source Strategy
// ============================================================================

pub(crate) struct Html5VideoStrategy;

impl DetectionStrategy for Html5VideoStrategy {
    fn name(&self) -> &'static str {
        "HTML5Video"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();

        for video_elem in ctx.html.select(&VIDEO_SELECTOR) {
            // Check <video src="..."> directly
            if let Some(src) = video_elem.value().attr("src")
                && let Some(url) = resolve_url(ctx.base_url, src)
            {
                formats.push(DetectedFormat {
                    ext: ext_from_url(&url),
                    url,
                    quality: None,
                    confidence: Confidence::Medium,
                    source: "video[src]",
                });
            }

            // Check child <source> elements
            for source_elem in video_elem.select(&SOURCE_SELECTOR) {
                if let Some(src) = source_elem.value().attr("src")
                    && let Some(url) = resolve_url(ctx.base_url, src)
                {
                    let ext = source_elem
                        .value()
                        .attr("type")
                        .and_then(mime_to_ext)
                        .or_else(|| ext_from_url(&url));

                    let quality = source_elem
                        .value()
                        .attr("label")
                        .or_else(|| source_elem.value().attr("data-quality"))
                        .map(|s| s.to_string());

                    formats.push(DetectedFormat {
                        url,
                        ext,
                        quality,
                        confidence: Confidence::Medium,
                        source: "video>source",
                    });
                }
            }
        }

        // Also check top-level <source> elements not inside <video>
        // (some sites use <source> outside <video> in custom players)
        for source_elem in ctx.html.select(&SOURCE_SELECTOR) {
            if let Some(src) = source_elem.value().attr("src")
                && let Some(url) = resolve_url(ctx.base_url, src)
            {
                // Only add if not already found
                if !formats.iter().any(|f| f.url == url) {
                    let ext = source_elem
                        .value()
                        .attr("type")
                        .and_then(mime_to_ext)
                        .or_else(|| ext_from_url(&url));
                    formats.push(DetectedFormat {
                        url,
                        ext,
                        quality: None,
                        confidence: Confidence::Low,
                        source: "source",
                    });
                }
            }
        }

        formats
    }
}

// ============================================================================
// Iframe Embed Strategy (advisory)
// ============================================================================

/// Known video hosting domains for iframe embed detection.
const KNOWN_VIDEO_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "player.vimeo.com",
    "dailymotion.com",
    "twitch.tv",
    "facebook.com",
    "streamable.com",
    "vidyard.com",
    "wistia.com",
    "brightcove.net",
];

pub(crate) struct IframeEmbedStrategy;

impl DetectionStrategy for IframeEmbedStrategy {
    fn name(&self) -> &'static str {
        "IframeEmbed"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        // This strategy doesn't return formats — it logs advisory messages
        // about embedded players that the user could extract directly.
        for iframe_elem in ctx.html.select(&IFRAME_SELECTOR) {
            if let Some(src) = iframe_elem.value().attr("src") {
                for host in KNOWN_VIDEO_HOSTS {
                    if src.contains(host) {
                        log::info!(
                            "Found embedded player from {} — try using that URL directly (iframe: {})",
                            host,
                            src
                        );
                        break;
                    }
                }
            }
        }

        // Advisory only — no formats returned
        Vec::new()
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Convert a MIME type to a file extension.
fn mime_to_ext(mime: &str) -> Option<String> {
    let mime = mime.to_lowercase();
    match mime.as_str() {
        "video/mp4" => Some("mp4".to_string()),
        "video/webm" | "audio/webm" => Some("webm".to_string()),
        "video/ogg" | "audio/ogg" => Some("ogg".to_string()),
        "video/x-flv" => Some("flv".to_string()),
        "video/x-matroska" => Some("mkv".to_string()),
        "video/quicktime" => Some("mov".to_string()),
        "video/mp2t" | "video/MP2T" => Some("ts".to_string()),
        "audio/mpeg" => Some("mp3".to_string()),
        "audio/mp4" => Some("m4a".to_string()),
        "audio/wav" => Some("wav".to_string()),
        "audio/flac" => Some("flac".to_string()),
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => Some("m3u8".to_string()),
        "application/dash+xml" => Some("mpd".to_string()),
        _ => None,
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
    fn video_src_extracted() {
        let raw = r#"<html><body>
            <video src="https://cdn.example.com/video.mp4"></video>
        </body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = Html5VideoStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
        assert_eq!(formats[0].ext, Some("mp4".to_string()));
    }

    #[test]
    fn video_source_children_extracted() {
        let raw = r#"<html><body>
            <video>
                <source src="/video_720.mp4" type="video/mp4" label="720p">
                <source src="/video_1080.webm" type="video/webm" label="1080p">
            </video>
        </body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = Html5VideoStrategy.detect(&ctx);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].url, "https://example.com/video_720.mp4");
        assert_eq!(formats[0].ext, Some("mp4".to_string()));
        assert_eq!(formats[0].quality, Some("720p".to_string()));
        assert_eq!(formats[1].url, "https://example.com/video_1080.webm");
        assert_eq!(formats[1].ext, Some("webm".to_string()));
        assert_eq!(formats[1].quality, Some("1080p".to_string()));
    }

    #[test]
    fn relative_url_resolved() {
        let raw = r#"<html><body>
            <video src="/videos/clip.mp4"></video>
        </body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = Html5VideoStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://example.com/videos/clip.mp4");
    }

    #[test]
    fn no_video_elements_returns_empty() {
        let raw = r#"<html><body><p>No video here</p></body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(Html5VideoStrategy.detect(&ctx).is_empty());
    }

    #[test]
    fn iframe_embed_is_advisory_only() {
        let raw = r#"<html><body>
            <iframe src="https://www.youtube.com/embed/dQw4w9WgXcQ"></iframe>
        </body></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        // IframeEmbedStrategy logs but returns no formats
        let formats = IframeEmbedStrategy.detect(&ctx);
        assert!(formats.is_empty());
    }

    #[test]
    fn mime_to_ext_works() {
        assert_eq!(mime_to_ext("video/mp4"), Some("mp4".to_string()));
        assert_eq!(mime_to_ext("video/webm"), Some("webm".to_string()));
        assert_eq!(
            mime_to_ext("application/vnd.apple.mpegurl"),
            Some("m3u8".to_string())
        );
        assert_eq!(mime_to_ext("text/html"), None);
    }
}
