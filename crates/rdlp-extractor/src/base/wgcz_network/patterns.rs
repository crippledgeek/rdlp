//! Inline-JS regex patterns used by XVideos and XNXX.
//!
//! Both sites emit the same `html5player.setXxx('value')` calls from an
//! inline `<script>` on the video page. These patterns capture the string
//! argument for the subset of calls we care about.

use regex::Regex;
use std::sync::LazyLock;

/// `html5player.setVideoHLS('<m3u8 url>')`
pub static VIDEO_HLS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"html5player\.setVideoHLS\(['"]([^'"]+)['"]\)"#).unwrap());

/// `html5player.setVideoUrlLow('<mp4 url>')`
pub static VIDEO_URL_LOW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"html5player\.setVideoUrlLow\(['"]([^'"]+)['"]\)"#).unwrap());

/// `html5player.setVideoUrlHigh('<mp4 url>')`
pub static VIDEO_URL_HIGH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"html5player\.setVideoUrlHigh\(['"]([^'"]+)['"]\)"#).unwrap());

/// `html5player.setVideoTitle('<title>')`
pub static VIDEO_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"html5player\.setVideoTitle\(['"]([^'"]+)['"]\)"#).unwrap());

/// `html5player.setThumbUrl('<url>')`
pub static THUMB_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"html5player\.setThumbUrl\(['"]([^'"]+)['"]\)"#).unwrap());

/// `html5player.setUploaderName('<name>')`
pub static UPLOADER_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"html5player\.setUploaderName\(['"]([^'"]+)['"]\)"#).unwrap());

/// Validate that a URL captured from inline JS starts with `http://` or
/// `https://`. Returns `None` for values with any other scheme (e.g.
/// `javascript:`, `data:`) so they are silently discarded by callers.
///
/// This is a post-extraction filter applied to URL-bearing patterns
/// (`VIDEO_HLS`, `VIDEO_URL_LOW`, `VIDEO_URL_HIGH`, `THUMB_URL`) to guard
/// against XSS-style injections embedded in the page's inline JavaScript.
#[must_use]
pub fn require_http_scheme(url: &str) -> Option<&str> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        html5player.setVideoHLS('https://hls-cdn77.xvideos-cdn.com/TOK,123/uuid/3/hls.m3u8');
        html5player.setVideoUrlLow('https://mp4-cdn77.xvideos-cdn.com/uuid/3/mp4_sd.mp4?secure=SIG,123');
        html5player.setVideoUrlHigh('https://mp4-cdn77.xvideos-cdn.com/uuid/3/mp4_hd.mp4?secure=SIG,123');
        html5player.setVideoTitle('Example Title');
        html5player.setThumbUrl('https://thumb.example/xv_13_t.jpg');
        html5player.setUploaderName('Acme');
    "#;

    #[test]
    fn captures_hls_url() {
        let cap = VIDEO_HLS.captures(SAMPLE).expect("match");
        assert_eq!(
            &cap[1],
            "https://hls-cdn77.xvideos-cdn.com/TOK,123/uuid/3/hls.m3u8"
        );
    }

    #[test]
    fn captures_low_mp4_url() {
        let cap = VIDEO_URL_LOW.captures(SAMPLE).expect("match");
        assert!(cap[1].ends_with("mp4_sd.mp4?secure=SIG,123"));
    }

    #[test]
    fn captures_high_mp4_url() {
        let cap = VIDEO_URL_HIGH.captures(SAMPLE).expect("match");
        assert!(cap[1].ends_with("mp4_hd.mp4?secure=SIG,123"));
    }

    #[test]
    fn captures_title_thumb_uploader() {
        assert_eq!(&VIDEO_TITLE.captures(SAMPLE).unwrap()[1], "Example Title");
        assert_eq!(
            &THUMB_URL.captures(SAMPLE).unwrap()[1],
            "https://thumb.example/xv_13_t.jpg"
        );
        assert_eq!(&UPLOADER_NAME.captures(SAMPLE).unwrap()[1], "Acme");
    }

    /// Regression guard for L5: `require_http_scheme` must reject non-http(s)
    /// schemes so that injected values like `javascript:alert(1)` cannot be
    /// forwarded to the downloader as a video URL.
    ///
    /// Before the fix there was no scheme validation; the captured value was
    /// used as-is.
    #[test]
    fn require_http_scheme_rejects_javascript_scheme() {
        assert_eq!(
            require_http_scheme("javascript:alert(1)"),
            None,
            "javascript: scheme must be rejected"
        );
    }

    #[test]
    fn require_http_scheme_rejects_data_uri() {
        assert_eq!(
            require_http_scheme("data:text/html,<h1>hello</h1>"),
            None,
            "data: URI must be rejected"
        );
    }

    #[test]
    fn require_http_scheme_rejects_empty_string() {
        assert_eq!(
            require_http_scheme(""),
            None,
            "empty string must be rejected"
        );
    }

    #[test]
    fn require_http_scheme_accepts_https() {
        let url = "https://hls-cdn77.xvideos-cdn.com/TOK,123/uuid/3/hls.m3u8";
        assert_eq!(require_http_scheme(url), Some(url));
    }

    #[test]
    fn require_http_scheme_accepts_http() {
        let url = "http://example.com/video.mp4";
        assert_eq!(require_http_scheme(url), Some(url));
    }

    /// Regression guard: simulate a page where `setVideoHLS` contains a
    /// `javascript:` injection. The captured group value must be rejected by
    /// `require_http_scheme`.
    #[test]
    fn video_hls_injection_rejected_by_scheme_filter() {
        let html = r#"html5player.setVideoHLS('javascript:alert(1)')"#;
        let captured = VIDEO_HLS
            .captures(html)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        assert_eq!(
            require_http_scheme(&captured),
            None,
            "javascript: injection in setVideoHLS must be rejected"
        );
    }
}
