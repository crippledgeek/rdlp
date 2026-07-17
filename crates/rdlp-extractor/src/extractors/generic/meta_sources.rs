//! OpenGraph and Twitter meta tag detection strategies.

use scraper::Selector;
use std::sync::LazyLock;
use url::Url;

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
    Root(OgKind),
    /// The HTTPS alternate of the open root's resource — not a new entry.
    SecureUrl(OgKind),
    /// The open root's MIME type.
    Type(OgKind),
}

impl OgTag {
    /// Classify a `property` attribute. Returns `None` for tags that carry no
    /// URL or type (`og:video:width`, `:height`, `:duration`, …).
    fn classify(property: &str) -> Option<Self> {
        match property {
            "og:video" | "og:video:url" => Some(Self::Root(OgKind::Video)),
            "og:audio" | "og:audio:url" => Some(Self::Root(OgKind::Audio)),
            "og:video:secure_url" => Some(Self::SecureUrl(OgKind::Video)),
            "og:audio:secure_url" => Some(Self::SecureUrl(OgKind::Audio)),
            "og:video:type" => Some(Self::Type(OgKind::Video)),
            "og:audio:type" => Some(Self::Type(OgKind::Audio)),
            _ => None,
        }
    }
}

/// Which OpenGraph media namespace a tag belongs to.
///
/// Load-bearing: a structured property binds to a root of its *own* namespace.
/// `og:video:type` describes an `og:video` — never a neighbouring `og:audio`
/// that merely happens to be the most recent tag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OgKind {
    Video,
    Audio,
}

impl OgKind {
    /// Dense index into the fixed-size orphan-type table — there are exactly
    /// two kinds, so a two-slot array indexed by this beats a `HashMap` (no
    /// heap alloc, no hashing, no `Hash` derive) while staying just as clear.
    const fn index(self) -> usize {
        match self {
            Self::Video => 0,
            Self::Audio => 1,
        }
    }

    /// The `DetectedFormat::source` tag, which feeds the emitted `format_id`.
    const fn source(self) -> &'static str {
        match self {
            Self::Video => "og:video",
            Self::Audio => "og:audio",
        }
    }
}

/// One OpenGraph media entry: a single asset (never two — see [`OgEntry::resolve`])
/// plus the MIME type declared for it.
///
/// `root` is never optional: an [`OgEntry`] cannot exist without one, since it
/// is only ever constructed from a `Root` tag or a fallback orphan `SecureUrl`
/// (see [`OgTag::SecureUrl`]) — both of which supply a URL up front. Making the
/// no-URL state unrepresentable removes the need for any later "does this
/// entry have a URL" check.
struct OgEntry<'a> {
    kind: OgKind,
    /// The tag's own URL.
    root: &'a str,
    /// The HTTPS alternate for the *same* resource, if declared. Per ogp.me,
    /// `og:video:secure_url` is "an alternate url to use if the webpage
    /// requires HTTPS" — not a second asset — so it replaces `root` as the
    /// emitted candidate rather than joining it (issue #495).
    secure_url: Option<&'a str>,
    mime: Option<&'a str>,
}

impl<'a> OgEntry<'a> {
    fn new(kind: OgKind, url: &'a str) -> Self {
        Self {
            kind,
            root: url,
            secure_url: None,
            mime: None,
        }
    }

    /// The single URL this entry contributes as a candidate, resolved against
    /// the page's base: `secure_url` when declared *and usable* (ogp.me: "use
    /// this if you need HTTPS" — and HTTPS is the better default besides),
    /// else `root`. Never both.
    ///
    /// The preference is resolution-aware on purpose. `resolve_url` rejects
    /// empty, `data:`, and `javascript:` values, and real meta soup carries all
    /// three — so preferring `secure_url` *blindly* would let a malformed
    /// alternate take a legitimate stream down with it, turning a recoverable
    /// bad tag into total extraction failure. That is the same rule the
    /// `OgTag::Type` branch already states for an empty `content`: a malformed
    /// sibling tag must not drop a real stream.
    fn resolve(&self, base: &Url) -> Option<String> {
        self.secure_url
            .and_then(|raw| resolve_url(base, raw))
            .or_else(|| resolve_url(base, self.root))
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

impl OpenGraphStrategy {
    /// The entry a structured property binds to: the most recently declared root
    /// **of the same kind**, per the OGP spec's document-order rule.
    fn open_root<'e, 'a>(
        entries: &'e mut [OgEntry<'a>],
        kind: OgKind,
    ) -> Option<&'e mut OgEntry<'a>> {
        entries.iter_mut().rev().find(|entry| entry.kind == kind)
    }
}

impl DetectionStrategy for OpenGraphStrategy {
    fn name(&self) -> &'static str {
        "OpenGraph"
    }

    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat> {
        // Walk in document order, binding each structured property to the root
        // that precedes it, per the OGP spec's array/structured-property rules.
        let mut entries: Vec<OgEntry<'_>> = Vec::new();

        // A `:type` seen before any root of its kind has no entry to bind to
        // yet. ogp.me (<https://ogp.me>) carries no RFC 2119 language and
        // leaves out-of-order authoring undefined, so rather than discard it
        // (which let a `text/html` embed ship as a format — issue #498, the
        // #493 defect via malformed ordering) we hold it here and apply it,
        // after the walk, to that kind's *first* root — fill-if-absent, so an
        // explicit in-block type always wins. A later same-kind orphan
        // overwrites an earlier one: the orphan nearest its eventual root wins.
        // Indexed by `OgKind::index` — two kinds, so a two-slot array suffices.
        let mut orphan_types: [Option<&str>; 2] = [None, None];

        for elem in ctx.html.select(&OG_MEDIA_SELECTOR) {
            let (Some(property), Some(content)) =
                (elem.value().attr("property"), elem.value().attr("content"))
            else {
                continue;
            };

            let Some(tag) = OgTag::classify(property) else {
                continue;
            };

            match tag {
                OgTag::Root(kind) => entries.push(OgEntry::new(kind, content)),
                // A `secure_url` belongs to its kind's open root. A page may also
                // declare one before any root (or with none at all) — keep the URL
                // as its own entry rather than discarding it.
                OgTag::SecureUrl(kind) => match Self::open_root(&mut entries, kind) {
                    Some(entry) => entry.secure_url = Some(content),
                    None => entries.push(OgEntry::new(kind, content)),
                },
                // An empty `content` leaves the type unspecified rather than
                // declaring a non-media one — real meta soup carries these, and
                // they must not drop a legitimate stream.
                OgTag::Type(kind) => {
                    if content.trim().is_empty() {
                        continue;
                    }
                    match Self::open_root(&mut entries, kind) {
                        Some(entry) => entry.mime = Some(content),
                        None => orphan_types[kind.index()] = Some(content),
                    }
                }
            }
        }

        for kind in [OgKind::Video, OgKind::Audio] {
            let Some(mime) = orphan_types[kind.index()] else {
                continue;
            };
            if let Some(entry) = entries.iter_mut().find(|entry| entry.kind == kind)
                && entry.mime.is_none()
            {
                entry.mime = Some(mime);
            }
        }

        entries
            .into_iter()
            .filter(OgEntry::is_media)
            .filter_map(|entry| {
                // A declared media type names the container even when the URL has
                // no extension to sniff — better than guessing downstream.
                let ext_from_mime = entry.mime.and_then(super::content_type_to_ext);
                let url = entry.resolve(ctx.base_url)?;
                let ext = super::detection::ext_from_url(&url)
                    .or_else(|| ext_from_mime.map(str::to_owned));
                Some(DetectedFormat {
                    url,
                    ext,
                    quality: None,
                    confidence: Confidence::High,
                    source: entry.kind.source(),
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

    /// Canonical order (root → secure_url): ogp.me defines `secure_url` as
    /// "an alternate url to use if the webpage requires HTTPS" for the SAME
    /// asset, not a second asset — so the entry yields exactly one candidate,
    /// and it is the HTTPS one.
    ///
    /// This test previously asserted `formats.len() == 2`, which encoded the
    /// #495 defect (both URLs surfaced as separate formats for one video),
    /// and it declared the tags in *reverse* order — secure_url before root —
    /// so it actually exercised the orphan-secure_url fallback path (see
    /// `og_video_secure_url_without_root_becomes_own_entry` below) rather
    /// than the canonical root-then-secure_url path it claimed to test.
    #[test]
    fn og_video_secure_url_shares_entry_with_root() {
        let raw = r#"<html><head>
            <meta property="og:video:url" content="http://cdn.example.com/video.mp4">
            <meta property="og:video:secure_url" content="https://cdn.example.com/video.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1, "root + secure_url is one asset, not two");
        assert_eq!(formats[0].url, "https://cdn.example.com/video.mp4");
    }

    /// A `secure_url` that cannot resolve must fall back to the entry's root
    /// rather than taking the whole entry down with it.
    ///
    /// `resolve_url` rejects empty, `data:`, and `javascript:` values, and real
    /// meta soup carries all three. Preferring `secure_url` *blindly* turned a
    /// recoverable bad tag into total extraction failure on an otherwise-fine
    /// page — the preference has to be resolution-aware. This is the same rule
    /// the `OgTag::Type` branch already states for an empty `content`: a
    /// malformed sibling tag must not drop a legitimate stream.
    #[test]
    fn og_unresolvable_secure_url_falls_back_to_root() {
        for bad in ["", "javascript:void(0)", "data:text/html,x"] {
            let raw = format!(
                r#"<html><head>
                <meta property="og:video:url" content="http://cdn.example.com/video.mp4">
                <meta property="og:video:secure_url" content="{bad}">
                </head></html>"#
            );
            let html = Html::parse_document(&raw);
            let url = Url::parse("https://example.com/page").unwrap();
            let ctx = make_ctx(&html, &raw, &url);

            let formats = OpenGraphStrategy.detect(&ctx);
            assert_eq!(
                formats.len(),
                1,
                "secure_url={bad:?} is unusable; the root must still be emitted"
            );
            assert_eq!(
                formats[0].url, "http://cdn.example.com/video.mp4",
                "secure_url={bad:?} is unusable; the candidate must fall back to the root"
            );
        }
    }

    /// A `secure_url` declared with no preceding root of its kind keeps
    /// today's fallback: it becomes its own entry rather than being dropped.
    #[test]
    fn og_video_secure_url_without_root_becomes_own_entry() {
        let raw = r#"<html><head>
            <meta property="og:video:secure_url" content="https://cdn.example.com/secure.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://cdn.example.com/secure.mp4");
    }

    /// Two distinct roots each with their own `secure_url` yield two
    /// candidates — one per asset — not four.
    #[test]
    fn og_video_multiple_entries_each_collapse_secure_url() {
        let raw = r#"<html><head>
            <meta property="og:video:url" content="http://cdn.example.com/a.mp4">
            <meta property="og:video:secure_url" content="https://cdn.example.com/a.mp4">
            <meta property="og:video:url" content="http://cdn.example.com/b.mp4">
            <meta property="og:video:secure_url" content="https://cdn.example.com/b.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 2, "one candidate per asset, not per URL tag");
        assert!(
            formats
                .iter()
                .any(|f| f.url == "https://cdn.example.com/a.mp4")
        );
        assert!(
            formats
                .iter()
                .any(|f| f.url == "https://cdn.example.com/b.mp4")
        );
    }

    /// `og:video:type` declared *before* any `og:video` root: ogp.me
    /// (<https://ogp.me>) carries no RFC 2119 language and leaves out-of-order
    /// authoring undefined, so an orphaned type binds to the kind's first
    /// root (fill-if-absent) rather than being discarded — matching yt-dlp's
    /// whole-document first-match regex and open-graph-scraper's
    /// collect-then-zip, both of which would catch this ordering. Pre-fix
    /// this dropped the type and let a `text/html` embed page ship as a
    /// format (issue #498, the #493 defect via malformed ordering).
    #[test]
    fn og_video_type_before_root_still_gates() {
        let raw = r#"<html><head>
            <meta property="og:video:type" content="text/html">
            <meta property="og:video" content="https://example.com/embed/353006/">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(
            OpenGraphStrategy.detect(&ctx).is_empty(),
            "an orphaned :type binds to the kind's first root"
        );
    }

    /// An orphaned type binds only when the root it lands on has no type of
    /// its own — an explicit in-block type always wins over an orphan.
    #[test]
    fn og_video_orphan_type_does_not_clobber_explicit_type() {
        let raw = r#"<html><head>
            <meta property="og:video:type" content="text/html">
            <meta property="og:video" content="https://cdn.example.com/real.mp4">
            <meta property="og:video:type" content="video/mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1, "explicit video/mp4 type wins over orphan");
        assert_eq!(formats[0].url, "https://cdn.example.com/real.mp4");
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

    /// `og:video:type` binds to the most recent `og:video` root — not to whatever
    /// entry happens to be last.
    ///
    /// With a bare "last entry" rule the `text/html` here would bind to the
    /// *audio* entry, dropping the real mp3 and letting the video embed page ship
    /// as a format — reintroducing issue #493.
    #[test]
    fn og_type_binds_to_root_of_matching_kind() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://example.com/embed/1">
            <meta property="og:audio" content="https://cdn.example.com/song.mp3">
            <meta property="og:video:type" content="text/html">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        assert_eq!(formats.len(), 1, "the video embed is dropped, audio kept");
        assert_eq!(formats[0].url, "https://cdn.example.com/song.mp3");
        assert_eq!(formats[0].source, "og:audio");
    }

    /// A `secure_url` attaches to its own kind's root, never to an unrelated one.
    #[test]
    fn og_secure_url_attaches_to_root_of_matching_kind() {
        let raw = r#"<html><head>
            <meta property="og:audio" content="https://cdn.example.com/song.mp3">
            <meta property="og:video:secure_url" content="https://cdn.example.com/movie.mp4">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        let formats = OpenGraphStrategy.detect(&ctx);
        let movie = formats
            .iter()
            .find(|f| f.url == "https://cdn.example.com/movie.mp4")
            .expect("video secure_url is still surfaced");
        assert_eq!(
            movie.source, "og:video",
            "a video URL must not be labelled og:audio"
        );
    }

    /// An empty `content=""` means the type is unspecified, not non-media — real
    /// meta soup carries these, and they must not drop a legitimate stream.
    #[test]
    fn og_empty_type_treated_as_unspecified() {
        let raw = r#"<html><head>
            <meta property="og:video" content="https://cdn.example.com/video.mp4">
            <meta property="og:video:type" content="">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert_eq!(OpenGraphStrategy.detect(&ctx).len(), 1);
    }

    /// Canonical OGP order (root → secure_url → type): the type gates both URLs
    /// of the entry, and the `secure_url` collapses into the open root rather
    /// than starting a new entry.
    #[test]
    fn og_secure_url_in_canonical_order_shares_entry_type() {
        let raw = r#"<html><head>
            <meta property="og:video" content="http://cdn.example.com/embed/1">
            <meta property="og:video:secure_url" content="https://cdn.example.com/embed/1">
            <meta property="og:video:type" content="text/html">
        </head></html>"#;
        let html = Html::parse_document(raw);
        let url = Url::parse("https://example.com/page").unwrap();
        let ctx = make_ctx(&html, raw, &url);

        assert!(
            OpenGraphStrategy.detect(&ctx).is_empty(),
            "the entry's text/html type gates its root AND its secure_url"
        );
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
