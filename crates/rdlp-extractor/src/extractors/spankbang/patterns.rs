//! Static URL and HTML extraction patterns for SpankBang.

use lazy_regex::{lazy_regex, Lazy, Regex};

/// Matches SpankBang video and playlist URLs (yt-dlp parity, ported to Rust regex).
/// Captures `id` for `<id>/(video|play|embed)`, `id_2` for `<scope>-<id>/playlist/`.
pub(super) static VIDEO_URL: Lazy<Regex> = lazy_regex!(r"(?x)
    ^https?://
    (?:[^/]+\.)?spankbang\.com/
    (?:
        (?P<id>[\da-z]+)/(?:video|play|embed)\b
        |
        [\da-z]+-(?P<id_2>[\da-z]+)/playlist/[^/?\#&]+
    )
    ");

/// Extract the video ID from a SpankBang URL (video or playlist form).
pub(super) fn extract_video_id(url: &str) -> Option<String> {
    let caps = VIDEO_URL.captures(url)?;
    caps.name("id")
        .or_else(|| caps.name("id_2"))
        .map(|m| m.as_str().to_string())
}

/// Routing predicate.
pub(super) fn is_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// Inline `stream_data = { ... }` Python-dict-shaped object on the video page.
/// Primary format source on current pages. Body is captured (curlies inclusive)
/// for conversion to JSON via [`pydict_to_json`].
pub(super) static STREAM_DATA_INLINE: Lazy<Regex> = lazy_regex!(r"(?s)stream_data\s*=\s*(\{.*?\});");

/// `data-streamkey="..."` — opaque token used by the formats-API fallback.
pub(super) static STREAMKEY: Lazy<Regex> = lazy_regex!(r#"data-streamkey\s*=\s*"([^"]+)""#);

/// Convert a Python-dict-shaped string (single-quoted keys/values) to JSON.
/// Replaces `'` with `"` while preserving backslash escapes, and rewrites
/// bare `True/False/None` to JSON literals. Sufficient for SpankBang's
/// `stream_data` shape; not a general-purpose Python literal parser.
/// Safety bound: the `: True` / `: False` / `: None` rewrites are substring
/// replacements; they are safe only because no observed SpankBang URL or
/// string value contains those substrings. Re-validate if the upstream
/// `stream_data` shape ever embeds free-form text.
pub(super) fn pydict_to_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => out.push('"'),
            '\\' => {
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ => out.push(c),
        }
    }
    out.replace(": True", ": true")
        .replace(": False", ": false")
        .replace(": None", ": null")
}

/// `<h1 ... title="...">` — primary title source on video pages.
pub(super) static TITLE_H1: Lazy<Regex> = lazy_regex!(r#"<h1[^>]+title="([^"]+)""#);

/// `<meta property="og:KEY" content="VALUE">` — captures key + value.
/// Tolerant of attribute order (`property` before `content` or vice versa)
/// and of additional unrelated attributes inside the meta tag.
pub(super) static OG_META: Lazy<Regex> = lazy_regex!(r#"<meta\b[^>]*\bproperty="og:([^"]+)"[^>]*\bcontent="([^"]*)"[^>]*>"#);

/// `<a href="/profile/SLUG">DISPLAY</a>` — uploader profile link in the
/// plain-text form. Capture group 1 = slug, group 2 = display name (may
/// differ from the slug in capitalisation, e.g. "GammaEntertainment" vs
/// "gammaentertainment"). Fails on the icon-wrapped form (use
/// [`UPLOADER_LINK_NAMED`] in addition).
///
/// Slug character class `[a-z0-9._-]+` matches the SpankBang server's
/// case-folded form of the RFC 3986 unreserved set (ALPHA / DIGIT /
/// "-" / "." / "_" / "~"); SpankBang lowercases all profile URLs, and
/// the tilde is not observed in profile slugs. The period is required
/// — usernames like `coutinho.vasconcelos61` are common.
pub(super) static UPLOADER_LINK: Lazy<Regex> = lazy_regex!(r#"<a[^>]*\bhref="/profile/([a-z0-9._-]+)"[^>]*>([^<]+)</a>"#);

/// Modern SpankBang profile-link form where the anchor wraps an icon
/// SVG, a `<span class="name">DISPLAY</span>`, and a chevron SVG —
/// e.g. user-uploaded amateur videos rendered with the Subscribe-button
/// chrome. The `[^<]+` inside `UPLOADER_LINK` rejects the leading SVG.
///
/// Capture group 1 = slug, group 2 = display name from the inner
/// `<span class="name">`. Tolerant of arbitrary preceding nested tags.
/// Same slug character class as [`UPLOADER_LINK`].
pub(super) static UPLOADER_LINK_NAMED: Lazy<Regex> = lazy_regex!(r#"(?s)<a[^>]*\bhref="/profile/([a-z0-9._-]+)"[^>]*>.*?<span[^>]*\bclass="[^"]*\bname\b[^"]*"[^>]*>\s*([^<]+?)\s*</span>"#);

/// `<a href="/<id>/video/<slug>" title="<title>">` — search result anchor.
/// SpankBang renders each result twice per card (image wrapper + title link);
/// the title-bearing form is the one with `title="..."`. Capture group 1 =
/// video ID, group 2 = slug, group 3 = title text.
pub(super) static SEARCH_RESULT: Lazy<Regex> = lazy_regex!(r#"<a[^>]*\bhref="/([a-z0-9]+)/video/([^"]+)"[^>]*\btitle="([^"]+)"[^>]*>"#);

/// Search-result image-wrapper anchor: captures (id, thumbnail URL,
/// duration label like "3m" or "1h23m"). The wrapper anchor's structure
/// embeds an `<img>` tag and a `<div data-testid="video-item-length">DUR</div>`
/// that the title-only `SEARCH_RESULT` regex skips.
///
/// Capture groups: (1) video id, (2) thumbnail URL, (3) duration text.
pub(super) static SEARCH_CARD_THUMB_DURATION: Lazy<Regex> = lazy_regex!(r#"(?s)<a[^>]*\bhref="/([a-z0-9]+)/video/[^"]*"[^>]*class="relative[^"]*"[^>]*>.*?<img[^>]*\bsrc="([^"]+)"[^>]*>.*?data-testid="video-item-length"[^>]*>\s*([^<]+?)\s*<"#);

/// Per-card info block: captures the view count + the video id (joined via
/// the title-link `href` inside the same block).
///
/// Capture groups: (1) view text (e.g. "940K"), (2) video id from the
/// trailing title link.
pub(super) static SEARCH_CARD_VIEWS: Lazy<Regex> = lazy_regex!(r#"(?s)<span\s+data-testid="views"[^>]*>.*?<span[^>]*>([^<]+)</span>.*?</span>.*?<a[^>]*\bhref="/([a-z0-9]+)/video/"#);

/// Per-card uploader-or-creator block. The data-testid="title" link inside
/// an info-with-badge container points to either a tag (`/s/<query>/`),
/// a channel (`/<prefix>/channel/<slug>/`), a creator
/// (`/<prefix>/creator/<slug>/`), or a pornstar
/// (`/<prefix>/pornstar/<slug>/`). We capture all three creator forms —
/// channel, creator, pornstar — and skip plain tags. Slugs use `+` for
/// spaces, so the character class is permissive (`[^/"]+`).
///
/// Capture groups: (1) creator URL slug, (2) creator display name, (3) video
/// id from the trailing title link.
pub(super) static SEARCH_CARD_CHANNEL: Lazy<Regex> = lazy_regex!(r#"(?s)<a[^>]*data-testid="title"[^>]*\bhref="/[a-z0-9_+-]+/(?:channel|creator|pornstar)/([^/"]+)/?"[^>]*>\s*(?:<[^>]+>\s*)*<span[^>]*>([^<]+)</span>.*?<a[^>]*\bhref="/([a-z0-9]+)/video/"#);

/// The `<div class="searches ...">` container on the video page that holds
/// the horizontal tag bar (studio channel, pornstars, plain tags). Captures
/// the inner HTML (group 1) so a follow-up pass can extract per-link types
/// without picking up unrelated `/s/<slug>/` anchors elsewhere on the page
/// (recommendation rails, footer categories, etc.).
pub(super) static SEARCHES_BAR: Lazy<Regex> = lazy_regex!(r#"(?s)<div[^>]*\bclass="searches[^"]*"[^>]*>(.*?)</div>"#);

/// `/<prefix>/(channel|creator)/<slug>/` link inside the searches bar —
/// the video's studio / channel / verified-creator attribution. SpankBang
/// uses three namespaces:
/// - `/channel/<slug>/` — branded studios (BRAZZERS, Dogfart Network, …)
/// - `/creator/<slug>/` — verified creator accounts (Oopsfamily, …)
/// - `/pornstar/<slug>/` — individual performers (matched separately)
///
/// May embed `<img>` (channel avatar) before the text label, so the
/// display-name capture skips through preceding nested tags. Group 1 =
/// slug, group 2 = display name.
pub(super) static CHANNEL_LINK_BAR: Lazy<Regex> = lazy_regex!(r#"(?s)<a[^>]*\bhref="/[a-z0-9_+-]+/(?:channel|creator)/([^/"]+)/?"[^>]*>(?:\s*<[^>]+>)*\s*([^<]+?)\s*</a>"#);

/// `/<prefix>/pornstar/<slug>/` link — captures the display name (group 1).
pub(super) static PORNSTAR_LINK: Lazy<Regex> = lazy_regex!(r#"<a[^>]*\bhref="/[a-z0-9_+-]+/pornstar/[^/"]+/?"[^>]*>([^<]+)</a>"#);

/// `/s/<slug>/` plain-tag link — captures the tag display text (group 1).
/// Run only against the SEARCHES_BAR inner HTML so it doesn't catch
/// recommendation-rail or footer category links.
pub(super) static TAG_LINK_BAR: Lazy<Regex> = lazy_regex!(r#"<a[^>]*\bhref="/s/[^/"]+/?"[^>]*>(?:\s*<[^>]+>)*\s*([^<]+?)\s*</a>"#);

/// `<id|class="video_removed">` — yt-dlp's removed-video sentinel.
pub(super) static VIDEO_REMOVED: Lazy<Regex> = lazy_regex!(r#"<[^>]+\b(?:id|class)=["']video_removed"#);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_pattern_matches_video_forms() {
        for url in [
            "https://spankbang.com/56b3d/video/the+slut+maker",
            "https://m.spankbang.com/3vvn/play/fantasy+solo/480p/",
            "https://spankbang.com/2y3td/embed/",
            "https://spankbang.com/2v7ik-7ecbgu/playlist/latina+booty",
        ] {
            assert!(is_suitable(url), "should match: {url}");
        }
    }

    #[test]
    fn url_pattern_rejects_other_sites() {
        for url in [
            "https://www.xnxx.com/video-14cco143/x",
            "https://spankbang.com/explore/something",
            "https://spankbang.com/random",
        ] {
            assert!(!is_suitable(url), "should not match: {url}");
        }
    }

    #[test]
    fn extract_id_from_video_url() {
        assert_eq!(
            extract_video_id("https://spankbang.com/56b3d/video/foo"),
            Some("56b3d".to_string())
        );
    }

    #[test]
    fn extract_id_from_playlist_url() {
        assert_eq!(
            extract_video_id("https://spankbang.com/abc-7ecbgu/playlist/x"),
            Some("7ecbgu".to_string())
        );
    }

    #[test]
    fn pydict_to_json_quotes_and_literals() {
        let py = "{'a': 'b', 'n': True, 'x': None}";
        let j = pydict_to_json(py);
        let parsed: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed["a"], "b");
        assert_eq!(parsed["n"], true);
        assert!(parsed["x"].is_null());
    }
}
