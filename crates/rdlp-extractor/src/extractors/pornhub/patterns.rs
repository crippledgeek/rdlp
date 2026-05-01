//! URL patterns for PornHub extractor
//!
//! Contains all regex patterns for matching and parsing URLs.

use lazy_regex::{Lazy, Regex, lazy_regex};

/// URL pattern for PornHub videos
///
/// Supports:
/// - Standard: `https://www.pornhub.com/view_video.php?viewkey=ph123`
/// - Numeric: `https://www.pornhub.com/view_video.php?viewkey=123456`
/// - Video/show: `https://www.pornhub.com/video/show?viewkey=ph123`
/// - Embed: `https://www.pornhub.com/embed/ph123`
/// - Thumbzilla: `https://www.thumbzilla.com/video/ph123/title`
/// - Alt TLDs: `.net`, `.org`
/// - Country codes: `de.pornhub.com`, `fr.pornhub.com`
/// - Onion: `pornhubvybmsymdol4iibwgwtkpwmeyd6luq2gxajgjzfjvotyt5zhyd.onion`
pub static PORNHUB_VIDEO_URL_PATTERN: Lazy<Regex> = lazy_regex!(
    r"(?x)
    https?://
    (?:
        (?:(?:www|[a-z]{2})\.)?
        (?:pornhub(?:premium)?\.(?:com|net|org)|pornhubvybmsymdol4iibwgwtkpwmeyd6luq2gxajgjzfjvotyt5zhyd\.onion)
        /(?:(?:view_video\.php|video/show)\?viewkey=|embed/)
        |
        (?:www\.)?thumbzilla\.com/video/
    )
    (?P<id>[\da-z]+)
    "
);

/// URL pattern for PornHub playlists
///
/// Supports:
/// - Standard: `https://www.pornhub.com/playlist/123456`
/// - Alt TLDs and country codes
pub static PORNHUB_PLAYLIST_URL_PATTERN: Lazy<Regex> = lazy_regex!(
    r"(?x)
    https?://
    (?:(?:www|[a-z]{2})\.)?
    (?:pornhub(?:premium)?\.(?:com|net|org)|pornhubvybmsymdol4iibwgwtkpwmeyd6luq2gxajgjzfjvotyt5zhyd\.onion)
    /playlist/(?P<id>\d+)
    "
);

/// Pattern to extract video links from playlist HTML
pub static VIDEO_LINK_PATTERN: Lazy<Regex> = lazy_regex!(
    r#"href="(/(?:view_video\.php\?.*?\bviewkey=|embed/)(ph[0-9a-f]+))[^"]*"[^>]*(?:title="([^"]*)")?"#
);

/// Pattern to extract video count from JavaScript
pub static VIDEO_COUNT_PATTERN: Lazy<Regex> = lazy_regex!(r"var\s+itemsCount\s*=\s*(\d+)");

/// Pattern to extract AJAX token from JavaScript
pub static AJAX_TOKEN_PATTERN: Lazy<Regex> = lazy_regex!(r#"var\s+token\s*=\s*"([^"]+)""#);

/// Pattern to extract flashvars JSON
pub static FLASHVARS_PATTERN: Lazy<Regex> = lazy_regex!(r"var\s+flashvars_\d+\s*=\s*(\{.+?});");

/// Pattern to extract quality from URL (e.g., "1080P_4000K")
pub static QUALITY_FROM_URL_PATTERN: Lazy<Regex> = lazy_regex!(r"(?i)(\d+)[pP]");

/// Pattern to extract qualityItems JSON arrays
pub static QUALITY_ITEMS_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"var\s+qualityItems_\w+\s*=\s*(\[.+?])\s*;"#);

/// Pattern to extract media/quality variables
pub static MEDIA_VAR_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"var\s+(media|quality)_(\w+)\s*=\s*["']([^"']+)["']\s*;"#);

/// Pattern to extract download button URLs
pub static DOWNLOAD_BTN_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"<a[^>]+\bclass=["'][^"']*downloadBtn[^"']*["'][^>]+\bhref=["']([^"']+)["']"#);

/// Check if URL matches video or playlist pattern
pub fn is_suitable(url: &str) -> bool {
    PORNHUB_VIDEO_URL_PATTERN.is_match(url) || PORNHUB_PLAYLIST_URL_PATTERN.is_match(url)
}

/// Check if URL is a playlist
pub fn is_playlist_url(url: &str) -> bool {
    PORNHUB_PLAYLIST_URL_PATTERN.is_match(url)
}

/// Extract video ID from URL
pub fn extract_video_id(url: &str) -> Option<String> {
    PORNHUB_VIDEO_URL_PATTERN
        .captures(url)
        .and_then(|cap| cap.name("id"))
        .map(|m| m.as_str().to_string())
}

/// Extract playlist ID from URL
pub fn extract_playlist_id(url: &str) -> Option<String> {
    PORNHUB_PLAYLIST_URL_PATTERN
        .captures(url)
        .and_then(|cap| cap.name("id"))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_url_pattern() {
        // Standard URLs
        assert!(
            PORNHUB_VIDEO_URL_PATTERN
                .is_match("https://www.pornhub.com/view_video.php?viewkey=ph5f490e56f1b51")
        );
        assert!(
            PORNHUB_VIDEO_URL_PATTERN
                .is_match("https://pornhub.com/view_video.php?viewkey=ph123456789abcd")
        );

        // video/show format
        assert!(
            PORNHUB_VIDEO_URL_PATTERN
                .is_match("https://www.pornhub.com/video/show?viewkey=ph5f490e56f1b51")
        );

        // Embed URLs
        assert!(
            PORNHUB_VIDEO_URL_PATTERN.is_match("https://www.pornhub.com/embed/ph5f490e56f1b51")
        );

        // Alternative TLDs
        assert!(
            PORNHUB_VIDEO_URL_PATTERN
                .is_match("https://www.pornhub.net/view_video.php?viewkey=ph123")
        );

        // Country codes
        assert!(
            PORNHUB_VIDEO_URL_PATTERN
                .is_match("https://de.pornhub.com/view_video.php?viewkey=ph789")
        );

        // Thumbzilla
        assert!(
            PORNHUB_VIDEO_URL_PATTERN.is_match("https://www.thumbzilla.com/video/ph5f490e56f1b51")
        );

        // Invalid URLs
        assert!(!PORNHUB_VIDEO_URL_PATTERN.is_match("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_playlist_url_pattern() {
        assert!(
            PORNHUB_PLAYLIST_URL_PATTERN.is_match("https://www.pornhub.com/playlist/186608902")
        );
        assert!(PORNHUB_PLAYLIST_URL_PATTERN.is_match("https://pornhub.com/playlist/12345"));
        assert!(PORNHUB_PLAYLIST_URL_PATTERN.is_match("https://de.pornhub.com/playlist/789"));

        assert!(
            !PORNHUB_PLAYLIST_URL_PATTERN
                .is_match("https://www.pornhub.com/view_video.php?viewkey=ph123")
        );
    }

    #[test]
    fn test_extract_video_id() {
        assert_eq!(
            extract_video_id("https://www.pornhub.com/view_video.php?viewkey=ph5f490e56f1b51"),
            Some("ph5f490e56f1b51".to_string())
        );
        assert_eq!(
            extract_video_id("https://www.pornhub.com/embed/ph123456789abcd"),
            Some("ph123456789abcd".to_string())
        );
        assert_eq!(
            extract_video_id("https://www.pornhub.com/view_video.php?viewkey=695c042a5cb60"),
            Some("695c042a5cb60".to_string())
        );
    }

    #[test]
    fn test_extract_playlist_id() {
        assert_eq!(
            extract_playlist_id("https://www.pornhub.com/playlist/186608902"),
            Some("186608902".to_string())
        );
    }
}
