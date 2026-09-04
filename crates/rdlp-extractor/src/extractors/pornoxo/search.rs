//! Listing-grid parsing shared by the `/tags/` and `/search/` routes.

use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_types::{SearchQuery, SearchResultPreview};
use scraper::{Html, Selector};
use std::sync::LazyLock;
use url::form_urlencoded;

use crate::base::common::{
    BaseExtractor, SearchOrigin, filter_value, resolve_card_url, resolve_media_url,
};

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
/// The tile's poster: a direct child of the tile anchor, not merely the first
/// `img` in the tile. Scoped like `CARD_LINK` and `CARD_LENGTH` so a badge or
/// overlay icon added elsewhere in the tile cannot displace it. Neither
/// fixture discriminates these selectors, so the guard is the synthetic
/// `a_badge_image_does_not_displace_the_poster_thumbnail`. If the site ever
/// wraps the poster, this fails CLOSED (no thumbnail) rather than silently
/// reporting the wrong image.
static CARD_THUMB: LazyLock<Selector> = crate::static_selector!("div.mtile-x7__inner > a > img");
static CARD_LENGTH: LazyLock<Selector> = crate::static_selector!("span.mtile-x7__info-item-length");

/// Both pagination anchors: `Next` and `>>` (last page) each carry this class.
///
/// Matched as a SELECTOR, not a regex over raw HTML. The previous regexes were
/// anchored on the literal prefix `<a class="rightKey" href="`, so a reordered
/// or inserted attribute made them miss — and they failed OPEN: pagination
/// stopped after page 1 and, worse, `max_page` went `None`, which skips the
/// clamp guard and would serve page 1's videos labelled as the requested page.
/// CSS matching is order-insensitive and is the convention this file already
/// uses for the grid.
static PAGER_LINK: LazyLock<Selector> = crate::static_selector!("a.rightKey");

/// The `page` parameter, read from a pager anchor's own `href`.
///
/// Scoped to one attribute value rather than the whole document, so it cannot
/// be perturbed by markup around it. `?page=` on an unsorted listing and
/// `&page=` once a filter precedes it; `&amp;` never reaches here because the
/// HTML parser has already decoded the attribute.
static PAGE_PARAM: Lazy<Regex> = lazy_regex!(r"[?&]page=(\d+)");

/// One parsed listing page: the cards plus both pagination facts.
///
/// Returned together because `fetch_page` needs all three and the document is
/// 250-315 KB — parsing it once per fetch rather than once per question.
pub(crate) struct Listing {
    pub results: Vec<SearchResultPreview>,
    /// Whether a `Next` anchor is present.
    ///
    /// The ONLY safe `has_more` signal for this site: `?page=999` answers HTTP
    /// 200 with page 1's videos, so "stop when the grid comes back empty"
    /// never terminates.
    pub has_next: bool,
    /// The highest page number, from the `>>` anchor. `None` on the last page
    /// and on any page whose pager the site did not render.
    pub max_page: Option<u32>,
}

/// Parse a listing page: cards resolved against `origin`, plus its pager.
pub(crate) fn parse_listing_page(origin: &SearchOrigin, html: &str) -> Listing {
    let document = Html::parse_document(html);
    let mut has_next = false;
    let mut max_page = None;
    for anchor in document.select(&PAGER_LINK) {
        // Compared as decoded TEXT, so the literal `>>` this site emits and the
        // `&gt;&gt;` entity form both arrive here identically.
        match anchor.text().collect::<String>().trim() {
            "Next" => has_next = true,
            ">>" => {
                max_page = anchor
                    .value()
                    .attr("href")
                    .and_then(|href| PAGE_PARAM.captures(href))
                    .and_then(|c| c.get(1))
                    // Saturating, not `.parse().ok()`. `PAGE_PARAM` captures
                    // `(\d+)`, so the only way this parse fails is a value too
                    // large for `u32` — and `None` is the value that means "no
                    // pager on this page", which SKIPS the out-of-range guard
                    // in `fetch_page`. Collapsing an unrepresentable bound into
                    // it would make "no bound" and "a bound we could not read"
                    // indistinguishable, silently and in the unguarded
                    // direction. `u32::MAX` refuses no realistic page.
                    .map(|m| m.as_str().parse::<u32>().unwrap_or(u32::MAX));
            }
            _ => {}
        }
    }
    Listing {
        results: parse_cards(&document, origin),
        has_next,
        max_page,
    }
}

/// Parse the results grid into previews, resolving each card against `origin`.
///
/// Cards with no href, no title, or an href that resolves OFF the origin are
/// skipped rather than failing the page: a single malformed tile must not cost
/// the operator the other 51 results.
fn parse_cards(document: &Html, origin: &SearchOrigin) -> Vec<SearchResultPreview> {
    document
        .select(&GRID_ITEM)
        .filter_map(|li| {
            let link = li.select(&CARD_LINK).next()?;
            let href = link.value().attr("href")?;
            // Resolved, not concatenated. `href` is listing HTML and a bare
            // origin has no trailing slash, so `format!("{origin}{href}")` let
            // page content choose the authority — `@evil.test/…` yields the
            // well-formed `https://www.pornoxo.com@evil.test/…`, whose host is
            // `evil.test`. See `resolve_card_url` for the measured shapes.
            let video_url = resolve_card_url(origin.as_ref(), href)?;
            let title = link
                .value()
                .attr("title")
                .map(str::to_owned)
                .unwrap_or_else(|| link.text().collect::<String>().trim().to_owned());
            if title.is_empty() {
                return None;
            }
            Some(SearchResultPreview {
                video_url,
                title,
                // The desktop renders this straight into an `<img src>`, so a
                // `data:` or `javascript:` value from the listing must not
                // reach it. Host-agnostic on purpose: real posters are served
                // from `cdn77-t.pornoxo.com`, off the listing origin.
                thumbnail_url: li
                    .select(&CARD_THUMB)
                    .next()
                    .and_then(|i| i.value().attr("src"))
                    .filter(|s| !s.is_empty())
                    .and_then(|src| resolve_media_url(origin.as_ref(), src)),
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
/// emitted in [`URL_FILTER_PARAMS`] order, under the site parameter names that
/// table maps them to, so the output is deterministic regardless of the order
/// the caller supplied them in.
///
/// [`URL_FILTER_PARAMS`]: super::search_patterns::URL_FILTER_PARAMS
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

    for (key, param) in super::search_patterns::URL_FILTER_PARAMS {
        if let Some(value) = filter_value(&query.filters, key) {
            pairs.push((
                param,
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
        let results = parse_listing_page(&default_origin(), TAG_PAGE_RECOMMENDATIONS).results;
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
        assert_eq!(
            parse_listing_page(&default_origin(), TAG_PAGE)
                .results
                .len(),
            52
        );
    }

    #[test]
    fn parses_first_card_fields() {
        let listing = parse_listing_page(&default_origin(), TAG_PAGE).results;
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
        let listing = parse_listing_page(&default_origin(), TAG_PAGE).results;
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

    /// The documented fail-soft: a malformed tile costs that tile only.
    ///
    /// `parse_cards` skips a card with no href and a card with no title rather
    /// than failing the page, "a single malformed tile must not cost the
    /// operator the other 51 results". Every neighbouring property here has a
    /// test and this one did not, so the sentence was an assertion nothing
    /// checked — and the tempting simplification of the `filter_map` is a `?`
    /// that propagates instead of skipping.
    #[test]
    fn a_tile_with_no_href_costs_only_itself() {
        let stripped = TAG_PAGE.replacen(
            r#"<a href="/videos/2939836/test-v-e-c-xfivefive-six-mosaic-removed/" class="mtile-x7__title""#,
            r#"<a class="mtile-x7__title""#,
            1,
        );
        assert_ne!(stripped, TAG_PAGE, "the href must actually be stripped");
        assert_eq!(
            parse_listing_page(&default_origin(), &stripped).results.len(),
            51,
            "one hrefless tile must cost one card, not the page"
        );
    }

    /// The title half of the same branch, on a synthesised grid because the
    /// capture has no untitled tile: both the `title` attribute and the anchor
    /// text feed the fallback, so making a real tile titleless means emptying
    /// both, and a two-tile grid states that far more plainly.
    #[test]
    fn a_tile_with_no_title_costs_only_itself() {
        let grid = r#"<ul class="media-listing-grid main-listing-grid-offset">
            <li><a href="/videos/1/untitled/" class="mtile-x7__title" title="">  </a></li>
            <li><a href="/videos/2/real/" class="mtile-x7__title" title="Real">Real</a></li>
        </ul>"#;
        let results = parse_listing_page(&default_origin(), grid).results;
        assert_eq!(results.len(), 1, "the titleless tile must be skipped");
        assert_eq!(results[0].title, "Real");
    }

    /// The tile's POSTER, not merely its first `img`.
    ///
    /// Neither committed fixture can discriminate here — measured, every
    /// candidate selector (`img`, `.mtile-x7__inner img`, `.mtile-x7__inner >
    /// a > img`) returns the poster for all 52 tiles on both captures. So the
    /// discriminating case is synthesised: a badge or overlay icon added ahead
    /// of the poster would silently replace every thumbnail URL, and an
    /// unqualified `img` takes whichever comes first.
    #[test]
    fn a_badge_image_does_not_displace_the_poster_thumbnail() {
        let with_badge = TAG_PAGE.replace(
            r#"<li class="js-pop mtile-x7 ">"#,
            r#"<li class="js-pop mtile-x7 "><img src="https://badge.example/new.png" alt="new">"#,
        );
        assert_ne!(with_badge, TAG_PAGE, "the badge must actually be injected");

        let listing = parse_listing_page(&default_origin(), &with_badge).results;
        assert_eq!(listing.len(), 52, "the badge must not change tile parsing");
        assert!(
            listing.iter().all(|r| r
                .thumbnail_url
                .as_deref()
                .is_some_and(|t| !t.contains("badge.example"))),
            "a badge img must never be taken as the poster: {:?}",
            listing[0].thumbnail_url
        );
    }

    /// Every card on both captures carries a real poster — `parses_first_card_fields`
    /// checks one card on one fixture, which would not notice a selector that
    /// works only for the first tile.
    #[test]
    fn every_card_on_both_captures_has_a_cdn_thumbnail() {
        for html in [TAG_PAGE, TAG_PAGE_RECOMMENDATIONS] {
            let listing = parse_listing_page(&default_origin(), html).results;
            assert_eq!(listing.len(), 52);
            assert!(
                listing.iter().all(|r| r
                    .thumbnail_url
                    .as_deref()
                    .is_some_and(|t| t.starts_with("https://") && t.contains("/thumbs/"))),
                "every tile must resolve a CDN poster"
            );
        }
    }

    /// Replace the first tile's href with `hostile`, so the assertions below
    /// speak about a real grid with exactly one poisoned card in it.
    fn tag_page_with_first_href(hostile: &str) -> String {
        let real = r#"<a href="/videos/2939836/test-v-e-c-xfivefive-six-mosaic-removed/" class="mtile-x7__title""#;
        let poisoned = format!(r#"<a href="{hostile}" class="mtile-x7__title""#);
        let out = TAG_PAGE.replacen(real, &poisoned, 1);
        assert_ne!(out, TAG_PAGE, "the hostile href must actually be injected");
        out
    }

    /// The authority must come from the origin we fetched, never from the
    /// listing body.
    ///
    /// Asserted on the PARSED HOST, not on a `starts_with` prefix, because the
    /// prefix test is blind to the shape that makes this real:
    /// `https://www.pornoxo.com@evil.test/…` starts with the origin and is a
    /// perfectly well-formed URL whose host is `evil.test`. Under the previous
    /// `format!("{origin}{href}")` all four hrefs below produced an authority
    /// the page chose.
    #[test]
    fn no_card_can_move_the_authority_off_the_listing_origin() {
        let origin = default_origin();
        for hostile in [
            "@evil.test/videos/1/x/",
            ".evil.test/videos/1/x/",
            "https://evil.test/videos/1/x/",
            "//evil.test/videos/1/x/",
        ] {
            let listing = parse_listing_page(&origin, &tag_page_with_first_href(hostile)).results;
            for r in &listing {
                let host = url::Url::parse(&r.video_url)
                    .expect("every emitted card URL must parse")
                    .host_str()
                    .map(str::to_owned);
                assert_eq!(
                    host.as_deref(),
                    Some("www.pornoxo.com"),
                    "href {hostile:?} moved the authority: {}",
                    r.video_url
                );
            }
        }
    }

    /// A reference that CAN carry its own authority is dropped outright; one
    /// that cannot is resolved onto the origin and kept. The distinction is
    /// RFC 3986's, not ours: `@evil.test/…` is a relative-path reference, so
    /// joining it can only ever produce a path.
    #[test]
    fn an_off_origin_href_is_dropped_and_a_relative_one_is_resolved() {
        let origin = default_origin();

        for droppable in ["https://evil.test/videos/1/x/", "//evil.test/videos/1/x/"] {
            let listing = parse_listing_page(&origin, &tag_page_with_first_href(droppable)).results;
            assert_eq!(
                listing.len(),
                51,
                "{droppable:?} must cost exactly its own card"
            );
        }

        for neutralised in ["@evil.test/videos/1/x/", ".evil.test/videos/1/x/"] {
            let listing =
                parse_listing_page(&origin, &tag_page_with_first_href(neutralised)).results;
            assert_eq!(
                listing.len(),
                52,
                "{neutralised:?} is a path, so its card survives"
            );
            assert!(
                listing[0].video_url.starts_with("https://www.pornoxo.com/"),
                "must resolve onto the origin's path space: {}",
                listing[0].video_url
            );
        }
    }

    /// A `data:` or `javascript:` poster would flow into the desktop's
    /// `<img src>`. The scheme check is the whole guard here — an origin
    /// comparison would be wrong, because real posters are served from
    /// `cdn77-t.pornoxo.com`, off the listing origin.
    #[test]
    fn a_non_http_thumbnail_is_dropped_and_a_cdn_one_is_kept() {
        // Targeted at the `src=` form: the same URL also appears once as a
        // `<link rel="preload" href=…>`, which no selector here reads, so a
        // positional `replacen` would poison nothing and pass vacuously.
        let real_poster = concat!(
            r#"src="https://cdn77-t.pornoxo.com/b-pornoxo/thumbs/pxo-320x240/2026-08/b3/"#,
            r#"abd8ecfb35ed90985ef6ac7a826c0017b.mp4-320x240-5.jpg""#
        );
        let poisoned = TAG_PAGE.replace(
            real_poster,
            r#"src="data:text/html,<script>alert(1)</script>""#,
        );
        assert_ne!(
            poisoned, TAG_PAGE,
            "the data: URL must actually be injected"
        );

        let listing = parse_listing_page(&default_origin(), &poisoned).results;
        assert_eq!(listing.len(), 52, "a bad poster must not cost the card");
        assert!(
            listing.iter().all(|r| r
                .thumbnail_url
                .as_deref()
                .is_none_or(|t| t.starts_with("https://") || t.starts_with("http://"))),
            "no non-http(s) poster may reach the UI: {:?}",
            listing[0].thumbnail_url
        );
        assert!(
            listing[0].thumbnail_url.is_none(),
            "the poisoned tile's poster must be dropped, not rewritten: {:?}",
            listing[0].thumbnail_url
        );
    }

    /// A pager value too large for `u32` must still read as "we found a
    /// bound", saturated, rather than as `None`.
    ///
    /// `None` is the value that means "this listing rendered no pager", and it
    /// SKIPS the out-of-range guard in `fetch_page` entirely. Letting an
    /// unparseable number collapse to it makes "no bound published" and "a
    /// bound we could not represent" indistinguishable — silently, and in the
    /// direction that removes the guard. `u32::MAX` refuses no realistic page,
    /// so the saturation costs nothing and keeps the two cases apart.
    #[test]
    fn an_oversized_pager_bound_saturates_rather_than_vanishing() {
        let huge = TAG_PAGE.replace(
            r#"<a class="rightKey" href="/tags/creampie/?page=37">>></a>"#,
            r#"<a class="rightKey" href="/tags/creampie/?page=99999999999">>></a>"#,
        );
        assert_ne!(
            huge, TAG_PAGE,
            "the oversized bound must actually be injected"
        );
        assert_eq!(
            parse_listing_page(&default_origin(), &huge).max_page,
            Some(u32::MAX),
            "an all-digits bound beyond u32 must saturate, not become None"
        );
    }

    /// The origin is threaded in rather than hardcoded, so a listing served by
    /// one host never yields card URLs pointing at another.
    #[test]
    fn resolves_cards_against_the_supplied_origin() {
        let origin = SearchOrigin::new("http://127.0.0.1:1234").unwrap();
        let listing = parse_listing_page(&origin, TAG_PAGE).results;
        assert!(
            listing[0]
                .video_url
                .starts_with("http://127.0.0.1:1234/videos/")
        );
    }

    #[test]
    fn reads_pagination_bounds() {
        let page = parse_listing_page(&default_origin(), TAG_PAGE);
        assert!(page.has_next);
        // Both captures are page 1 of `/tags/creampie/`; the site's own `>>`
        // anchor reported a different last page on each (37 vs 159), so these
        // are per-capture observations, not an invariant of the tag.
        assert_eq!(page.max_page, Some(37));
        let recs = parse_listing_page(&default_origin(), TAG_PAGE_RECOMMENDATIONS);
        assert!(recs.has_next);
        assert_eq!(recs.max_page, Some(159));
    }

    /// Attribute ORDER is not part of the contract. Both pagination anchors
    /// were matched by regexes anchored on the literal prefix
    /// `<a class="rightKey" href="`, so a reordered or extra attribute made
    /// `has_next_page` false and `max_page` None — silently, and the second is
    /// the dangerous one: `None` skips the clamp guard entirely, so an
    /// explicit `--page 999` would be served page 1's videos labelled 999.
    #[test]
    fn pagination_survives_reordered_attributes() {
        let reordered = TAG_PAGE
            .replace(
                r#"<a class="rightKey" href="/tags/creampie/?page=2">Next</a>"#,
                r#"<a href="/tags/creampie/?page=2" rel="next" class="rightKey">Next</a>"#,
            )
            .replace(
                r#"<a class="rightKey" href="/tags/creampie/?page=37">>></a>"#,
                r#"<a href="/tags/creampie/?page=37" rel="last" class="rightKey">>></a>"#,
            );
        assert_ne!(reordered, TAG_PAGE, "the replacements must actually apply");
        let page = parse_listing_page(&default_origin(), &reordered);
        assert!(page.has_next, "Next anchor must still be found");
        assert_eq!(page.max_page, Some(37), "clamp bound must survive");
    }

    #[test]
    fn last_page_has_no_next_link() {
        // On the final page the site emits neither the `Next` nor the `>>`
        // anchor; both hang off `class="rightKey"`, so renaming it models that.
        let last = TAG_PAGE.replace(r#"<a class="rightKey""#, r#"<a class="notrightKey""#);
        let page = parse_listing_page(&default_origin(), &last);
        assert!(!page.has_next);
        assert_eq!(page.max_page, None);
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
                &[("length", "long"), ("sort", "lg"), ("quality", "hd")],
            ),
            1,
        );
        assert_eq!(
            u,
            "https://www.pornoxo.com/search/?q=x&sort=lg&filter_quality=hd&filter_length=long&page=1",
            "rdlp's quality/length keys are sent as the site's \
             filter_quality/filter_length, in URL_FILTER_PARAMS order"
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
        assert!(
            parse_listing_page(&default_origin(), "<html><body></body></html>")
                .results
                .is_empty()
        );
    }
}
