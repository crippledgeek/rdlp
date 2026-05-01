//! URL patterns for the XNXX extractor.
//!
//! Matches `xnxx.com` and `xnxx3.com` video and embed pages.

use lazy_regex::{Lazy, Regex, lazy_regex};

/// Matches canonical video pages and extracts the video ID.
///
/// Accepted forms:
/// - `https://www.xnxx.com/video-14cco143/slug`
/// - `https://www.xnxx3.com/video-14cco143/`
/// - `https://xnxx.com/video14cco143/slug` (no hyphen between `video` and id)
pub static VIDEO_URL: Lazy<Regex> =
    lazy_regex!(r"(?i)^https?://(?:www\.)?xnxx3?\.com/video-?([a-z0-9]+)(?:/|$)");

/// Matches embed pages and extracts the video ID.
///
/// Accepted form:
/// - `https://www.xnxx.com/embedframe/14cco143`
pub static EMBED_URL: Lazy<Regex> =
    lazy_regex!(r"(?i)^https?://(?:www\.)?xnxx3?\.com/embedframe/([a-z0-9]+)");

/// Return `true` if `url` looks like an XNXX video or embed URL.
#[must_use]
pub fn is_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url) || EMBED_URL.is_match(url)
}

/// Extract the video ID from an XNXX URL, or `None` if it does not match.
#[must_use]
pub fn extract_video_id(url: &str) -> Option<String> {
    VIDEO_URL
        .captures(url)
        .or_else(|| EMBED_URL.captures(url))
        .map(|c| c[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_canonical() {
        let url = "https://www.xnxx.com/video-14cco143/slug";
        let caps = VIDEO_URL.captures(url).expect("should match canonical URL");
        assert_eq!(&caps[1], "14cco143");
    }

    #[test]
    fn matches_xnxx3_domain() {
        let url = "https://www.xnxx3.com/video-14cco143/some-slug";
        let caps = VIDEO_URL.captures(url).expect("should match xnxx3.com");
        assert_eq!(&caps[1], "14cco143");
    }

    #[test]
    fn tolerates_no_hyphen() {
        let url = "https://www.xnxx.com/video14cco143/slug";
        let caps = VIDEO_URL
            .captures(url)
            .expect("should tolerate missing hyphen");
        assert_eq!(&caps[1], "14cco143");
    }

    #[test]
    fn rejects_xvideos_url() {
        let url = "https://www.xvideos.com/video.abc/slug";
        assert!(!VIDEO_URL.is_match(url), "must not match xvideos.com");
        assert!(!EMBED_URL.is_match(url), "must not match xvideos.com embed");
    }

    #[test]
    fn matches_embed() {
        let url = "https://www.xnxx.com/embedframe/14cco143";
        let caps = EMBED_URL.captures(url).expect("should match embed URL");
        assert_eq!(&caps[1], "14cco143");
    }
}
