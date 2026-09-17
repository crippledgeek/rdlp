//! Search listing parsing for pornone.com's `/search/` route.
//!
//! The site pads three distinct situations — no match, a page past the real
//! end, and an out-of-range page — with the SAME HTTP 200 "Popular Videos"
//! grid (measured 2026-09-13, #659). Neither the status code nor the row
//! count can tell a real page from filler; only the page's own content
//! (`No results` text or the `Popular Videos` heading) can, so [`Listing`]
//! carries that verdict explicitly rather than leaving callers to infer it
//! from an empty result vec (a real page's last page CAN legitimately be
//! empty in principle, though this site has not been observed to do so).

use rdlp_types::{SearchQuery, SearchResultPreview};
use scraper::{Html, Selector};
use std::sync::LazyLock;

use crate::base::common::{
    BaseExtractor, SearchOrigin, first_resolvable_media_attr, resolve_card_url,
};

const DEFAULT_ORIGIN: &str = "https://pornone.com";

pub(crate) fn default_origin() -> SearchOrigin {
    SearchOrigin::from_static(DEFAULT_ORIGIN)
}

/// Both markers were measured on every filler shape (#659: no-match,
/// page-past-end, out-of-range) and on no real result page; either alone
/// is decisive.
const FILLER_TEXT_MARKER: &str = "No results";
const FILLER_HEADING: &str = "Popular Videos";

static CARD: LazyLock<Selector> = crate::static_selector!("a.videocard[href]");
static CARD_TITLE: LazyLock<Selector> = crate::static_selector!(".videotitle");
/// No `[data-src]`/`[src]` qualifier: the capture shows eagerly-loaded cards
/// (first two, `fetchpriority="high"`) carry a real thumbnail in `src` with
/// no `data-src` attribute at all, while the remaining lazy-loaded cards
/// carry `src=""` and the real URL in `data-src`. Restricting the selector to
/// `[data-src]` (as originally drafted) silently drops the eager cards'
/// thumbnails — caught by asserting every result in the real capture, not a
/// hand-picked sample.
static CARD_THUMB: LazyLock<Selector> = crate::static_selector!("img.thumbimg");
static CARD_DURATION: LazyLock<Selector> = crate::static_selector!(".durlabel");
static CARD_UPLOADER: LazyLock<Selector> = crate::static_selector!(".author .font-semibold");
static HEADINGS: LazyLock<Selector> = crate::static_selector!("h2");

pub(crate) struct Listing {
    pub results: Vec<SearchResultPreview>,
    pub is_filler: bool,
}

pub(crate) fn build_search_url(origin: &SearchOrigin, query: &SearchQuery, page: u32) -> String {
    let q = rdlp_security::percent_encode_query_value(&query.query);
    format!("{origin}/search/?q={q}&page={page}")
}

pub(crate) fn parse_search_page(origin: &SearchOrigin, html: &str) -> Listing {
    let document = Html::parse_document(html);
    let is_filler = html.contains(FILLER_TEXT_MARKER)
        || document
            .select(&HEADINGS)
            .any(|h| h.text().collect::<String>().trim() == FILLER_HEADING);
    if is_filler {
        return Listing {
            results: Vec::new(),
            is_filler,
        };
    }
    let results = document
        .select(&CARD)
        .filter_map(|a| {
            let video_url = resolve_card_url(origin.as_ref(), a.value().attr("href")?)?;
            // Only video pages are results — same routing as `is_suitable`.
            if !super::patterns::is_suitable(&video_url) {
                return None;
            }
            let title = a
                .select(&CARD_TITLE)
                .next()?
                .text()
                .collect::<String>()
                .trim()
                .to_owned();
            if title.is_empty() {
                return None;
            }
            Some(SearchResultPreview {
                video_url,
                title,
                thumbnail_url: a.select(&CARD_THUMB).next().and_then(|i| {
                    first_resolvable_media_attr(&i, origin.as_ref(), &["data-src", "src"], |_| true)
                }),
                duration: a.select(&CARD_DURATION).next().and_then(|d| {
                    BaseExtractor::parse_duration(d.text().collect::<String>().trim())
                }),
                uploader: a
                    .select(&CARD_UPLOADER)
                    .next()
                    .map(|u| u.text().collect::<String>().trim().to_owned())
                    .filter(|u| !u.is_empty()),
                uploader_url: None,
                actors: Vec::new(),
                view_count: None,
                upload_date: None,
            })
        })
        .collect();
    Listing { results, is_filler }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_PAGE: &str = include_str!("tests/pornone_search_page.html");
    const FILLER_PAGE: &str = include_str!("tests/pornone_search_filler.html");

    fn origin() -> SearchOrigin {
        default_origin()
    }

    #[test]
    fn a_real_results_page_parses_its_cards() {
        let l = parse_search_page(&origin(), SEARCH_PAGE);
        assert!(!l.is_filler);
        assert!(l.results.len() >= 30, "{}", l.results.len());
        for r in &l.results {
            assert!(
                r.video_url.starts_with("https://pornone.com/"),
                "{}",
                r.video_url
            );
            assert!(
                crate::extractors::pornone::patterns::is_suitable(&r.video_url),
                "{}",
                r.video_url
            );
            assert!(!r.title.trim().is_empty());
            assert!(r.duration.is_some_and(|d| d > 0.0), "{:?}", r.duration);
            assert!(
                r.thumbnail_url
                    .as_deref()
                    .is_some_and(|t| t.starts_with("https://th-")),
                "{:?}",
                r.thumbnail_url
            );
        }
    }

    /// The filler trap: no-match, page-past-end and out-of-range all return
    /// HTTP 200 with the same fixed 35-video grid. Either marker makes the
    /// page zero results, and its grid is never parsed.
    #[test]
    fn a_filler_page_yields_zero_results() {
        let l = parse_search_page(&origin(), FILLER_PAGE);
        assert!(l.is_filler);
        assert!(l.results.is_empty());
    }

    #[test]
    fn each_filler_marker_alone_is_sufficient() {
        let card = r#"<a href="https://pornone.com/a/b/1/" class="popbop vidLinkFX  videocard linkage"><div class="videotitle">t</div></a>"#;
        let no_results = format!("<html><body><p>No results</p>{card}</body></html>");
        let popular = format!("<html><body><h2>Popular Videos</h2>{card}</body></html>");
        assert!(parse_search_page(&origin(), &no_results).results.is_empty());
        assert!(parse_search_page(&origin(), &popular).results.is_empty());
        // and a page with neither marker parses the card
        let plain = format!("<html><body>{card}</body></html>");
        assert_eq!(parse_search_page(&origin(), &plain).results.len(), 1);
    }

    #[test]
    fn a_card_cannot_move_the_authority_off_pornone() {
        for hostile in [
            "https://evil.test/a/b/1/",
            "//evil.test/a/b/1/",
            "https://pornone.com@evil.test/a/b/1/",
            "javascript:alert(1)",
        ] {
            let html = format!(
                r#"<a href="{hostile}" class="popbop vidLinkFX  videocard linkage"><div class="videotitle">t</div></a>"#
            );
            assert!(
                parse_search_page(&origin(), &html).results.is_empty(),
                "{hostile}"
            );
        }
    }

    fn query(q: &str) -> SearchQuery {
        SearchQuery {
            query: q.to_owned(),
            filters: Vec::new(),
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn builds_the_search_url_with_encoding_and_page() {
        let q = query("big tits & more");
        assert_eq!(
            build_search_url(&origin(), &q, 1),
            "https://pornone.com/search/?q=big%20tits%20%26%20more&page=1"
        );
    }
}
