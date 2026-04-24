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
}
