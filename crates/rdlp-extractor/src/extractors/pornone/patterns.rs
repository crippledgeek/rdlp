//! URL routing for pornone.com.

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

/// The `/{category}/{slug}/{id}/` path shape, without the host.
///
/// Used ONLY by the `#[cfg(test)]` loopback seam in [`parse_video_id`], never
/// in production — see that function.
#[cfg(test)]
static VIDEO_PATH_PATTERN: Lazy<Regex> =
    lazy_regex!(r"\A/[a-z0-9-]+/[a-z0-9-]+/(?P<id>\d+)/?(?:[?#].*)?\z");

pub(crate) fn is_suitable(url: &str) -> bool {
    URL_PATTERN.is_match(url)
}

/// The numeric video id from a canonical PornOne video URL.
///
/// Production behavior: host-anchored via [`URL_PATTERN`], so a foreign or
/// lookalike host yields `None`.
///
/// Test behavior: additionally accepts the path shape when the URL is a
/// loopback origin, so the mockito-backed `extract` tests can drive one, via
/// the shared `base::common::manifest_url::loopback_path_id` helper (also
/// used by PornoXO) — so the two extractors cannot come to disagree about
/// which origins qualify.
pub(crate) fn parse_video_id(url: &str) -> Option<String> {
    if let Some(id) = URL_PATTERN.captures(url).and_then(|c| c.name("id")) {
        return Some(id.as_str().to_owned());
    }

    #[cfg(test)]
    return crate::base::common::manifest_url::loopback_path_id(url, &VIDEO_PATH_PATTERN);

    #[cfg(not(test))]
    None
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
