//! URL patterns for PornoXO.

use lazy_regex::{Lazy, Regex, lazy_regex};

/// Canonical video URL: `/videos/{numeric-id}/{slug}[/]`.
///
/// The host alternation is anchored on both sides so `pornoxo.com.evil.test`
/// and `notpornoxo.com` do not match (SSRF-adjacent: `suitable()` decides
/// which extractor claims an operator-supplied URL).
pub(crate) static URL_PATTERN: Lazy<Regex> =
    lazy_regex!(r"^https?://(?:www\.)?pornoxo\.com/videos/(\d+)/[^/?#]+/?(?:[?#]|$)");

/// The `/videos/{numeric-id}/{slug}` path shape, without the host.
///
/// Host gating belongs to [`URL_PATTERN`] / [`is_suitable`], which decide
/// *which* extractor claims a URL. Once a URL has been claimed, reading its id
/// is a pure path parse — so this pattern deliberately omits the host, which
/// also lets the mockito-backed `extract` tests drive a loopback origin.
static VIDEO_PATH_PATTERN: Lazy<Regex> = lazy_regex!(r"/videos/(\d+)/[^/?#]+");

/// Whether this URL is a PornoXO video page.
pub(crate) fn is_suitable(url: &str) -> bool {
    URL_PATTERN.is_match(url)
}

/// The numeric video id from a video URL's path.
pub(crate) fn parse_video_id(url: &str) -> Option<String> {
    VIDEO_PATH_PATTERN
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_video_url() {
        let u = "https://www.pornoxo.com/videos/2928541/he-fucked-his-stepsister-while-she-was-on-the-phone/";
        assert!(is_suitable(u));
        assert_eq!(parse_video_id(u).as_deref(), Some("2928541"));
    }

    #[test]
    fn accepts_without_www_and_without_trailing_slash() {
        let u = "https://pornoxo.com/videos/2928541/some-slug";
        assert!(is_suitable(u));
        assert_eq!(parse_video_id(u).as_deref(), Some("2928541"));
    }

    #[test]
    fn rejects_non_video_paths() {
        assert!(!is_suitable("https://www.pornoxo.com/tags/creampie/"));
        assert!(!is_suitable("https://www.pornoxo.com/search/?q=creampie"));
        assert!(!is_suitable("https://www.pornoxo.com/"));
    }

    #[test]
    fn rejects_lookalike_hosts() {
        assert!(!is_suitable("https://pornoxo.com.evil.test/videos/1/x/"));
        assert!(!is_suitable("https://notpornoxo.com/videos/1/x/"));
        assert!(!is_suitable("https://www.pornoxo.org/videos/1/x/"));
    }

    #[test]
    fn rejects_non_numeric_id() {
        assert!(!is_suitable("https://www.pornoxo.com/videos/abc/slug/"));
        assert_eq!(
            parse_video_id("https://www.pornoxo.com/videos/abc/slug/"),
            None
        );
    }

    /// `parse_video_id` reads the path only; host gating is `is_suitable`'s job.
    /// This split is what lets the mockito `extract` tests use a loopback origin.
    #[test]
    fn parses_id_from_path_on_any_host() {
        assert_eq!(
            parse_video_id("http://127.0.0.1:1234/videos/2928541/x/").as_deref(),
            Some("2928541")
        );
        assert!(!is_suitable("http://127.0.0.1:1234/videos/2928541/x/"));
    }
}
