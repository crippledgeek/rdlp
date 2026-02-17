//! URL patterns for xHamster extractor
//!
//! Static regex patterns compiled once at first use via `once_cell::sync::Lazy`.
//!
//! ## Supported Domains
//!
//! - `xhamster.com`, `.one`, `.desi`
//! - `xhms.pro`, `xhday.com`, `xhvid.com`
//! - Numbered domains: `xhamster2.com`, `xhamster11.com`, `xhamster26.com`, etc.
//! - Country-code subdomains: `de.xhamster.com`, `pt.xhamster.com`
//! - Mobile: `m.xhamster.com` (rewritten to desktop in extractor)

use once_cell::sync::Lazy;
use regex::Regex;

/// Shared domain pattern for all xHamster URL regexes.
const DOMAINS: &str =
    r"(?:xhamster\.(?:com|one|desi)|xhms\.pro|xhamster\d+\.(?:com|desi)|xhday\.com|xhvid\.com)";

/// URL pattern for xHamster videos.
///
/// Supports two URL schemas:
/// - Old: `/movies/{id}/{slug}.html`
/// - New: `/videos/{slug}-{id}`
///
/// Video IDs can be numeric or alphanumeric (e.g. `xhnBJZx`).
pub static XHAMSTER_VIDEO_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"https?://(?:[^/?#]+\.)?{DOMAINS}/(?:movies/(?P<id>[0-9A-Za-z]+)/(?P<display_id>[^/]*)\.html|videos/(?P<display_id_2>[^/]*)-(?P<id_2>[0-9A-Za-z]+))"
    ))
    .expect("Valid xHamster video URL pattern")
});

/// URL pattern for xHamster embed pages.
///
/// Matches: `https://xhamster.com/xembed.php?video=12345`
pub static XHAMSTER_EMBED_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"https?://(?:[^/?#]+\.)?{DOMAINS}/xembed\.php\?video=(?P<id>\d+)"
    ))
    .expect("Valid xHamster embed URL pattern")
});

/// URL pattern for xHamster user/creator pages.
///
/// Matches:
/// - `https://xhamster.com/users/netvideogirls/videos`
/// - `https://xhamster.com/creators/squirt-orgasm-69`
pub static XHAMSTER_USER_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"https?://(?:[^/?#]+\.)?{DOMAINS}/(?:(?P<user>users)|creators)/(?P<id>[^/?#&]+)"
    ))
    .expect("Valid xHamster user URL pattern")
});

/// Regex to extract `window.initials` JSON from page source.
///
/// Uses `(?s)` (DOTALL) so `.` matches newlines — the JSON often spans multiple lines.
pub static INITIALS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)window\.initials\s*=\s*(\{.+?\})\s*;\s*(?:</script>|$)")
        .expect("Valid initials pattern")
});

/// Fallback pattern for `window.initials` (less strict).
pub static INITIALS_FALLBACK_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)window\.initials\s*=\s*(\{.+?\})\s*;")
        .expect("Valid initials fallback pattern")
});

/// Pattern for legacy `sources: {...}` JS object.
pub static LEGACY_SOURCES_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"sources\s*:\s*(\{.+?\})\s*,?\s*\n").expect("Valid legacy sources pattern")
});

/// Pattern for legacy `file: "url"` variable.
pub static LEGACY_FILE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"file\s*:\s*["'](?P<url>.+?)["']"#).expect("Valid legacy file pattern")
});

/// Pattern for `<a class="mp4Thumb" href="url">`.
pub static LEGACY_MP4_THUMB_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<a\s+href=["'](?P<url>.+?)["']\s+class=["']mp4Thumb"#)
        .expect("Valid legacy mp4Thumb pattern")
});

/// Pattern for `<video file="url">`.
pub static LEGACY_VIDEO_FILE_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<video[^>]+file=["'](?P<url>.+?)["'][^>]*>"#)
        .expect("Valid legacy video file pattern")
});

/// Pattern for error/closed video detection.
pub static VIDEO_CLOSED_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"<div[^>]+id=["']videoClosed["'][^>]*>(.+?)</div>"#)
        .expect("Valid videoClosed pattern")
});

/// Pattern for embed page video URL extraction.
pub static EMBED_VIDEO_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"href="(https?://xhamster\.com/(?:movies/\d+/[^"]*\.html|videos/[^/]*-[0-9A-Za-z]+))[^"]*""#)
        .expect("Valid embed video URL pattern")
});

/// Pattern for embed page `vars` JSON.
pub static EMBED_VARS_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"vars\s*:\s*(\{.+?\})\s*,?\s*\n").expect("Valid embed vars pattern"));

/// Check if a URL matches any xHamster pattern (video, embed, or user).
pub fn is_suitable(url: &str) -> bool {
    XHAMSTER_VIDEO_PATTERN.is_match(url)
        || XHAMSTER_EMBED_PATTERN.is_match(url)
        || XHAMSTER_USER_PATTERN.is_match(url)
        || XHAMSTER_SEARCH_PATTERN.is_match(url)
}

/// Check if URL is an embed URL.
pub fn is_embed_url(url: &str) -> bool {
    XHAMSTER_EMBED_PATTERN.is_match(url)
}

/// Check if URL is a user/creator page.
pub fn is_user_url(url: &str) -> bool {
    XHAMSTER_USER_PATTERN.is_match(url)
}

/// Extract video ID from a video URL (old or new schema).
pub fn extract_video_id(url: &str) -> Option<String> {
    XHAMSTER_VIDEO_PATTERN.captures(url).and_then(|caps| {
        caps.name("id")
            .or_else(|| caps.name("id_2"))
            .map(|m| m.as_str().to_string())
    })
}

/// Extract display ID (slug) from a video URL.
pub fn extract_display_id(url: &str) -> Option<String> {
    XHAMSTER_VIDEO_PATTERN.captures(url).and_then(|caps| {
        caps.name("display_id")
            .or_else(|| caps.name("display_id_2"))
            .map(|m| m.as_str().to_string())
    })
}

/// Extract user/creator ID and whether it's a `/users/` path.
pub fn extract_user_info(url: &str) -> Option<(String, bool)> {
    XHAMSTER_USER_PATTERN.captures(url).and_then(|caps| {
        let id = caps.name("id")?.as_str().to_string();
        let is_user = caps.name("user").is_some();
        Some((id, is_user))
    })
}

/// Pattern to strip `m.` subdomain from mobile xHamster URLs.
static MOBILE_URL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(https?://(?:.+?\.)?)m\.").expect("Valid mobile URL pattern"));

/// Rewrite mobile URL to desktop (strip `m.` subdomain).
pub fn rewrite_mobile_url(url: &str) -> String {
    MOBILE_URL_PATTERN.replace(url, "$1").into_owned()
}

/// URL pattern for xHamster search pages.
///
/// Matches: `https://xhamster.com/search/query-terms`
pub static XHAMSTER_SEARCH_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"https?://(?:[^/?#]+\.)?(?:{DOMAINS})/search/[^/?#]+"
    ))
    .expect("Valid xHamster search URL pattern")
});

/// Check if a URL is an xHamster search URL.
pub fn is_search_url(url: &str) -> bool {
    XHAMSTER_SEARCH_PATTERN.is_match(url)
}

/// Build a search URL from a `SearchQuery`.
///
/// Format: `https://xhamster.com/search/{encoded_query}?{filter_params}`
pub fn build_search_url(query: &rdlp_core::SearchQuery) -> String {
    let encoded_query = query.query.split_whitespace().collect::<Vec<_>>().join("+");

    let mut url = format!("https://xhamster.com/search/{encoded_query}");

    let mut first_param = true;
    for filter in &query.filters {
        let sep = if first_param { '?' } else { '&' };
        url.push(sep);
        url.push_str(&filter.key);
        url.push('=');
        url.push_str(&filter.value);
        first_param = false;
    }

    url
}

/// Build a search URL for a specific page number.
pub fn build_search_url_page(query: &rdlp_core::SearchQuery, page: usize) -> String {
    let mut url = build_search_url(query);
    if url.contains('?') {
        url.push_str(&format!("&page={page}"));
    } else {
        url.push_str(&format!("?page={page}"));
    }
    url
}

/// Return the static filter descriptors for xHamster search.
pub fn search_filter_descriptors() -> Vec<rdlp_core::SearchFilterDescriptor> {
    vec![
        rdlp_core::SearchFilterDescriptor {
            key: "quality".to_string(),
            display_name: "Minimum quality".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "720p".to_string(),
                    label: "720p+".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "1080p".to_string(),
                    label: "1080p+".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "2160p".to_string(),
                    label: "4K+".to_string(),
                },
            ],
            default: None,
        },
        rdlp_core::SearchFilterDescriptor {
            key: "sort".to_string(),
            display_name: "Sort by".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "relevance".to_string(),
                    label: "Relevance".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "newest".to_string(),
                    label: "Newest".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "views".to_string(),
                    label: "Most viewed".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "best".to_string(),
                    label: "Top rated".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "longest".to_string(),
                    label: "Longest".to_string(),
                },
            ],
            default: Some("relevance".to_string()),
        },
        rdlp_core::SearchFilterDescriptor {
            key: "date".to_string(),
            display_name: "Upload date".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "week".to_string(),
                    label: "Last week".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "month".to_string(),
                    label: "Last month".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "year".to_string(),
                    label: "Last year".to_string(),
                },
            ],
            default: None,
        },
        rdlp_core::SearchFilterDescriptor {
            key: "orientations".to_string(),
            display_name: "Orientation".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "straight".to_string(),
                    label: "Straight".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "gay".to_string(),
                    label: "Gay".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "shemale".to_string(),
                    label: "Transgender".to_string(),
                },
            ],
            default: None,
        },
        rdlp_core::SearchFilterDescriptor {
            key: "format".to_string(),
            display_name: "Format".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "any".to_string(),
                    label: "Any".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "vr".to_string(),
                    label: "VR".to_string(),
                },
            ],
            default: Some("any".to_string()),
        },
        rdlp_core::SearchFilterDescriptor {
            key: "categories".to_string(),
            display_name: "Category".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "amateur".to_string(),
                    label: "Amateur".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "mature".to_string(),
                    label: "Mature".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "teen".to_string(),
                    label: "Teen (18+)".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "anal".to_string(),
                    label: "Anal".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "lesbian".to_string(),
                    label: "Lesbian".to_string(),
                },
            ],
            default: None,
        },
        rdlp_core::SearchFilterDescriptor {
            key: "duration-min".to_string(),
            display_name: "Min duration (minutes)".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "0".to_string(),
                    label: "0".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "5".to_string(),
                    label: "5".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "10".to_string(),
                    label: "10".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "20".to_string(),
                    label: "20".to_string(),
                },
            ],
            default: Some("0".to_string()),
        },
        rdlp_core::SearchFilterDescriptor {
            key: "duration-max".to_string(),
            display_name: "Max duration (minutes)".to_string(),
            allowed_values: vec![
                rdlp_core::SearchFilterValue {
                    value: "5".to_string(),
                    label: "5".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "10".to_string(),
                    label: "10".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "20".to_string(),
                    label: "20".to_string(),
                },
                rdlp_core::SearchFilterValue {
                    value: "40".to_string(),
                    label: "40".to_string(),
                },
            ],
            default: Some("40".to_string()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_url_new_schema() {
        assert!(
            XHAMSTER_VIDEO_PATTERN.is_match(
                "https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"
            )
        );
        assert_eq!(
            extract_video_id(
                "https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"
            ),
            Some("1509445".to_string())
        );
        assert_eq!(
            extract_display_id(
                "https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"
            ),
            Some("femaleagent-shy-beauty-takes-the-bait".to_string())
        );
    }

    #[test]
    fn test_video_url_old_schema() {
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("http://xhamster.com/movies/1509445/femaleagent_shy_beauty.html")
        );
        assert_eq!(
            extract_video_id("http://xhamster.com/movies/1509445/femaleagent_shy_beauty.html"),
            Some("1509445".to_string())
        );
    }

    #[test]
    fn test_video_url_alphanumeric_id() {
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("http://de.xhamster.com/videos/skinny-girl-fucks-herself-xhnBJZx")
        );
        assert_eq!(
            extract_video_id("http://de.xhamster.com/videos/skinny-girl-fucks-herself-xhnBJZx"),
            Some("xhnBJZx".to_string())
        );
    }

    #[test]
    fn test_video_url_alt_domains() {
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://xhamster.one/videos/femaleagent-shy-beauty-1509445")
        );
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://xhamster.desi/videos/femaleagent-shy-beauty-1509445")
        );
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://xhamster2.com/videos/femaleagent-shy-beauty-1509445")
        );
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://xhamster11.com/videos/femaleagent-shy-beauty-1509445")
        );
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://xhamster26.com/videos/femaleagent-shy-beauty-1509445")
        );
        assert!(
            XHAMSTER_VIDEO_PATTERN.is_match("https://xhday.com/videos/strapless-threesome-xhh7yVf")
        );
        assert!(XHAMSTER_VIDEO_PATTERN.is_match("https://xhvid.com/videos/lk-mm-xhc6wn6"));
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://xhamster20.desi/videos/my-verification-video-11937369")
        );
    }

    #[test]
    fn test_video_url_country_codes() {
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://de.xhamster.com/videos/euro-pedal-pumping-7937821")
        );
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://pt.xhamster.com/videos/euro-pedal-pumping-7937821")
        );
    }

    #[test]
    fn test_video_url_mobile() {
        assert!(
            XHAMSTER_VIDEO_PATTERN
                .is_match("https://m.xhamster.com/videos/cute-teen-solo-masturbation-8559111")
        );
    }

    #[test]
    fn test_video_url_invalid() {
        assert!(!XHAMSTER_VIDEO_PATTERN.is_match("https://youtube.com/watch?v=test"));
        assert!(
            !XHAMSTER_VIDEO_PATTERN.is_match("https://pornhub.com/view_video.php?viewkey=ph123")
        );
    }

    #[test]
    fn test_embed_url() {
        assert!(XHAMSTER_EMBED_PATTERN.is_match("http://xhamster.com/xembed.php?video=3328539"));
        let caps = XHAMSTER_EMBED_PATTERN
            .captures("http://xhamster.com/xembed.php?video=3328539")
            .unwrap();
        assert_eq!(caps.name("id").unwrap().as_str(), "3328539");
    }

    #[test]
    fn test_user_url() {
        let (id, is_user) =
            extract_user_info("https://xhamster.com/users/netvideogirls/videos").unwrap();
        assert_eq!(id, "netvideogirls");
        assert!(is_user);

        let (id, is_user) =
            extract_user_info("https://xhamster.com/creators/squirt-orgasm-69").unwrap();
        assert_eq!(id, "squirt-orgasm-69");
        assert!(!is_user);
    }

    #[test]
    fn test_is_suitable() {
        assert!(is_suitable("https://xhamster.com/videos/test-1509445"));
        assert!(is_suitable("http://xhamster.com/xembed.php?video=3328539"));
        assert!(is_suitable(
            "https://xhamster.com/users/netvideogirls/videos"
        ));
        assert!(!is_suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_rewrite_mobile_url() {
        assert_eq!(
            rewrite_mobile_url("https://m.xhamster.com/videos/test-123"),
            "https://xhamster.com/videos/test-123"
        );
        assert_eq!(
            rewrite_mobile_url("https://xhamster.com/videos/test-123"),
            "https://xhamster.com/videos/test-123"
        );
    }

    #[test]
    fn test_search_url_detected() {
        assert!(is_search_url("https://xhamster.com/search/amateur"));
        assert!(is_search_url(
            "https://xhamster.com/search/hot+videos?quality=1080p"
        ));
        assert!(is_search_url(
            "https://xhamster.com/search/test?page=2&sort=newest"
        ));
        assert!(is_search_url("https://xhamster.desi/search/query"));
    }

    #[test]
    fn test_non_search_url_rejected() {
        assert!(!is_search_url("https://xhamster.com/videos/test-123"));
        assert!(!is_search_url(
            "https://xhamster.com/users/someone/videos"
        ));
        assert!(!is_search_url("https://xhamster.com/"));
    }

    #[test]
    fn test_build_search_url_basic() {
        let query = rdlp_core::SearchQuery {
            query: "test query".to_string(),
            filters: vec![],
            max_results: None,
        };
        let url = build_search_url(&query);
        assert!(url.starts_with("https://xhamster.com/search/"));
        assert!(url.contains("test"));
    }

    #[test]
    fn test_build_search_url_with_filters() {
        let query = rdlp_core::SearchQuery {
            query: "test".to_string(),
            filters: vec![
                rdlp_core::SearchFilter {
                    key: "quality".to_string(),
                    value: "1080p".to_string(),
                },
                rdlp_core::SearchFilter {
                    key: "sort".to_string(),
                    value: "newest".to_string(),
                },
            ],
            max_results: None,
        };
        let url = build_search_url(&query);
        assert!(url.contains("quality=1080p"));
        assert!(url.contains("sort=newest"));
    }

    #[test]
    fn test_build_search_url_encodes_special_chars() {
        let query = rdlp_core::SearchQuery {
            query: "hello world".to_string(),
            filters: vec![],
            max_results: None,
        };
        let url = build_search_url(&query);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
    }

    #[test]
    fn test_search_filter_descriptors_not_empty() {
        let filters = search_filter_descriptors();
        assert!(!filters.is_empty());
        let keys: Vec<&str> = filters.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"quality"));
        assert!(keys.contains(&"sort"));
        assert!(keys.contains(&"date"));
    }

    #[test]
    fn test_search_filter_descriptors_have_values() {
        let filters = search_filter_descriptors();
        for filter in &filters {
            assert!(
                !filter.allowed_values.is_empty(),
                "Filter '{}' has no values",
                filter.key
            );
            assert!(
                !filter.display_name.is_empty(),
                "Filter '{}' has no display name",
                filter.key
            );
        }
    }
}
