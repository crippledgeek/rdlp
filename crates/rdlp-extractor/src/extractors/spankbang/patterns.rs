//! Static URL and HTML extraction patterns for SpankBang.

use regex::Regex;
use std::sync::LazyLock;

/// Matches SpankBang video and playlist URLs (yt-dlp parity, ported to Rust regex).
/// Captures `id` for `<id>/(video|play|embed)`, `id_2` for `<scope>-<id>/playlist/`.
pub(super) static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        ^https?://
        (?:[^/]+\.)?spankbang\.com/
        (?:
            (?P<id>[\da-z]+)/(?:video|play|embed)\b
            |
            [\da-z]+-(?P<id_2>[\da-z]+)/playlist/[^/?\#&]+
        )
        ",
    )
    .expect("SpankBang VIDEO_URL regex")
});

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
pub(super) static STREAM_DATA_INLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)stream_data\s*=\s*(\{.*?\});")
        .expect("SpankBang STREAM_DATA_INLINE regex")
});

/// `data-streamkey="..."` — opaque token used by the formats-API fallback.
pub(super) static STREAMKEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-streamkey\s*=\s*"([^"]+)""#).expect("SpankBang STREAMKEY regex")
});

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
pub(super) static TITLE_H1: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<h1[^>]+title="([^"]+)""#).expect("SpankBang TITLE_H1 regex")
});

/// `<meta property="og:KEY" content="VALUE">` — captures key + value.
/// Tolerant of attribute order (`property` before `content` or vice versa)
/// and of additional unrelated attributes inside the meta tag.
pub(super) static OG_META: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<meta\b[^>]*\bproperty="og:([^"]+)"[^>]*\bcontent="([^"]*)"[^>]*>"#,
    )
    .expect("SpankBang OG_META regex")
});

/// `<a href="/profile/SLUG">DISPLAY</a>` — uploader profile link.
/// Capture group 1 = slug, capture group 2 = display name (may differ
/// from the slug in capitalisation, e.g. "GammaEntertainment" vs
/// "gammaentertainment").
pub(super) static UPLOADER_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a[^>]*\bhref="/profile/([a-z0-9_-]+)"[^>]*>([^<]+)</a>"#)
        .expect("SpankBang UPLOADER_LINK regex")
});

/// `<a href="/<id>/video/<slug>" title="<title>">` — search result anchor.
/// SpankBang renders each result twice per card (image wrapper + title link);
/// the title-bearing form is the one with `title="..."`. Capture group 1 =
/// video ID, group 2 = slug, group 3 = title text.
pub(super) static SEARCH_RESULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a[^>]*\bhref="/([a-z0-9]+)/video/([^"]+)"[^>]*\btitle="([^"]+)"[^>]*>"#,
    )
    .expect("SpankBang SEARCH_RESULT regex")
});

/// Search-result image-wrapper anchor: captures (id, thumbnail URL,
/// duration label like "3m" or "1h23m"). The wrapper anchor's structure
/// embeds an `<img>` tag and a `<div data-testid="video-item-length">DUR</div>`
/// that the title-only `SEARCH_RESULT` regex skips.
///
/// Capture groups: (1) video id, (2) thumbnail URL, (3) duration text.
pub(super) static SEARCH_CARD_THUMB_DURATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<a[^>]*\bhref="/([a-z0-9]+)/video/[^"]*"[^>]*class="relative[^"]*"[^>]*>.*?<img[^>]*\bsrc="([^"]+)"[^>]*>.*?data-testid="video-item-length"[^>]*>\s*([^<]+?)\s*<"#,
    )
    .expect("SpankBang SEARCH_CARD_THUMB_DURATION regex")
});

/// Per-card info block: captures the view count + the video id (joined via
/// the title-link `href` inside the same block).
///
/// Capture groups: (1) view text (e.g. "940K"), (2) video id from the
/// trailing title link.
pub(super) static SEARCH_CARD_VIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<span\s+data-testid="views"[^>]*>.*?<span[^>]*>([^<]+)</span>.*?</span>.*?<a[^>]*\bhref="/([a-z0-9]+)/video/"#,
    )
    .expect("SpankBang SEARCH_CARD_VIEWS regex")
});

/// Per-card uploader-or-creator block. The data-testid="title" link inside
/// an info-with-badge container points to either a tag (`/s/<query>/`),
/// a channel (`/<prefix>/channel/<slug>/`), or a pornstar
/// (`/<prefix>/pornstar/<slug>/`). We capture both creator forms — channel
/// and pornstar — and skip plain tags. Slugs use `+` for spaces, so the
/// character class is permissive (`[^/"]+`).
///
/// Capture groups: (1) creator URL slug, (2) creator display name, (3) video
/// id from the trailing title link.
pub(super) static SEARCH_CARD_CHANNEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<a[^>]*data-testid="title"[^>]*\bhref="/[a-z0-9_+-]+/(?:channel|pornstar)/([^/"]+)/?"[^>]*>\s*(?:<[^>]+>\s*)*<span[^>]*>([^<]+)</span>.*?<a[^>]*\bhref="/([a-z0-9]+)/video/"#,
    )
    .expect("SpankBang SEARCH_CARD_CHANNEL regex")
});

/// `<id|class="video_removed">` — yt-dlp's removed-video sentinel.
pub(super) static VIDEO_REMOVED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<[^>]+\b(?:id|class)=["']video_removed"#)
        .expect("SpankBang VIDEO_REMOVED regex")
});

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
