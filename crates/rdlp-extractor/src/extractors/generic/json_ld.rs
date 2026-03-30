//! JSON-LD `VideoObject` detection strategy.
//!
//! Uses the shared `base::common::json_ld` module for parsing, and wraps
//! the results as `DetectedFormat` entries for the generic pipeline.

use crate::base::common::json_ld as shared;

use super::detection::{
    Confidence, DetectedFormat, DetectionStrategy, PageContext, ext_from_url, resolve_url,
};

// ============================================================================
// Extracted metadata (passed back to the main extractor)
// ============================================================================

/// Metadata extracted from JSON-LD, beyond just formats.
#[derive(Debug, Default)]
pub(crate) struct JsonLdMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub thumbnail: Option<String>,
    pub duration_seconds: Option<f64>,
    pub upload_date: Option<String>,
}

// ============================================================================
// Strategy
// ============================================================================

pub(crate) struct JsonLdStrategy;

impl JsonLdStrategy {
    /// Extract metadata from JSON-LD in addition to formats.
    pub(crate) fn extract_metadata(ctx: &PageContext<'_>) -> JsonLdMetadata {
        if let Some(video) = shared::extract_json_ld(ctx.html) {
            // Extract borrowed fields before moving owned fields out
            let thumbnail = shared::get_thumbnail_url(&video);
            let duration_seconds = video
                .duration
                .as_deref()
                .and_then(shared::parse_iso8601_duration);
            JsonLdMetadata {
                title: video.name,
                description: video.description,
                thumbnail,
                duration_seconds,
                upload_date: video.upload_date,
            }
        } else {
            JsonLdMetadata::default()
        }
    }
}

impl DetectionStrategy for JsonLdStrategy {
    fn name(&self) -> &'static str {
        "JSON-LD"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        let mut formats = Vec::new();

        if let Some(video) = shared::extract_json_ld(ctx.html) {
            // Prefer contentUrl over embedUrl
            if let Some(url) = video
                .content_url
                .as_deref()
                .and_then(|u| resolve_url(ctx.base_url, u))
            {
                formats.push(DetectedFormat {
                    ext: ext_from_url(&url),
                    url,
                    quality: None,
                    confidence: Confidence::High,
                    source: "json-ld:contentUrl",
                });
            }
            if let Some(url) = video
                .embed_url
                .as_deref()
                .and_then(|u| resolve_url(ctx.base_url, u))
            {
                // embedUrl is often an iframe, not a direct media link
                formats.push(DetectedFormat {
                    ext: ext_from_url(&url),
                    url,
                    quality: None,
                    confidence: Confidence::Low,
                    source: "json-ld:embedUrl",
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
    fn json_ld_content_url_extracted() {
        let raw = r#"<html><head><script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test Video",
            "contentUrl": "https://cdn.example.com/video.mp4"
        }
        </script></head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = JsonLdStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
        assert_eq!(formats[0].confidence, Confidence::High);
        assert_eq!(formats[0].source, "json-ld:contentUrl");
    }

    #[test]
    fn json_ld_embed_url_fallback() {
        let raw = r#"<html><head><script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test Video",
            "embedUrl": "https://example.com/embed/123"
        }
        </script></head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = JsonLdStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://example.com/embed/123");
        assert_eq!(formats[0].confidence, Confidence::Low);
    }

    #[test]
    fn json_ld_graph_array() {
        let raw = r#"<html><head><script type="application/ld+json">
        {
            "@graph": [
                {"@type": "WebPage", "name": "Page"},
                {"@type": "VideoObject", "name": "Video", "contentUrl": "https://cdn.example.com/v.mp4"}
            ]
        }
        </script></head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = JsonLdStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/v.mp4");
    }

    #[test]
    fn json_ld_metadata_extraction() {
        let raw = r#"<html><head><script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "My Video",
            "description": "A great video",
            "thumbnailUrl": "https://example.com/thumb.jpg",
            "duration": "PT1H2M30S",
            "uploadDate": "2026-01-15"
        }
        </script></head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let meta = JsonLdStrategy::extract_metadata(&ctx);
        assert_eq!(meta.title.as_deref(), Some("My Video"));
        assert_eq!(meta.description.as_deref(), Some("A great video"));
        assert_eq!(
            meta.thumbnail.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
        assert_eq!(meta.duration_seconds, Some(3750.0));
        assert_eq!(meta.upload_date.as_deref(), Some("2026-01-15"));
    }

    #[test]
    fn json_ld_non_video_ignored() {
        let raw = r#"<html><head><script type="application/ld+json">
        {"@type": "Article", "name": "Not a video"}
        </script></head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(JsonLdStrategy.detect(&ctx).is_empty());
    }
}
