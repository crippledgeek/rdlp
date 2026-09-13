//! URL routing for pornone.com.

// Consumed by `PornoneExtractor` (Task 3, #659) — not yet wired, so these
// items are unused outside `#[cfg(test)]` until then. `expect` rather than
// `allow`: it is self-cleaning — Task 3 wiring these in makes the lint stop
// firing, and an unfulfilled `expect` then errors, forcing this line's
// removal instead of letting it silently outlive its reason.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PornoneExtractor in Task 3 (#659); test module already exercises these"
    )
)]

use lazy_regex::{Lazy, Regex, lazy_regex};

/// A video page: `/{category}/{slug}/{id}/` — the category segment varies per
/// video (`/full-movie/`, `/1080p/`, `/argentine/`, …), so it is any slug.
/// Anchored to the URL start so an embedded pornone URL in a foreign query
/// string cannot route here (the hqporner lesson, #722). Two-segment site
/// routes (`/search/`, `/categories/`, `/pornstars/`) never match: a video
/// URL has three path segments and a numeric third.
pub(crate) static URL_PATTERN: Lazy<Regex> = lazy_regex!(
    r"\Ahttps?://(?:www\.)?pornone\.com/(?P<category>[a-z0-9-]+)/(?P<slug>[a-z0-9-]+)/(?P<id>\d+)/?(?:[?#].*)?\z"
);

pub(crate) fn is_suitable(url: &str) -> bool {
    URL_PATTERN.is_match(url)
}

pub(crate) fn parse_video_id(url: &str) -> Option<String> {
    URL_PATTERN
        .captures(url)
        .and_then(|c| c.name("id"))
        .map(|m| m.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_urls_route_here_whatever_the_category() {
        for url in [
            "https://pornone.com/1080p/pornone-sex-video-278218023/278218023/",
            "https://pornone.com/full-movie/some-slug/123/",
            "https://www.pornone.com/argentine/x/1/",
            "https://pornone.com/ass/slug-1/42",
            "https://pornone.com/ass/slug-1/42/?lang=en",
        ] {
            assert!(is_suitable(url), "{url}");
        }
        assert_eq!(
            parse_video_id("https://pornone.com/1080p/pornone-sex-video-278218023/278218023/")
                .as_deref(),
            Some("278218023")
        );
    }

    #[test]
    fn non_video_routes_and_foreign_authorities_do_not_route_here() {
        for url in [
            "https://pornone.com/search/?q=milf",
            "https://pornone.com/categories/",
            "https://pornone.com/pornstars/",
            "https://pornone.com/playlist/",
            "https://pornone.com/",
            "https://evil.test/?r=https://pornone.com/ass/slug/42/",
            "https://pornone.com.evil.test/ass/slug/42/",
            "https://notpornone.com/ass/slug/42/",
            "https://pornone.com/ass/slug/notanid/",
        ] {
            assert!(!is_suitable(url), "{url}");
        }
        assert_eq!(parse_video_id("https://pornone.com/search/?q=milf"), None);
    }
}
