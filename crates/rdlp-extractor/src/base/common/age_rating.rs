//! RTA (Restricted to Adults) age-rating detection.
//!
//! Mirrors yt-dlp's `InfoExtractor._rta_search` (`common.py:1520-1539`,
//! <https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/common.py>).
//! RTA labeling (<https://www.rtalabel.org/>) is a self-declared adult-content
//! tag still deployed in the wild as of 2026-07-17 (live-verified on
//! pornhub.com and xhamster.com, which serve the meta tag with differing
//! `name` casing — `rating` vs `RATING` — hence the case-insensitive match).

use lazy_regex::{Lazy, Regex, lazy_regex};

/// Age (in years) implied by a bare RTA/2257 marker with no explicit number
/// captured. RTA labeling and 18 U.S.C. §2257 record-keeping both denote
/// adult (18+) content; this is the fixed default yt-dlp's `_rta_search`
/// uses when a marker's capture group is absent.
const DEFAULT_ADULT_AGE: u8 = 18;

/// The canonical RTA meta tag content value, registered by rtalabel.org.
/// Kept as an exact-string match (not a fuzzy pattern) so that a future
/// change to the RTA scheme fails safe — no match, no `age_limit` — rather
/// than misclassifying an unrelated `rating` meta tag.
///
/// Deliberately NOT anchored to the tag's closing `>`: yt-dlp's `_rta_search`
/// stops at the closing quote, and a real tag may carry further attributes
/// after `content=` (`<meta name="rating" content="RTA-…" data-nosnippet="1">`).
/// Requiring the terminator here silently dropped those pages.
///
/// Attribute ORDER (`name` before `content`) is required, matching upstream —
/// a reordered tag is missed by yt-dlp too, so this is parity, not a gap we
/// introduce.
static RTA_META_TAG: Lazy<Regex> =
    lazy_regex!(r#"(?i)<meta\s+name="rating"\s+content="RTA-5042-1996-1400-1577-RTA""#);

/// RTA-label anchor marker: the rtalabel.org attribution link some sites
/// embed instead of (or alongside) the meta tag.
static RTA_LABEL_MARKER: Lazy<Regex> = lazy_regex!(
    r#"Proudly Labeled <a href="http://www\.rtalabel\.org/" title="Restricted to Adults">RTA</a>"#
);

/// Age-acknowledgment marker; captures an explicit age in group 1.
static RTA_AGE_ACK_MARKER: Lazy<Regex> =
    lazy_regex!(r">[^<]*you acknowledge you are at least (\d+) years old");

/// The 18 U.S.C. §2257 record-keeping marker, matched separately since it
/// has no capture group and always implies [`DEFAULT_ADULT_AGE`].
static RTA_2257_MARKER: Lazy<Regex> =
    lazy_regex!(r">\s*(?:18\s+U(?:\.S\.C\.|SC)\s+)?(?:§+\s*)?2257\b");

/// Detect a self-declared age restriction from RTA labeling or adjacent
/// adult-content markers in an HTML page.
///
/// Returns the highest age implied across all matched markers, or `None`
/// when none match (an *unknown* age limit — distinct from "no restriction",
/// which would be `Some(0)`).
#[must_use]
pub fn rta_search(html: &str) -> Option<u8> {
    if RTA_META_TAG.is_match(html) {
        return Some(DEFAULT_ADULT_AGE);
    }

    let mut age_limit: Option<u8> = None;

    for re in [&*RTA_LABEL_MARKER, &*RTA_AGE_ACK_MARKER] {
        if let Some(cap) = re.captures(html) {
            let val = cap
                .get(1)
                .and_then(|g| g.as_str().parse::<u8>().ok())
                .unwrap_or(DEFAULT_ADULT_AGE);
            age_limit = Some(age_limit.map_or(val, |x| x.max(val)));
        }
    }

    if RTA_2257_MARKER.is_match(html) {
        age_limit = Some(age_limit.map_or(DEFAULT_ADULT_AGE, |x| x.max(DEFAULT_ADULT_AGE)));
    }

    age_limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rta_meta_tag_lowercase_name_yields_18() {
        let html = r#"<html><head><meta name="rating" content="RTA-5042-1996-1400-1577-RTA"></head></html>"#;
        assert_eq!(rta_search(html), Some(18));
    }

    /// xhamster serves `name="RATING"` (uppercase) — the match must be
    /// case-insensitive, unlike a naive exact-string port would be.
    #[test]
    fn rta_meta_tag_uppercase_name_yields_18() {
        let html = r#"<html><head><meta name="RATING" content="RTA-5042-1996-1400-1577-RTA"></head></html>"#;
        assert_eq!(rta_search(html), Some(18));
    }

    /// The tag may carry further attributes after `content=`. An earlier draft
    /// anchored the pattern to the closing `>` and silently dropped these —
    /// yt-dlp's `_rta_search` stops at the closing quote, so anchoring was a
    /// false-negative we invented rather than inherited.
    #[test]
    fn rta_meta_tag_with_trailing_attribute_yields_18() {
        let html =
            r#"<meta name="rating" content="RTA-5042-1996-1400-1577-RTA" data-nosnippet="1">"#;
        assert_eq!(rta_search(html), Some(18));
    }

    /// Self-closing form, as served by xhamster (`…-RTA"/>`).
    #[test]
    fn rta_meta_tag_self_closing_yields_18() {
        let html = r#"<meta name="RATING" content="RTA-5042-1996-1400-1577-RTA"/>"#;
        assert_eq!(rta_search(html), Some(18));
    }

    #[test]
    fn no_tag_and_no_markers_yields_none() {
        let html = r#"<html><head><title>Just a page</title></head></html>"#;
        assert_eq!(rta_search(html), None);
    }

    #[test]
    fn section_2257_marker_yields_18() {
        let html = r#"<html><body><footer>See our >18 USC 2257 statement</footer></body></html>"#;
        assert_eq!(rta_search(html), Some(18));
    }

    #[test]
    fn age_acknowledgment_marker_parses_captured_number() {
        let html = r#"<html><body><p>By entering >you acknowledge you are at least 21 years old</p></body></html>"#;
        assert_eq!(rta_search(html), Some(21));
    }

    #[test]
    fn multiple_markers_max_wins() {
        let html = r#"<html><body>
            <p>>you acknowledge you are at least 18 years old</p>
            <footer>>2257</footer>
        </body></html>"#;
        assert_eq!(rta_search(html), Some(18));
    }

    #[test]
    fn higher_acknowledged_age_beats_bare_2257_default() {
        let html = r#"<html><body>
            <p>>you acknowledge you are at least 21 years old</p>
            <footer>>2257</footer>
        </body></html>"#;
        assert_eq!(rta_search(html), Some(21));
    }

    /// An unrelated `rating` meta tag with a different content value must
    /// not false-positive as RTA (exact-string match, not fuzzy).
    #[test]
    fn unrelated_rating_meta_tag_yields_none() {
        let html = r#"<html><head><meta name="rating" content="general"></head></html>"#;
        assert_eq!(rta_search(html), None);
    }
}
