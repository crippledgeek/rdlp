//! Extraction of the signed HLS master URL from PornoXO's inline `playerConfig`.
//!
//! The page carries `sources` as a JSON object literal with escaped slashes
//! (`"https:\/\/cdn..."`), so it is parsed with `serde_json` rather than a
//! hand-rolled unescape.

use anyhow::{Context, Result, bail};
use lazy_regex::{Lazy, Regex, lazy_regex};
use serde::Deserialize;

/// The `sources:` object literal inside `var playerConfig = { ... }`.
/// Non-greedy to the first `}` — `sources` has no nested objects.
static SOURCES_PATTERN: Lazy<Regex> = lazy_regex!(r"sources:\s*(\{[^}]*\})");

#[derive(Deserialize)]
struct Sources {
    #[serde(rename = "hlsAuto")]
    hls_auto: Option<String>,
}

/// The signed HLS master-playlist URL for this page load.
///
/// The signature is minted per load and expires (~132 min observed), so the
/// returned URL must be used immediately and never cached across extractions.
pub(crate) fn extract_master_url(html: &str) -> Result<String> {
    if !html.contains("playerConfig") {
        bail!("no playerConfig found on page");
    }
    let captures = SOURCES_PATTERN
        .captures(html)
        .context("playerConfig present but no `sources:` object literal")?;
    let raw = captures
        .get(1)
        .context("playerConfig `sources` capture group missing")?
        .as_str();
    let sources: Sources =
        serde_json::from_str(raw).context("playerConfig `sources` is not valid JSON")?;
    sources
        .hls_auto
        .context("playerConfig `sources` has no hlsAuto entry")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO_PAGE: &str = include_str!("tests/pornoxo_video_page.html");

    #[test]
    fn extracts_master_url_from_real_page() {
        let url = extract_master_url(VIDEO_PAGE).expect("fixture has a playerConfig");
        assert!(url.starts_with("https://cdn.pornoxo.com/key="));
        assert!(url.contains("/media=hls4A/"));
        assert!(url.ends_with("_TPL_.mp4"));
    }

    #[test]
    fn unescapes_json_slashes() {
        // The page embeds "https:\/\/cdn..." — serde_json must un-escape it.
        let url = extract_master_url(VIDEO_PAGE).expect("fixture has a playerConfig");
        assert!(!url.contains(r"\/"), "escaped slashes survived: {url}");
    }

    #[test]
    fn errors_when_playerconfig_absent() {
        let e = extract_master_url("<html><body>nothing here</body></html>").unwrap_err();
        assert!(e.to_string().contains("playerConfig"), "got: {e}");
    }

    #[test]
    fn errors_when_sources_lacks_hls_auto() {
        let html =
            r#"<script>var playerConfig = { sources: {"mp4":"https://x.test/a.mp4"}, };</script>"#;
        let e = extract_master_url(html).unwrap_err();
        assert!(e.to_string().contains("hlsAuto"), "got: {e}");
    }

    #[test]
    fn errors_when_sources_is_not_json() {
        let html = r#"<script>var playerConfig = { sources: {not json at all}, };</script>"#;
        assert!(extract_master_url(html).is_err());
    }
}
