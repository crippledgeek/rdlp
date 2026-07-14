//! OpenGraph and Twitter meta tag detection strategies.

use scraper::Selector;
use std::sync::LazyLock;

use super::detection::{Confidence, DetectedFormat, DetectionStrategy, PageContext, resolve_url};

// ============================================================================
// Selectors
// ============================================================================

/// Every `og:video*` / `og:audio*` meta tag, yielded in document order — which is
/// what binds a structured property to its root tag (see [`OgTag`]).
static OG_MEDIA_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[property^="og:video"], meta[property^="og:audio"]"#);

static TWITTER_PLAYER_STREAM_SELECTOR: LazyLock<Selector> = crate::static_selector!(
    r#"meta[name="twitter:player:stream"], meta[property="twitter:player:stream"]"#
);

// ============================================================================
// OpenGraph Strategy
// ============================================================================

/// The role a single `og:video*` / `og:audio*` meta tag plays while walking the
/// document.
///
/// The OpenGraph spec (<https://ogp.me>) defines structured properties by
/// position, not by name alone: "Put structured properties after you declare
/// their root tag. Whenever another root element is parsed, that structured
/// property is considered to be done and another one is started." So `:type`
/// binds to the *most recently declared* root — never to the document at large.
enum OgTag {
    /// A root tag: begins a new entry.
    Root(&'static str),
    /// The HTTPS alternate of the current root's resource — not a new entry.
    SecureUrl(&'static str),
    /// The current root's MIME type.
    Type,
}

impl OgTag {
    /// Classify a `property` attribute. Returns `None` for tags that carry no
    /// URL or type (`og:video:width`, `:height`, `:duration`, …).
    fn classify(property: &str) -> Option<Self> {
        match property {
            "og:video" | "og:video:url" => Some(Self::Root("og:video")),
            "og:audio" | "og:audio:url" => Some(Self::Root("og:audio")),
            "og:video:secure_url" => Some(Self::SecureUrl("og:video")),
            "og:audio:secure_url" => Some(Self::SecureUrl("og:audio")),
            "og:video:type" | "og:audio:type" => Some(Self::Type),
            _ => None,
        }
    }
}

/// One OpenGraph media entry: a root URL plus any `secure_url` alternate for the
/// same resource, and the MIME type declared for it.
struct OgEntry<'a> {
    source: &'static str,
    /// Root URL first, `secure_url` alternate (if any) after. Both are surfaced
    /// as candidates; pipeline dedup collapses them when identical.
    urls: Vec<&'a str>,
    mime: Option<&'a str>,
}

impl<'a> OgEntry<'a> {
    fn new(source: &'static str, url: &'a str) -> Self {
        Self {
            source,
            urls: vec![url],
            mime: None,
        }
    }

    /// Whether this entry denotes a downloadable stream.
    ///
    /// An entry with no `:type` sibling is kept — the type is merely unspecified,
    /// and pages that omit it must not regress. An entry that *declares* a
    /// non-media type is an embed/player page (`text/html`) or a plugin object
    /// (Flash), and is dropped: it is not a stream (issue #493).
    fn is_media(&self) -> bool {
        self.mime.is_none_or(super::patterns::is_media_content_type)
    }
}

pub(crate) struct OpenGraphStrategy;

impl DetectionStrategy for OpenGraphStrategy {
    fn name(&self) -> &'static str {
        "OpenGraph"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        // Walk in document order, binding each structured property to the root
        // that precedes it, per the OGP spec's array/structured-property rules.
        let mut entries: Vec<OgEntry<'_>> = Vec::new();

        for elem in ctx.html.select(&OG_MEDIA_SELECTOR) {
            let (Some(property), Some(content)) =
                (elem.value().attr("property"), elem.value().attr("content"))
            else {
                continue;
            };

            match OgTag::classify(property) {
                Some(OgTag::Root(source)) => entries.push(OgEntry::new(source, content)),
                // A `secure_url` belongs to the open root. Should a page declare
                // one with no preceding root, treat it as a root itself rather
                // than discarding a usable URL.
                Some(OgTag::SecureUrl(source)) => match entries.last_mut() {
                    Some(entry) => entry.urls.push(content),
                    None => entries.push(OgEntry::new(source, content)),
                },
                Some(OgTag::Type) => {
                    if let Some(entry) = entries.last_mut() {
                        entry.mime = Some(content);
                    }
                }
                None => {}
            }
        }

        entries
            .into_iter()
            .filter(OgEntry::is_media)
            .flat_map(|entry| {
                // A declared media type names the container even when the URL has
                // no extension to sniff — better than guessing downstream.
                let ext_from_mime = entry.mime.and_then(super::content_type_to_ext);
                entry.urls.into_iter().filter_map(move |raw| {
                    let url = resolve_url(ctx.base_url, raw)?;
                    let ext = super::detection::ext_from_url(&url)
                        .or_else(|| ext_from_mime.map(str::to_owned));
                    Some(DetectedFormat {
                        url,
                        ext,
                        quality: None,
                        confidence: Confidence::High,
                        source: entry.source,
                    })
                })
            })
            .collect()
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
                && let Some(url) = resolve_url(ctx.base_url, content)
            {
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
        assert!(
            formats
                .iter()
                .any(|f| f.url == "https://cdn.example.com/secure.mp4")
        );
    }

    /// A Flash object is a plugin embed, not a stream, so it is dropped.
    ///
    /// This previously asserted the entry survived at `Confidence::Low`. That
    /// encoded a defect: `Confidence` only orders dedup candidates (see
    /// `detection::run_detection_pipeline`) and never filters, so the demoted
    /// entry was still emitted as a format. Skipping is the behavior the demotion
    /// was reaching for (issue #493).
    #[test]
    fn og_flash_type_skipped() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://example.com/player.swf">
            <meta property="og:video:type" content="application/x-shockwave-flash">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(OpenGraphStrategy.detect(&ctx).is_empty());
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

    // ------------------------------------------------------------------
    // og:video:type media gating (issue #493)
    //
    // Per the OGP spec (https://ogp.me), `og:video` legitimately points at an
    // HTML embed/player page, signalled by `og:video:type: text/html` — YouTube
    // and friends do exactly this. Such an entry is not a downloadable stream
    // and must not become a format.
    // ------------------------------------------------------------------

    /// The #493 regression guard: an `og:video` embed page declared `text/html`
    /// must yield no format at all. Before the fix this emitted a High-confidence
    /// `mp4` pointing at an HTML page.
    #[test]
    fn og_video_html_embed_type_skipped() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://example.com/embed/353006/">
            <meta property="og:video:type" content="text/html">
            <meta property="og:video:width" content="1920">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(
            OpenGraphStrategy.detect(&ctx).is_empty(),
            "og:video:type=text/html marks an embed page, not a stream"
        );
    }

    /// Structured properties bind to the most recently declared root tag, so a
    /// page mixing an embed entry with a real media entry must keep the media
    /// one. Pre-fix, the first `og:video:type` was applied to every entry — so a
    /// naive "skip on non-media" would have wrongly dropped the real MP4 here.
    #[test]
    fn og_video_type_pairs_per_entry_not_globally() {
        let raw = r#"<html><head>
            <meta property="og:video:url" content="https://example.com/embed/1">
            <meta property="og:video:type" content="text/html">
            <meta property="og:video:url" content="https://cdn.example.com/real.mp4">
            <meta property="og:video:type" content="video/mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1, "only the video/mp4 entry survives");
        assert_eq!(formats[0].url, "https://cdn.example.com/real.mp4");
    }

    /// An entry with no `og:video:type` sibling keeps its unspecified type and is
    /// still emitted — pages without the tag must not regress.
    #[test]
    fn og_video_without_type_still_extracted() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://cdn.example.com/video.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert_eq!(OpenGraphStrategy.detect(&ctx).len(), 1);
    }

    /// Boundary: a media type is accepted even when the URL carries no extension
    /// to sniff, and the declared type supplies the ext instead of the `mp4` guess.
    #[test]
    fn og_video_media_type_accepted_without_url_extension() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://cdn.example.com/stream/1234">
            <meta property="og:video:type" content="video/webm">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(
            formats[0].ext.as_deref(),
            Some("webm"),
            "ext comes from og:video:type, not the mp4 default"
        );
    }

    /// MIME type/subtype are case-insensitive and may carry parameters
    /// (RFC 9110 §8.3.1) — neither may cause a real stream to be dropped.
    #[test]
    fn og_video_type_is_case_insensitive_and_ignores_parameters() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://cdn.example.com/a.mp4">
            <meta property="og:video:type" content="VIDEO/MP4; codecs=avc1.64001f">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert_eq!(OpenGraphStrategy.detect(&ctx).len(), 1);
    }

    /// `og:audio` carries the same structured `type` property and the same defect.
    #[test]
    fn og_audio_html_embed_type_skipped() {
        let raw = r#"<html><head>
            <meta property="og:audio" content="https://example.com/embed/audio">
            <meta property="og:audio:type" content="text/html">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(OpenGraphStrategy.detect(&ctx).is_empty());
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
