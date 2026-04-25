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
}
