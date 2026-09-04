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
/// Used ONLY by the `#[cfg(test)]` loopback seam in [`parse_video_id`], never
/// in production — see that function.
#[cfg(test)]
static VIDEO_PATH_PATTERN: Lazy<Regex> = lazy_regex!(r"/videos/(\d+)/[^/?#]+");

/// Whether this URL is a PornoXO video page.
pub(crate) fn is_suitable(url: &str) -> bool {
    URL_PATTERN.is_match(url)
}

/// The numeric video id from a canonical PornoXO video URL.
///
/// Production behavior: host-anchored via [`URL_PATTERN`], so a foreign or
/// lookalike host yields `None`.
///
/// Test behavior: additionally accepts the path shape when the URL is a
/// loopback origin, so the mockito-backed `extract` tests can drive one.
///
/// What counts as a loopback origin is NOT decided here. This shares the one
/// definition — `base::common::manifest_url::is_loopback_origin` — with the
/// SSRF gate's own `cfg(test)` seam, so the two cannot come to disagree about
/// which origins qualify. The scope is that predicate's: HTTP(S) on loopback
/// only, never arbitrary hosts, and production builds compile without the seam
/// at all.
pub(crate) fn parse_video_id(url: &str) -> Option<String> {
    if let Some(id) = URL_PATTERN.captures(url).and_then(|c| c.get(1)) {
        return Some(id.as_str().to_owned());
    }

    // Shares the definition of "loopback origin" with the SSRF gate's own
    // `cfg(test)` seam, so the two cannot drift apart on what loopback means.
    // The gates themselves stay separate: this is an id parser, not a security
    // boundary, and the two should be free to change independently.
    #[cfg(test)]
    if crate::base::common::manifest_url::is_loopback_origin(url) {
        return VIDEO_PATH_PATTERN
            .captures(url)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_owned());
    }

    None
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

    /// The path-only fallback is a TEST SEAM, scoped to loopback.
    ///
    /// Production stays host-anchored; only a loopback origin (which is what
    /// mockito binds) takes the path-only route. This pins the SCOPE — a
    /// foreign host is refused here exactly as in production, so the seam
    /// cannot silently become a blanket widening of the parser.
    ///
    /// The predicate itself is shared with the SSRF gate's seam and is covered
    /// directly in `base::common::manifest_url`; what this test adds is that
    /// `parse_video_id` actually routes through it.
    ///
    /// The production-only half (loopback rejected when `cfg(test)` is off) is
    /// not observable from a test binary — the same limitation every
    /// `cfg(test)` seam built on that predicate carries.
    #[test]
    fn path_fallback_is_scoped_to_loopback() {
        // The seam itself: mockito's origin resolves.
        assert_eq!(
            parse_video_id("http://127.0.0.1:1234/videos/2928541/x/").as_deref(),
            Some("2928541")
        );
        assert_eq!(
            parse_video_id("http://localhost:1234/videos/2928541/x/").as_deref(),
            Some("2928541")
        );

        // ...and it stops there. These are the hosts the widening must NOT reach.
        assert_eq!(parse_video_id("https://evil.test/videos/2928541/x/"), None);
        assert_eq!(
            parse_video_id("https://pornoxo.com.evil.test/videos/1/x/"),
            None
        );
        assert_eq!(parse_video_id("https://notpornoxo.com/videos/1/x/"), None);

        // Routing is never affected by the seam.
        assert!(!is_suitable("http://127.0.0.1:1234/videos/2928541/x/"));
    }
}
