//! Extraction of the signed HLS master URL from PornoXO's inline `playerConfig`.
//!
//! The page carries `sources` as a JSON object literal with escaped slashes
//! (`"https:\/\/cdn..."`), so it is parsed with `serde_json` rather than a
//! hand-rolled unescape.

use anyhow::{Context, Result, bail};
use lazy_regex::{Lazy, Regex, lazy_regex};
use serde::Deserialize;

/// The `sources:` object literal inside `var playerConfig = { ... }`.
///
/// The match is ANCHORED on the `playerConfig` occurrence and spans forward to
/// the nearest following `sources:`. An unanchored `sources:` search would take
/// the first one anywhere in the document — an ad script, a comment, a second
/// player, a user-controlled field — and `hlsAuto` decides what we fetch, so
/// page text placed before the player block would choose our target.
///
/// The anchor is the DECLARATION (`var playerConfig =`), not the bare word.
/// Matching the word alone would let any mention re-open the window — a
/// `<!-- playerConfig -->` comment, a `var notTheplayerConfig`, a UGC field —
/// and a decoy `sources:` following that mention would win again. The `var` is
/// optional so a reassignment or a differently-declared player still matches,
/// but `\s*=` is required, so a mention that assigns nothing cannot anchor.
///
/// `\b` is what makes "the identifier" rather than "the substring" the unit.
/// Without it `(?:var\s+)?` is optional, so the pattern matched inside ANY
/// identifier ending in `playerConfig` — `xplayerConfig={sources:…}` in an ad
/// script would have chosen our fetch target. Measured: `\b` rejects
/// `myplayerConfig`, `notTheplayerConfig` and `xplayerConfig` while still
/// admitting `var playerConfig`, `window.playerConfig` and a bare
/// `playerConfig =`.
///
/// `(?s)` so the window crosses newlines; `{0,4096}?` is non-greedy, so the
/// FIRST `sources:` after the anchor wins rather than a later one. The 4096
/// bound keeps the window inside the player block: in the captured fixture
/// `sources:` follows the declaration by ~30 characters, so this is ~2 orders
/// of magnitude of headroom while still stopping the scan from running on into
/// unrelated script further down the page.
///
/// `[^}]*` is safe because `sources` has no nested objects.
static SOURCES_PATTERN: Lazy<Regex> =
    lazy_regex!(r"(?s)(?:var\s+)?\bplayerConfig\s*=.{0,4096}?sources:\s*(\{[^}]*\})");

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
    // `SOURCES_PATTERN` already requires the `playerConfig` anchor, so this
    // check does not narrow the search — it only separates "no player on this
    // page" (login wall, removed video) from "player present but reshaped",
    // which are different things for whoever reads the log.
    if !html.contains("playerConfig") {
        bail!("no playerConfig found on page");
    }
    let captures = SOURCES_PATTERN
        .captures(html)
        .context("playerConfig present but no `sources:` object literal within reach of it")?;
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

    /// Guards the CHOICE of `serde_json` over a hand-rolled unescape, not any
    /// logic of ours: it cannot fail while the parse goes through serde. It
    /// earns its place by failing the day someone replaces that with manual
    /// string surgery, which is the tempting shortcut here.
    #[test]
    fn unescapes_json_slashes() {
        // The page embeds "https:\/\/cdn..." — serde_json must un-escape it.
        let url = extract_master_url(VIDEO_PAGE).expect("fixture has a playerConfig");
        assert!(!url.contains(r"\/"), "escaped slashes survived: {url}");
    }

    /// The match must be anchored to the `playerConfig` block, not merely
    /// gated on the word appearing somewhere. A `sources:` literal placed
    /// EARLIER in the document — an ad script, a second player, a UGC field —
    /// must not win: `hlsAuto` decides what we fetch, so letting page text
    /// before the player choose it hands an attacker the target.
    #[test]
    fn decoy_sources_before_the_player_block_is_ignored() {
        let html = concat!(
            r#"<script>var adConfig = { sources: {"hlsAuto":"https://decoy.test/x.mp4"}, };</script>"#,
            r#"<script>var playerConfig = { sources: {"hlsAuto":"https://real.test/y.mp4"}, };</script>"#
        );
        let url = extract_master_url(html).expect("must find the real player block");
        assert_eq!(url, "https://real.test/y.mp4");
    }

    /// The anchor must match the DECLARATION, not the bare word. Text that
    /// merely mentions `playerConfig` — a comment, a differently-named
    /// variable, a UGC field — would otherwise re-open the window and let a
    /// following decoy win, which is the same defect the anchor closed.
    ///
    /// Every decoy here is spelled with the SAME capitalisation as the real
    /// identifier, and the loop is what makes that non-negotiable: the
    /// original single-case version of this test used `notThePlayerConfig`
    /// and passed against a regex with no `\b` purely because a capital `P`
    /// cannot match a case-sensitive `playerConfig`. It asserted a spelling,
    /// not the property. Measured against the unpatched regex, three of these
    /// four cases resolve to the decoy.
    #[test]
    fn a_mere_mention_of_player_config_does_not_open_the_window() {
        for decoy_name in [
            "notTheplayerConfig",
            "window.myplayerConfig",
            "xplayerConfig",
            "notThePlayerConfig",
        ] {
            let html = format!(
                r#"<!-- playerConfig --><script>var {decoy_name} = {{ sources: {{"hlsAuto":"https://decoy.test/x.mp4"}}, }};</script><script>var playerConfig = {{ sources: {{"hlsAuto":"https://real.test/y.mp4"}}, }};</script>"#
            );
            let url = extract_master_url(&html).expect("must find the real player block");
            assert_eq!(
                url, "https://real.test/y.mp4",
                "`{decoy_name}` must not re-open the window"
            );
        }
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
