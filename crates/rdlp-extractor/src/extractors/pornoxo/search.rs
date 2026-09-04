//! Listing-grid parsing shared by the `/tags/` and `/search/` routes.

use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_types::{SearchQuery, SearchResultPreview};
use scraper::{Html, Selector};
use std::sync::LazyLock;
use url::form_urlencoded;

use crate::base::common::{BaseExtractor, SearchOrigin, filter_value};

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
pub(crate) fn parse_listing(origin: &SearchOrigin, html: &str) -> Vec<SearchResultPreview> {
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

/// PornoXO's production origin.
const DEFAULT_ORIGIN: &str = "https://www.pornoxo.com";

/// The production origin, as the typed value the URL builders take.
pub(crate) fn default_origin() -> SearchOrigin {
    SearchOrigin::from_static(DEFAULT_ORIGIN)
}

/// Normalise a free-text query into a tag slug: lowercased, whitespace runs
/// collapsed to a single `-`.
///
/// The server is forgiving here — `big-ass`, `big%20ass` and `bigass` all
/// resolve — so this is for tidy URLs, not correctness. Percent-encoding it
/// afterwards is NOT cosmetic: the slug is operator text interpolated into a
/// PATH, and an unencoded `?` or `&` would graft chosen parameters onto the
/// request (including a second `page=`, which would defeat the clamp guard).
fn tag_slug(query: &str) -> String {
    let normalised = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    form_urlencoded::byte_serialize(normalised.as_bytes()).collect()
}

/// Build the listing URL for `page` of `query`, on whichever route its `route`
/// filter selects.
///
/// `route=tag` puts the query in the PATH as a tag slug (`/tags/<slug>/`);
/// anything else uses the default full-text route (`/search/?q=`). Filters are
/// emitted in [`URL_FILTER_KEYS`] order so the output is deterministic
/// regardless of the order the caller supplied them in.
///
/// [`URL_FILTER_KEYS`]: super::search_patterns::URL_FILTER_KEYS
pub(crate) fn build_listing_url(origin: &SearchOrigin, query: &SearchQuery, page: u32) -> String {
    let mut pairs: Vec<(&str, String)> = Vec::new();

    let path = if filter_value(&query.filters, "route") == Some("tag") {
        format!("/tags/{}/", tag_slug(&query.query))
    } else {
        pairs.push((
            "q",
            form_urlencoded::byte_serialize(query.query.as_bytes()).collect(),
        ));
        "/search/".to_owned()
    };

    for key in super::search_patterns::URL_FILTER_KEYS {
        if let Some(value) = filter_value(&query.filters, key) {
            pairs.push((
                key,
                form_urlencoded::byte_serialize(value.as_bytes()).collect(),
            ));
        }
    }
    pairs.push(("page", page.to_string()));

    let query_string = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{origin}{path}?{query_string}")
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
    use rdlp_types::{SearchFilter, SearchQuery};

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

    /// The load-bearing test: this fixture carries two extra 10-card
    /// recommendation grids, so a selector that drops
    /// `.main-listing-grid-offset` collects 52 + 10 + 10 = 72 here and fails.
    /// Verified by mutation, not assumed.
    #[test]
    fn parses_the_results_grid_not_the_recommendation_grids() {
        let results = parse_listing(&default_origin(), TAG_PAGE_RECOMMENDATIONS);
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
        assert_eq!(parse_listing(&default_origin(), TAG_PAGE).len(), 52);
    }

    #[test]
    fn parses_first_card_fields() {
        let listing = parse_listing(&default_origin(), TAG_PAGE);
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
        let listing = parse_listing(&default_origin(), TAG_PAGE);
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
        let origin = SearchOrigin::new("http://127.0.0.1:1234").unwrap();
        let listing = parse_listing(&origin, TAG_PAGE);
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

    fn q(query: &str, filters: &[(&str, &str)]) -> SearchQuery {
        SearchQuery {
            query: query.to_owned(),
            filters: filters
                .iter()
                .map(|(k, v)| SearchFilter {
                    key: (*k).to_owned(),
                    value: (*v).to_owned(),
                })
                .collect(),
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn builds_the_tag_route_url_from_the_query_as_slug() {
        assert_eq!(
            build_listing_url(
                &default_origin(),
                &q("big ass", &[("route", "tag"), ("sort", "mr")]),
                3
            ),
            "https://www.pornoxo.com/tags/big-ass/?sort=mr&page=3"
        );
    }

    #[test]
    fn builds_the_search_route_url_by_default() {
        assert_eq!(
            build_listing_url(&default_origin(), &q("creampie", &[]), 1),
            "https://www.pornoxo.com/search/?q=creampie&page=1"
        );
    }

    #[test]
    fn percent_encodes_the_search_query() {
        let u = build_listing_url(&default_origin(), &q("big ass & more", &[]), 1);
        assert!(
            u.contains("q=big+ass+%26+more") || u.contains("q=big%20ass%20%26%20more"),
            "{u}"
        );
    }

    /// A tag name is operator-supplied text pasted into a URL PATH. Without
    /// encoding, a `?` or `&` in it would graft attacker-chosen parameters
    /// onto the request — including a `page=` that defeats the clamp guard.
    #[test]
    fn a_tag_slug_cannot_inject_query_parameters() {
        let u = build_listing_url(
            &default_origin(),
            &q("x/?page=1&sort=zz", &[("route", "tag")]),
            2,
        );
        assert!(u.ends_with("&page=2") || u.ends_with("?page=2"), "{u}");
        assert_eq!(u.matches("page=").count(), 1, "exactly one page param: {u}");
        assert!(!u.contains("sort=zz"), "slug must not become a filter: {u}");
    }

    #[test]
    fn forwards_every_supplied_filter_in_declaration_order() {
        let u = build_listing_url(
            &default_origin(),
            &q(
                "x",
                &[
                    ("filter_length", "long"),
                    ("sort", "lg"),
                    ("filter_quality", "hd"),
                ],
            ),
            1,
        );
        assert_eq!(
            u,
            "https://www.pornoxo.com/search/?q=x&sort=lg&filter_quality=hd&filter_length=long&page=1",
            "filter order follows URL_FILTER_KEYS, not the caller's ordering"
        );
    }

    #[test]
    fn slug_normalisation_lowercases_and_hyphenates() {
        let u = build_listing_url(
            &default_origin(),
            &q("  Big   ASS  ", &[("route", "tag")]),
            1,
        );
        assert!(
            u.starts_with("https://www.pornoxo.com/tags/big-ass/?"),
            "{u}"
        );
    }

    #[test]
    fn empty_page_yields_no_results() {
        assert!(parse_listing(&default_origin(), "<html><body></body></html>").is_empty());
    }
}
