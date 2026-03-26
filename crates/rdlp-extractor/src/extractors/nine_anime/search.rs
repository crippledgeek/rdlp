//! Search result parsing and SearchExtractor implementation for NineAnime.
//!
//! 9anime search pages are server-rendered HTML with URL-based pagination
//! (`?page=N`, 0-based). No AJAX needed — standard HTML scraping.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{SearchPageResponse, SearchQuery, SearchResultPreview};
use std::time::Duration;

use super::search_patterns;
use super::NineAnimeExtractor;
use crate::base::common::BaseExtractor;

const BASE_URL: &str = "https://9animetv.to";
const MAX_PLAYLIST_SIZE: usize = 500;

/// Parse search result items from 9anime search page HTML.
///
/// Uses regex to extract film-name links and match them with thumbnails
/// and episode counts by position.
pub(crate) fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let names: Vec<_> = search_patterns::FILM_NAME_PATTERN
        .captures_iter(html)
        .collect();
    let thumbs: Vec<_> = search_patterns::THUMBNAIL_PATTERN
        .captures_iter(html)
        .collect();
    let episodes: Vec<_> = search_patterns::EPISODE_PATTERN
        .captures_iter(html)
        .collect();

    names
        .iter()
        .enumerate()
        .map(|(i, cap)| {
            let href = &cap[1];
            let title = cap[2].to_string();
            let video_url = format!("{BASE_URL}{href}");

            let thumbnail_url = thumbs.get(i).map(|t| t[1].to_string());

            // Use episode text as format_note (e.g., "Ep 34/34")
            let _episode_info = episodes.get(i).map(|e| e[1].trim().to_string());

            SearchResultPreview {
                video_url,
                title,
                thumbnail_url,
                duration: None, // anime search doesn't show per-episode duration
                view_count: None,
                upload_date: None,
            }
        })
        .collect()
}

/// Check whether the page has a "Next" pagination link.
pub(crate) fn has_next_page(html: &str) -> bool {
    search_patterns::NEXT_PAGE_PATTERN.is_match(html)
}

/// Extract the total page count from "of {N}" text.
pub(crate) fn extract_total_pages(html: &str) -> Option<u32> {
    search_patterns::TOTAL_PAGES_PATTERN
        .captures(html)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

#[async_trait]
impl SearchExtractor for NineAnimeExtractor {
    fn name(&self) -> &str {
        "NineAnime"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        search_patterns::search_filter_descriptors()
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut page = 0_u32; // 0-based pagination

        loop {
            let page_url = search_patterns::build_search_url(query, page);
            let sanitized = rdlp_security::sanitize_for_logging(&page_url);
            debug!("[NineAnime] Fetching search page {}: {sanitized}", page + 1);

            let webpage = BaseExtractor::fetch_webpage_with_headers(
                &page_url,
                &[("Referer", "https://9animetv.to/")],
                ctx,
            )
            .await?;

            let page_results = parse_search_results(&webpage);
            if page_results.is_empty() {
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if !has_next_page(&webpage) {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(search_patterns::PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(
            "[NineAnime] Search complete: {} results across {} pages",
            all_results.len(),
            page + 1
        );
        Ok(all_results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        // Convert 1-based user page to 0-based URL page
        let user_page = query.page.unwrap_or(1);
        let url_page = if user_page > 0 { user_page - 1 } else { 0 };

        let page_url = search_patterns::build_search_url(query, url_page);

        let webpage = BaseExtractor::fetch_webpage_with_headers(
            &page_url,
            &[("Referer", "https://9animetv.to/")],
            ctx,
        )
        .await?;

        let page_results = parse_search_results(&webpage);
        let has_more = has_next_page(&webpage) && !page_results.is_empty();
        let total_pages = extract_total_pages(&webpage);
        let total_estimate =
            total_pages.map(|tp| tp as u64 * search_patterns::RESULTS_PER_PAGE);

        Ok(SearchPageResponse {
            results: page_results,
            page: user_page,
            has_more,
            total_estimate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_search_html() -> &'static str {
        r#"<html><body>
<div class="film_list-wrap">

<div class="flw-item item-qtip" data-id="643">
    <div class="film-poster">
        <div class="tick-item tick-quality">HD</div>
        <div class="tick ltr">
            <div class="tick-item tick-sub">SUB</div>
            <div class="tick-item tick-dub">DUB</div>
        </div>
        <div class="tick rtl">
            <div class="tick-item tick-eps">Ep 34/34</div>
        </div>
        <img data-src="https://cdn.noitatnemucod.net/thumbnail/300x400/100/thumb1.jpg"
             class="film-poster-img lazyload" alt="Sailor Moon: Sailor Stars">
        <a href="/watch/sailor-moon-sailor-stars-643" class="film-poster-ahref"><i class="fas fa-play"></i></a>
    </div>
    <div class="film-detail">
        <h3 class="film-name"><a href="/watch/sailor-moon-sailor-stars-643" title="Sailor Moon: Sailor Stars" class="dynamic-name"
                                 data-jname="Bishoujo Senshi Sailor Moon: Sailor Stars">Sailor Moon: Sailor Stars</a></h3>
    </div>
</div>

<div class="flw-item item-qtip" data-id="3635">
    <div class="film-poster">
        <div class="tick-item tick-quality">HD</div>
        <div class="tick ltr">
            <div class="tick-item tick-sub">SUB</div>
            <div class="tick-item tick-dub">DUB</div>
        </div>
        <div class="tick rtl">
            <div class="tick-item tick-eps">Ep 26/26</div>
        </div>
        <img data-src="https://cdn.noitatnemucod.net/thumbnail/300x400/100/thumb2.jpg"
             class="film-poster-img lazyload" alt="Sailor Moon Crystal">
        <a href="/watch/sailor-moon-crystal-season-i-ii-3635" class="film-poster-ahref"><i class="fas fa-play"></i></a>
    </div>
    <div class="film-detail">
        <h3 class="film-name"><a href="/watch/sailor-moon-crystal-season-i-ii-3635" title="Sailor Moon Crystal" class="dynamic-name"
                                 data-jname="Bishoujo Senshi Sailor Moon Crystal">Sailor Moon Crystal</a></h3>
    </div>
</div>

<div class="flw-item item-qtip" data-id="1067">
    <div class="film-poster">
        <div class="tick-item tick-quality">HD</div>
        <div class="tick ltr">
            <div class="tick-item tick-sub">SUB</div>
        </div>
        <div class="tick rtl">
            <div class="tick-item tick-eps">Ep Full</div>
        </div>
        <img data-src="https://cdn.noitatnemucod.net/thumbnail/300x400/100/thumb3.jpg"
             class="film-poster-img lazyload" alt="Sailor Moon Movie">
        <a href="/watch/sailor-moon-movie-1067" class="film-poster-ahref"><i class="fas fa-play"></i></a>
    </div>
    <div class="film-detail">
        <h3 class="film-name"><a href="/watch/sailor-moon-movie-1067" title="Sailor Moon Movie" class="dynamic-name"
                                 data-jname="Sailor Moon Movie">Sailor Moon Movie</a></h3>
    </div>
</div>

</div>

<div class="anime-pagination">
    <div class="ap_-nav">
        <div class="ap__-btn ap__-btn-prev"><a href="/search?keyword=sailor moon&page=0" class="btn btn-sm btn-focus more-padding disabled">Previous</a></div>
        <div class="ap__-input"><div class="btn btn-sm btn-blank">page</div><input class="form-control" value="1"><button type="button" class="btn btn-sm btn-focus btn-go-page">go</button><div class="btn btn-sm btn-blank">of 3</div></div>
        <div class="ap__-btn ap__-btn-next"><a href="/search?keyword=sailor moon&page=2" class="btn btn-sm btn-focus more-padding ">Next <i class="fas fa-angle-right"></i></a></div>
    </div>
</div>
</body></html>"#
    }

    fn sample_last_page_html() -> &'static str {
        r#"<html><body>
<div class="film_list-wrap">
<div class="flw-item item-qtip" data-id="999">
    <div class="film-poster">
        <div class="tick rtl">
            <div class="tick-item tick-eps">Ep 12/12</div>
        </div>
        <img data-src="https://cdn.noitatnemucod.net/thumb4.jpg"
             class="film-poster-img lazyload" alt="Last Anime">
        <a href="/watch/last-anime-999" class="film-poster-ahref"></a>
    </div>
    <div class="film-detail">
        <h3 class="film-name"><a href="/watch/last-anime-999" title="Last Anime" class="dynamic-name">Last Anime</a></h3>
    </div>
</div>
</div>

<div class="anime-pagination">
    <div class="ap_-nav">
        <div class="ap__-btn ap__-btn-prev"><a href="/search?keyword=test&page=1" class="btn">Previous</a></div>
        <div class="ap__-input"><div class="btn btn-sm btn-blank">of 3</div></div>
    </div>
</div>
</body></html>"#
    }

    #[test]
    fn test_parse_search_results() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "Sailor Moon: Sailor Stars");
        assert_eq!(
            results[0].video_url,
            "https://9animetv.to/watch/sailor-moon-sailor-stars-643"
        );
        assert_eq!(
            results[0].thumbnail_url.as_deref(),
            Some("https://cdn.noitatnemucod.net/thumbnail/300x400/100/thumb1.jpg")
        );
        assert_eq!(results[0].duration, None); // anime search has no duration
    }

    #[test]
    fn test_parse_search_results_second_item() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results[1].title, "Sailor Moon Crystal");
        assert_eq!(
            results[1].video_url,
            "https://9animetv.to/watch/sailor-moon-crystal-season-i-ii-3635"
        );
    }

    #[test]
    fn test_parse_search_results_third_item() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results[2].title, "Sailor Moon Movie");
    }

    #[test]
    fn test_parse_search_results_empty() {
        let results = parse_search_results("<html><body></body></html>");
        assert!(results.is_empty());
    }

    #[test]
    fn test_has_next_page_true() {
        assert!(has_next_page(sample_search_html()));
    }

    #[test]
    fn test_has_next_page_false() {
        assert!(!has_next_page(sample_last_page_html()));
    }

    #[test]
    fn test_extract_total_pages() {
        assert_eq!(extract_total_pages(sample_search_html()), Some(3));
    }

    #[test]
    fn test_extract_total_pages_none() {
        assert_eq!(extract_total_pages("<html>no pagination</html>"), None);
    }

    #[test]
    fn test_search_name() {
        let ext = NineAnimeExtractor::new();
        assert_eq!(SearchExtractor::name(&ext), "NineAnime");
    }

    #[test]
    fn test_supported_filters() {
        let ext = NineAnimeExtractor::new();
        let filters = SearchExtractor::supported_filters(&ext);
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[0].allowed_values.len(), 7);
    }

    #[test]
    fn test_search_page_converts_1_based_to_0_based() {
        // Verify that user page 1 maps to URL page 0
        let query = SearchQuery {
            query: "test".to_string(),
            filters: vec![],
            max_results: None,
            page: Some(1),
        };
        let url = search_patterns::build_search_url(&query, 0); // page 1 → URL page 0
        assert!(url.contains("page=0"));
    }
}
