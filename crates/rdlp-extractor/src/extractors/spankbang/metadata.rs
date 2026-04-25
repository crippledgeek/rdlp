//! Page-level metadata extraction for SpankBang (title, thumbnail, duration,
//! uploader). All functions are pure and testable against captured HTML.

use std::collections::HashMap;

use super::patterns;

/// Aggregate metadata pulled from a SpankBang video page.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PageMetadata {
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) thumbnail: Option<String>,
    pub(super) duration_secs: Option<u64>,
    pub(super) uploader_id: Option<String>,
}

/// `true` when the page renders the "video removed" sentinel.
pub(super) fn is_removed(html: &str) -> bool {
    patterns::VIDEO_REMOVED.is_match(html)
}

/// Collect all `<meta property="og:KEY" content="VALUE">` pairs.
fn collect_og(html: &str) -> HashMap<String, String> {
    patterns::OG_META
        .captures_iter(html)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

/// Parse all extractable metadata from a SpankBang video page HTML.
pub(super) fn parse(html: &str) -> PageMetadata {
    let og = collect_og(html);
    let title = patterns::TITLE_H1
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .or_else(|| og.get("title").cloned());

    let description = og.get("description").cloned();
    let thumbnail = og.get("image").cloned();
    let duration_secs = og.get("video:duration").and_then(|s| s.parse().ok());

    let uploader_id = patterns::UPLOADER_ID
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    PageMetadata {
        title,
        description,
        thumbnail,
        duration_secs,
        uploader_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("tests/spankbang_video_page.html");

    #[test]
    fn live_fixture_is_not_removed() {
        assert!(!is_removed(PAGE));
    }

    #[test]
    fn parses_title_thumbnail_duration_uploader() {
        let m = parse(PAGE);
        let title = m.title.expect("title required");
        assert!(
            title.to_lowercase().contains("dogfart"),
            "title should contain DOGFART, got: {title}"
        );
        let thumb = m.thumbnail.expect("og:image required");
        assert!(thumb.starts_with("https://"), "absolute thumbnail URL");
        assert_eq!(m.duration_secs, Some(910));
        assert_eq!(m.uploader_id.as_deref(), Some("gammaentertainment"));
    }

    #[test]
    fn description_present_from_og() {
        let m = parse(PAGE);
        // Fixture's description is ~145 chars; threshold of 50 catches a
        // truncation regression while staying robust to minor copy edits.
        assert!(m.description.unwrap_or_default().len() > 50);
    }
}
