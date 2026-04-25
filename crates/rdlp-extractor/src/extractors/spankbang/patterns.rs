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
