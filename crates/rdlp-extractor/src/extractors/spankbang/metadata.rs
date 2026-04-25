//! Page-level metadata extraction for SpankBang (title, thumbnail, duration,
//! uploader). All functions are pure and testable against captured HTML.

use std::collections::HashMap;

use super::patterns;

/// Aggregate metadata pulled from a SpankBang video page.
///
/// `actors` and `tags` come from the horizontal `<div class="searches">`
/// bar near the top of the page (studio channel + pornstars + tag-search
/// links). Scoping the tag harvest to that container keeps recommendation
/// rails and footer categories out of the result.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PageMetadata {
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) thumbnail: Option<String>,
    pub(super) uploader: Option<String>,
    pub(super) duration_secs: Option<u64>,
    pub(super) uploader_id: Option<String>,
    pub(super) actors: Vec<String>,
    pub(super) tags: Vec<String>,
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

    // Profile-link uploader (user uploads to a SpankBang profile page).
    // Try the plain-text anchor form first; fall back to the modern
    // icon-wrapped form (Subscribe-button chrome with `<span class="name">`).
    let (mut uploader_id, mut uploader) = patterns::UPLOADER_LINK
        .captures(html)
        .or_else(|| patterns::UPLOADER_LINK_NAMED.captures(html))
        .map(|c| {
            let slug = c.get(1).map(|m| m.as_str().to_string());
            let name = c.get(2).map(|m| m.as_str().trim().to_string());
            (slug, name)
        })
        .unwrap_or((None, None));

    // Studio / pornstars / tags — all live inside the <div class="searches">
    // tag bar near the page header. Scoping the parse there avoids picking
    // up unrelated /s/<slug>/ anchors elsewhere on the page.
    let mut actors: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    if let Some(bar) = patterns::SEARCHES_BAR.captures(html).and_then(|c| c.get(1)) {
        let inner = bar.as_str();

        // Studio / channel link in the bar overrides any blank profile-link
        // uploader; if both exist, prefer the studio (matches the screenshot
        // hierarchy where the badge nearest the title is the studio).
        if let Some(c) = patterns::CHANNEL_LINK_BAR.captures(inner) {
            let slug = c.get(1).map(|m| m.as_str().to_string());
            let name = c.get(2).map(|m| m.as_str().trim().to_string());
            if uploader.is_none() {
                uploader = name;
            }
            if uploader_id.is_none() {
                uploader_id = slug;
            }
        }

        for c in patterns::PORNSTAR_LINK.captures_iter(inner) {
            if let Some(name) = c.get(1) {
                let n = name.as_str().trim();
                if !n.is_empty() && !actors.iter().any(|x| x == n) {
                    actors.push(n.to_string());
                }
            }
        }

        for c in patterns::TAG_LINK_BAR.captures_iter(inner) {
            if let Some(name) = c.get(1) {
                let n = name.as_str().trim();
                // Skip badge-style entries that aren't real tags ("Exclusive"
                // promo flag, empty captures from img-only anchors, etc.).
                if n.is_empty()
                    || n.eq_ignore_ascii_case("exclusive")
                    || actors.iter().any(|a| a.eq_ignore_ascii_case(n))
                    || tags.iter().any(|t| t.eq_ignore_ascii_case(n))
                {
                    continue;
                }
                tags.push(n.to_string());
            }
        }
    }

    PageMetadata {
        title,
        description,
        thumbnail,
        uploader,
        duration_secs,
        uploader_id,
        actors,
        tags,
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
        assert_eq!(m.uploader.as_deref(), Some("GammaEntertainment"));
    }

    #[test]
    fn parses_uploader_from_icon_wrapped_profile_link() {
        // Amateur uploads render the profile link with leading icon SVG +
        // <span class="name"> + chevron SVG instead of plain text. Captured
        // 2026-04-26 from /a4fcc/video/porn (uploader: shocker4).
        const AMATEUR_PAGE: &str =
            include_str!("tests/spankbang_video_page_amateur.html");
        let m = parse(AMATEUR_PAGE);
        assert_eq!(
            m.uploader.as_deref(),
            Some("shocker4"),
            "icon-wrapped /profile/shocker4 link must extract via UPLOADER_LINK_NAMED"
        );
        assert_eq!(m.uploader_id.as_deref(), Some("shocker4"));
    }

    #[test]
    fn parses_actors_and_tags_from_searches_bar() {
        let m = parse(PAGE);

        // Pornstars from /pornstar/<slug>/ links inside the searches bar.
        // Fixture page is the Dogfart video → Slim Poke + Katalina Kyle.
        assert!(
            m.actors.iter().any(|a| a.eq_ignore_ascii_case("Slim Poke")),
            "actors should include Slim Poke; got {:?}",
            m.actors
        );
        assert!(
            m.actors
                .iter()
                .any(|a| a.eq_ignore_ascii_case("Katalina Kyle")),
            "actors should include Katalina Kyle; got {:?}",
            m.actors
        );

        // Tags from /s/<slug>/ links inside the searches bar; recommendation
        // rails and footer categories must NOT bleed in.
        let lc: Vec<String> = m.tags.iter().map(|t| t.to_lowercase()).collect();
        for expected in ["interracial", "blowjob", "blonde"] {
            assert!(
                lc.iter().any(|t| t == expected),
                "tags should include {expected}; got {:?}",
                m.tags
            );
        }

        // De-duplication: every tag appears at most once.
        let mut sorted = lc.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), lc.len(), "tags must be deduped: {:?}", m.tags);

        // No tag string should be empty or the "Exclusive" badge label.
        assert!(m.tags.iter().all(|t| !t.is_empty()));
        assert!(
            m.tags
                .iter()
                .all(|t| !t.eq_ignore_ascii_case("exclusive")),
            "Exclusive promo badge must be filtered out"
        );
    }

    #[test]
    fn description_present_from_og() {
        let m = parse(PAGE);
        // Fixture's description is ~145 chars; threshold of 50 catches a
        // truncation regression while staying robust to minor copy edits.
        assert!(m.description.unwrap_or_default().len() > 50);
    }
}
