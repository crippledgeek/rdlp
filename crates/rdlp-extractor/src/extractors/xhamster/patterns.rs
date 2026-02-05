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
    Regex::new(r#"file\s*:\s*["'](?P<url>.+?)["']"#)
        .expect("Valid legacy file pattern")
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
pub static EMBED_VARS_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"vars\s*:\s*(\{.+?\})\s*,?\s*\n").expect("Valid embed vars pattern")
});

/// Check if a URL matches any xHamster pattern (video, embed, or user).
pub fn is_suitable(url: &str) -> bool {
    XHAMSTER_VIDEO_PATTERN.is_match(url)
        || XHAMSTER_EMBED_PATTERN.is_match(url)
        || XHAMSTER_USER_PATTERN.is_match(url)
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

/// Rewrite mobile URL to desktop (strip `m.` subdomain).
pub fn rewrite_mobile_url(url: &str) -> String {
    Regex::new(r"^(https?://(?:.+?\.)?)m\.")
        .unwrap()
        .replace(url, "$1")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_url_new_schema() {
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster.com/videos/femaleagent-shy-beauty-takes-the-bait-1509445"));
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
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("http://xhamster.com/movies/1509445/femaleagent_shy_beauty.html"));
        assert_eq!(
            extract_video_id(
                "http://xhamster.com/movies/1509445/femaleagent_shy_beauty.html"
            ),
            Some("1509445".to_string())
        );
    }

    #[test]
    fn test_video_url_alphanumeric_id() {
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("http://de.xhamster.com/videos/skinny-girl-fucks-herself-xhnBJZx"));
        assert_eq!(
            extract_video_id(
                "http://de.xhamster.com/videos/skinny-girl-fucks-herself-xhnBJZx"
            ),
            Some("xhnBJZx".to_string())
        );
    }

    #[test]
    fn test_video_url_alt_domains() {
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster.one/videos/femaleagent-shy-beauty-1509445"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster.desi/videos/femaleagent-shy-beauty-1509445"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster2.com/videos/femaleagent-shy-beauty-1509445"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster11.com/videos/femaleagent-shy-beauty-1509445"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster26.com/videos/femaleagent-shy-beauty-1509445"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhday.com/videos/strapless-threesome-xhh7yVf"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhvid.com/videos/lk-mm-xhc6wn6"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://xhamster20.desi/videos/my-verification-video-11937369"));
    }

    #[test]
    fn test_video_url_country_codes() {
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://de.xhamster.com/videos/euro-pedal-pumping-7937821"));
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://pt.xhamster.com/videos/euro-pedal-pumping-7937821"));
    }

    #[test]
    fn test_video_url_mobile() {
        assert!(XHAMSTER_VIDEO_PATTERN
            .is_match("https://m.xhamster.com/videos/cute-teen-solo-masturbation-8559111"));
    }

    #[test]
    fn test_video_url_invalid() {
        assert!(!XHAMSTER_VIDEO_PATTERN.is_match("https://youtube.com/watch?v=test"));
        assert!(!XHAMSTER_VIDEO_PATTERN.is_match("https://pornhub.com/view_video.php?viewkey=ph123"));
    }

    #[test]
    fn test_embed_url() {
        assert!(XHAMSTER_EMBED_PATTERN
            .is_match("http://xhamster.com/xembed.php?video=3328539"));
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
        assert!(is_suitable("https://xhamster.com/users/netvideogirls/videos"));
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
}
