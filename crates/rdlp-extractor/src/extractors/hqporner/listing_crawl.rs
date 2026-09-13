//! Bounded crawl state for hqporner listing pagination (#722).

use std::collections::HashSet;

use crate::base::common::MAX_PLAYLIST_SIZE;
use crate::extractors::hqporner::search_patterns::next_listing_page_url;

/// Video cards on one hqporner listing page, measured on
/// `https://hqporner.com/category/amateur` on 2026-09-13 (50 distinct
/// `/hdporn/…html` hrefs). Only the *order of magnitude* matters here: it
/// sizes the page cap below.
const LISTING_CARDS_PER_PAGE_MEASURED: usize = 50;

/// Hard ceiling on listing pages fetched in one crawl. A healthy listing
/// fills [`MAX_PLAYLIST_SIZE`] in `MAX_PLAYLIST_SIZE / cards-per-page`
/// pages; the factor of two is headroom for a site-side layout change that
/// halves the card count. The cap exists for the unhealthy case — every
/// card extraction failing, so `all_results` never grows and the
/// size-based exit never fires — and bounds that case to this many GETs.
pub(super) const MAX_LISTING_PAGES: usize =
    2 * MAX_PLAYLIST_SIZE.div_ceil(LISTING_CARDS_PER_PAGE_MEASURED);

/// Pagination state for one listing crawl: which page URLs have been
/// fetched, so a self-referential or cyclic Next link is a terminal, not a
/// loop, and how many pages have been admitted against [`MAX_LISTING_PAGES`].
pub(super) struct ListingCrawl {
    visited: HashSet<String>,
}

impl ListingCrawl {
    /// Start a crawl at `entry`; the entry page is already visited.
    pub(super) fn starting_at(entry: &str) -> Self {
        Self {
            visited: HashSet::from([entry.to_owned()]),
        }
    }

    /// The next page to fetch after `page_url` served `webpage`, or `None`
    /// when the crawl stops: no Next link or an off-origin one (both
    /// decided by [`next_listing_page_url`]), a URL already fetched, or the
    /// page cap reached. The caller treats every `None` the same way —
    /// return what it has gathered.
    pub(super) fn next_page(&mut self, webpage: &str, page_url: &str) -> Option<String> {
        let next = next_listing_page_url(webpage, page_url)?;
        if self.visited.len() >= MAX_LISTING_PAGES {
            log::debug!(
                "[HQPorner] Listing page cap {MAX_LISTING_PAGES} reached, stopping pagination"
            );
            return None;
        }
        if !self.visited.insert(next.clone()) {
            log::debug!(
                "[HQPorner] Pagination revisits {}, stopping",
                rdlp_redact::RedactedUrl::new(&next)
            );
            return None;
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "https://hqporner.com/category/amateur";

    fn next_link(href: &str) -> String {
        format!(r#"<a href="{href}" class="pagi-btn">Next</a>"#)
    }

    #[test]
    fn fresh_next_href_is_returned() {
        let mut crawl = ListingCrawl::starting_at(ENTRY);
        assert_eq!(
            crawl.next_page(&next_link("/category/amateur/2"), ENTRY),
            Some("https://hqporner.com/category/amateur/2".to_string())
        );
    }

    #[test]
    fn self_referential_next_terminates() {
        // A Next href that resolves to the page that served it is a fixed
        // point; without the visited set the old loop spun on it whenever
        // every card extraction failed.
        let mut crawl = ListingCrawl::starting_at(ENTRY);
        assert_eq!(
            crawl.next_page(&next_link("/category/amateur"), ENTRY),
            None
        );
    }

    #[test]
    fn revisiting_an_earlier_page_terminates() {
        let mut crawl = ListingCrawl::starting_at(ENTRY);
        let p2 = crawl
            .next_page(&next_link("/category/amateur/2"), ENTRY)
            .expect("page 2 is new");
        assert_eq!(crawl.next_page(&next_link("/category/amateur"), &p2), None);
    }

    #[test]
    fn no_next_link_terminates() {
        let mut crawl = ListingCrawl::starting_at(ENTRY);
        assert_eq!(crawl.next_page("<div>no pagination</div>", ENTRY), None);
    }

    #[test]
    fn page_cap_is_enforced_at_the_boundary() {
        // Boundary on both sides: page MAX_LISTING_PAGES is admitted,
        // page MAX_LISTING_PAGES + 1 is refused.
        let mut crawl = ListingCrawl::starting_at(ENTRY);
        let mut current = ENTRY.to_string();
        for n in 2..=MAX_LISTING_PAGES {
            let href = format!("/category/amateur/{n}");
            current = crawl
                .next_page(&next_link(&href), &current)
                .unwrap_or_else(|| panic!("page {n} is within the cap"));
        }
        let over = format!("/category/amateur/{}", MAX_LISTING_PAGES + 1);
        assert_eq!(crawl.next_page(&next_link(&over), &current), None);
    }
}
