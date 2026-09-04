//! Listing-grid parsing shared by the `/tags/` and `/search/` routes.

use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_types::SearchResultPreview;
use scraper::{Html, Selector};
use std::sync::LazyLock;

use crate::base::common::BaseExtractor;

/// The RESULTS grid only.
///
/// `.main-listing-grid-offset` is load-bearing, not decoration. A load that
/// also serves the two "Hot new Videos" recommendation grids carries three
/// `ul.media-listing-grid` elements, and the unscoped
/// `ul.media-listing-grid li` collects all 72 tiles where the site shows 52.
/// Measured against both committed fixtures with `scraper` itself: the class
/// selector returns 52 on each, the unscoped one returns 52 then 72.
///
/// Note for anyone re-reading issue #658: its `#maincolumn ul.media-listing-grid
/// li` also returns 52, because the recommendation grids sit OUTSIDE
/// `#maincolumn` — so scoping by id is not what makes this correct, and the
/// class is doing the work. Pinned by
/// `parses_the_results_grid_not_the_recommendation_grids`.
static GRID_ITEM: LazyLock<Selector> =
    crate::static_selector!("ul.media-listing-grid.main-listing-grid-offset > li");
static CARD_LINK: LazyLock<Selector> = crate::static_selector!("a.mtile-x7__title");
static CARD_THUMB: LazyLock<Selector> = crate::static_selector!("img");
static CARD_LENGTH: LazyLock<Selector> = crate::static_selector!("span.mtile-x7__info-item-length");

/// The `Next` pagination anchor, e.g. `href="/tags/creampie/?page=2"`.
static NEXT_PAGE: Lazy<Regex> =
    lazy_regex!(r#"<a class="rightKey" href="[^"]*"[^>]*>\s*Next\s*</a>"#);

/// The `>>` (last page) anchor, e.g. `href="/tags/creampie/?page=37"`.
///
/// The site emits the chevrons as literal `>>`, not the `&gt;&gt;` entity, and
/// `page=` is reached via `?` on an unsorted listing and `&`/`&amp;` once a
/// `sort` precedes it — hence all three spellings are accepted.
static LAST_PAGE: Lazy<Regex> = lazy_regex!(
    r#"<a class="rightKey" href="[^"]*[?&](?:amp;)?page=(\d+)"[^>]*>\s*(?:&gt;&gt;|>>)\s*</a>"#
);

/// Parse the results grid into previews, resolving each card against `origin`.
///
/// Cards with no href or no title are skipped rather than failing the page: a
/// single malformed tile must not cost the operator the other 51 results.
pub(crate) fn parse_listing(origin: &str, html: &str) -> Vec<SearchResultPreview> {
    let document = Html::parse_document(html);
    document
        .select(&GRID_ITEM)
        .filter_map(|li| {
            let link = li.select(&CARD_LINK).next()?;
            let href = link.value().attr("href")?;
            let title = link
                .value()
                .attr("title")
                .map(str::to_owned)
                .unwrap_or_else(|| link.text().collect::<String>().trim().to_owned());
            if title.is_empty() {
                return None;
            }
            Some(SearchResultPreview {
                video_url: format!("{origin}{href}"),
                title,
                thumbnail_url: li
                    .select(&CARD_THUMB)
                    .next()
                    .and_then(|i| i.value().attr("src"))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
                // The card's `H:MM:SS` / `M:SS` label is exactly the colon
                // vocabulary `BaseExtractor::parse_duration` already owns for
                // four sibling extractors; a site-local copy would be a
                // thirteenth.
                duration: li
                    .select(&CARD_LENGTH)
                    .next()
                    .and_then(|s| BaseExtractor::parse_duration(&s.text().collect::<String>())),
                uploader: None,
                uploader_url: None,
                actors: vec![],
                // The grid markup carries no per-card view count or date.
                view_count: None,
                upload_date: None,
            })
        })
        .collect()
}

/// Whether a `Next` pagination anchor is present.
///
/// This is the ONLY safe `has_more` signal for this site: `?page=999` answers
/// HTTP 200 with page 1's videos, so "stop when the grid comes back empty"
/// never terminates.
pub(crate) fn has_next_page(html: &str) -> bool {
    NEXT_PAGE.is_match(html)
}

/// The highest page number, read from the `>>` anchor. `None` on the last page,
/// and on any page whose pager the site did not render.
pub(crate) fn max_page(html: &str) -> Option<u32> {
    LAST_PAGE.captures(html)?.get(1)?.as_str().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The everyday production shape: one grid, no recommendation blocks.
    const TAG_PAGE: &str = include_str!("tests/pornoxo_tag_page.html");

    /// The SAME tag listing, captured minutes apart, on a load where the site
    /// also served two "Hot new Videos" recommendation grids. Those grids are
    /// served intermittently, so [`TAG_PAGE`] cannot discriminate a selector
    /// that over-collects — against it the wrong selector also returns 52 and
    /// the guard below passes green. This fixture exists solely so that
    /// assertion can fail. It is not a duplicate; do not delete it.
    const TAG_PAGE_RECOMMENDATIONS: &str =
        include_str!("tests/pornoxo_tag_page_recommendations.html");

    const ORIGIN: &str = "https://www.pornoxo.com";

    /// The load-bearing test: this fixture carries two extra 10-card
    /// recommendation grids, so a selector that drops
    /// `.main-listing-grid-offset` collects 52 + 10 + 10 = 72 here and fails.
    /// Verified by mutation, not assumed.
    #[test]
    fn parses_the_results_grid_not_the_recommendation_grids() {
        let results = parse_listing(ORIGIN, TAG_PAGE_RECOMMENDATIONS);
        assert_eq!(
            results.len(),
            52,
            "must not absorb the two 10-card recommendation grids (52 + 10 + 10 = 72)"
        );
    }

    /// The one-grid capture is a real production shape too, and must parse to
    /// the same 52 rows.
    #[test]
    fn parses_a_page_that_carries_only_the_results_grid() {
        assert_eq!(parse_listing(ORIGIN, TAG_PAGE).len(), 52);
    }

    #[test]
    fn parses_first_card_fields() {
        let listing = parse_listing(ORIGIN, TAG_PAGE);
        let r = &listing[0];
        assert_eq!(r.title, "Test V*E*C*XFiveFive Six Mosaic Removed");
        assert_eq!(
            r.video_url,
            "https://www.pornoxo.com/videos/2939836/test-v-e-c-xfivefive-six-mosaic-removed/"
        );
        // 1:43:11 — pins that the card's length span is routed through the
        // shared `BaseExtractor::parse_duration`, which owns this vocabulary.
        assert_eq!(r.duration, Some(6191.0));
        assert_eq!(
            r.thumbnail_url.as_deref(),
            Some(
                "https://cdn77-t.pornoxo.com/b-pornoxo/thumbs/pxo-320x240/2026-08/b3/\
                 abd8ecfb35ed90985ef6ac7a826c0017b.mp4-320x240-5.jpg"
            )
        );
    }

    #[test]
    fn every_card_has_an_absolute_video_url_and_a_duration() {
        let listing = parse_listing(ORIGIN, TAG_PAGE);
        assert!(
            listing
                .iter()
                .all(|r| r.video_url.starts_with("https://www.pornoxo.com/videos/")),
            "every card resolves against the origin that served the listing"
        );
        assert!(
            listing.iter().all(|r| r.duration.is_some()),
            "every card in this capture carries a length span"
        );
    }

    /// The origin is threaded in rather than hardcoded, so a listing served by
    /// one host never yields card URLs pointing at another.
    #[test]
    fn resolves_cards_against_the_supplied_origin() {
        let listing = parse_listing("http://127.0.0.1:1234", TAG_PAGE);
        assert!(
            listing[0]
                .video_url
                .starts_with("http://127.0.0.1:1234/videos/")
        );
    }

    #[test]
    fn reads_pagination_bounds() {
        assert!(has_next_page(TAG_PAGE));
        // Both captures are page 1 of `/tags/creampie/`; the site's own `>>`
        // anchor reported a different last page on each (37 vs 159), so these
        // are per-capture observations, not an invariant of the tag.
        assert_eq!(max_page(TAG_PAGE), Some(37));
        assert!(has_next_page(TAG_PAGE_RECOMMENDATIONS));
        assert_eq!(max_page(TAG_PAGE_RECOMMENDATIONS), Some(159));
    }

    #[test]
    fn last_page_has_no_next_link() {
        // On the final page the site emits neither the `Next` nor the `>>`
        // anchor; both hang off `class="rightKey"`, so renaming it models that.
        let last = TAG_PAGE.replace(r#"<a class="rightKey""#, r#"<a class="notrightKey""#);
        assert!(!has_next_page(&last));
        assert_eq!(max_page(&last), None);
    }

    #[test]
    fn empty_page_yields_no_results() {
        assert!(parse_listing(ORIGIN, "<html><body></body></html>").is_empty());
    }
}
