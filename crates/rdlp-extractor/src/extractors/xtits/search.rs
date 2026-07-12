//! Search result parsing and SearchExtractor implementation for XTits.
//!
//! XTits uses KVS AJAX pagination. All pages (including page 1) are fetched
//! from the same async endpoint that returns HTML fragments.

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{SearchPageResponse, SearchQuery, SearchResultPreview};

use super::XTitsExtractor;
use super::search_patterns;
use crate::base::common::{PagedSearch, SearchPage, SearchPageSpec};

/// Maximum results cap for a full search (matches the pre-refactor
/// `unwrap_or(500)`; mirrors the xnxx/xvideos siblings' named cap).
const MAX_PLAYLIST_SIZE: usize = 500;

/// Parse search result items from KVS AJAX response HTML.
pub(crate) fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let items: Vec<_> = search_patterns::ITEM_PATTERN.captures_iter(html).collect();
    let durations: Vec<_> = search_patterns::DURATION_PATTERN
        .captures_iter(html)
        .collect();

    items
        .iter()
        .enumerate()
        .map(|(i, cap)| {
            let video_url = cap[1].to_string();
            let title = cap[2].to_string();
            let thumbnail_url = {
                let thumb = &cap[3];
                if thumb.is_empty() {
                    None
                } else {
                    Some(thumb.to_string())
                }
            };

            let duration = durations
                .get(i)
                .and_then(|d| search_patterns::parse_duration(&d[1]));

            SearchResultPreview {
                video_url,
                title,
                thumbnail_url,
                duration,
                uploader: None,
                uploader_url: None,
                actors: vec![],
                view_count: None,
                upload_date: None,
            }
        })
        .collect()
}

/// Detect the highest page number in pagination links.
///
/// Returns the max page number found, or the current page if no pagination links exist.
pub(crate) fn detect_max_page(html: &str) -> u32 {
    search_patterns::PAGE_NUMBER_PATTERN
        .captures_iter(html)
        .filter_map(|c| c.get(1)?.as_str().parse::<u32>().ok())
        .max()
        .unwrap_or(1)
}

impl PagedSearch for XTitsExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[XTits]"
    }

    // XTits has no filter validation today (the pre-refactor `run_search_page`
    // path never validated); `Ok(())` is the only value that preserves that.
    fn validate_search_filters(&self, _filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        Ok(())
    }

    fn first_page_index(&self) -> u32 {
        1
    }

    fn max_results_default(&self) -> usize {
        MAX_PLAYLIST_SIZE
    }

    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let spec = SearchPageSpec {
            first_page_index: 1, // struct field exists until 3b-8; fetch_via_spec ignores it
            headers: &[
                ("X-Requested-With", "XMLHttpRequest"),
                ("Referer", "https://www.xtits.com/search/"),
            ],
            build_url: search_patterns::build_search_url,
            parse: |body, _query, page| {
                let results = parse_search_results(body);
                let max_page = detect_max_page(body);
                Ok(SearchPage {
                    has_more: page < max_page && !results.is_empty(),
                    total_estimate: Some(max_page as u64 * search_patterns::RESULTS_PER_PAGE),
                    results,
                })
            },
        };
        self.fetch_via_spec(spec, query, page, ctx).await
    }
}

#[async_trait]
impl SearchExtractor for XTitsExtractor {
    fn name(&self) -> &str {
        "XTits"
    }

    fn supported_filters(&self) -> Vec<rdlp_types::SearchFilterDescriptor> {
        search_patterns::search_filter_descriptors()
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        self.search_all_pages(query, ctx).await
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        self.search_page_response(query, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_search_html() -> &'static str {
        r#"<div id="list_videos_videos_list_search_result" class="box">
<h1 class="title">Videos for: amateur, Page 1</h1>
<div class="item thumb-item">
    <a class="link js-open-popup" href="https://www.xtits.com/videos/50088/blonde-amateur-gf-amateur/" title="Blonde Amateur GF - Amateur" thumb="https://i.xtits.com/contents/videos_screenshots/50000/50088/402x225/2.jpg" vthumb="https://www.xtits.com/get_file/5/abc/50088vthumbs.mp4/">
        <div class="img-holder">
            <img class="thumb img" src="https://i.xtits.com/contents/videos_screenshots/50000/50088/402x225/2.jpg" alt="Blonde Amateur GF - Amateur">
            <span class="label hd"><i class="icon-hd"></i></span>
            <span class="label time"><i class="icon-hd"></i>10:35</span>
        </div>
        <div class="info-holder">
            <p class="title">Blonde Amateur GF - Amateur</p>
        </div>
    </a>
</div>
<div class="item thumb-item">
    <a class="link js-open-popup" href="https://www.xtits.com/videos/180969/chubby-amateur-fucked-amateur/" title="Chubby Amateur Fucked - Amateur" thumb="https://i.xtits.com/contents/videos_screenshots/180000/180969/402x225/10.jpg" vthumb="https://www.xtits.com/get_file/6/def/180969vthumbs.mp4/">
        <div class="img-holder">
            <img class="thumb img" src="https://i.xtits.com/contents/videos_screenshots/180000/180969/402x225/10.jpg" alt="Chubby Amateur Fucked - Amateur">
            <span class="label time"><i class="icon-hd"></i>26:47</span>
        </div>
        <div class="info-holder">
            <p class="title">Chubby Amateur Fucked - Amateur</p>
        </div>
    </a>
</div>
<div class="item thumb-item">
    <a class="link js-open-popup" href="https://www.xtits.com/videos/172127/amateur-bbw-2k-amateur/" title="Amateur BBW(2K) - Amateur" thumb="https://i.xtits.com/contents/videos_screenshots/172000/172127/402x225/20.jpg" vthumb="https://www.xtits.com/get_file/5/ghi/172127vthumbs.mp4/">
        <div class="img-holder">
            <img class="thumb img" src="https://i.xtits.com/contents/videos_screenshots/172000/172127/402x225/20.jpg" alt="Amateur BBW(2K) - Amateur">
            <span class="label time"><i class="icon-hd"></i>19:04</span>
        </div>
        <div class="info-holder">
            <p class="title">Amateur BBW(2K) - Amateur</p>
        </div>
    </a>
</div>
<div class="pagination" id="list_videos_videos_list_search_result_pagination">
    <ul class="pagination-holder">
        <li class="item-pagin active"><span class="link">01</span></li>
        <li class="item-pagin"><a class="link" data-parameters="q:amateur;sort_by:;from_videos+from_albums:02">02</a></li>
        <li class="item-pagin"><a class="link" data-parameters="q:amateur;sort_by:;from_videos+from_albums:03">03</a></li>
    </ul>
</div>
</div>"#
    }

    fn sample_last_page_html() -> &'static str {
        r#"<div id="list_videos_videos_list_search_result" class="box">
<h1 class="title">Videos for: amateur, Page 3</h1>
<div class="item thumb-item">
    <a class="link js-open-popup" href="https://www.xtits.com/videos/99999/last-video/" title="Last Video" thumb="https://i.xtits.com/thumb.jpg" vthumb="https://www.xtits.com/get_file/5/xyz/99999vthumbs.mp4/">
        <div class="img-holder">
            <span class="label time"><i class="icon-hd"></i>5:00</span>
        </div>
    </a>
</div>
<div class="pagination" id="list_videos_videos_list_search_result_pagination">
    <ul class="pagination-holder">
        <li class="item-pagin"><a class="link" data-parameters="q:amateur;sort_by:;from_videos+from_albums:02">02</a></li>
        <li class="item-pagin active"><span class="link">03</span></li>
    </ul>
</div>
</div>"#
    }

    #[test]
    fn test_parse_search_results() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "Blonde Amateur GF - Amateur");
        assert_eq!(
            results[0].video_url,
            "https://www.xtits.com/videos/50088/blonde-amateur-gf-amateur/"
        );
        assert_eq!(
            results[0].thumbnail_url.as_deref(),
            Some("https://i.xtits.com/contents/videos_screenshots/50000/50088/402x225/2.jpg")
        );
        assert_eq!(results[0].duration, Some(635.0)); // 10:35
    }

    #[test]
    fn test_parse_search_results_second_item() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results[1].title, "Chubby Amateur Fucked - Amateur");
        assert_eq!(results[1].duration, Some(1607.0)); // 26:47
    }

    #[test]
    fn test_parse_search_results_third_item() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results[2].title, "Amateur BBW(2K) - Amateur");
        assert_eq!(results[2].duration, Some(1144.0)); // 19:04
    }

    #[test]
    fn test_parse_search_results_empty() {
        let results = parse_search_results("<html><body></body></html>");
        assert!(results.is_empty());
    }

    #[test]
    fn test_detect_max_page_with_pagination() {
        assert_eq!(detect_max_page(sample_search_html()), 3);
    }

    #[test]
    fn test_detect_max_page_last_page() {
        // Last page: highest link is 03 (active), links point to 02
        assert_eq!(detect_max_page(sample_last_page_html()), 2);
    }

    #[test]
    fn test_detect_max_page_no_pagination() {
        assert_eq!(detect_max_page("<html>no pagination</html>"), 1);
    }

    #[test]
    fn test_has_more_page_1() {
        let max_page = detect_max_page(sample_search_html());
        let current_page = 1_u32;
        assert!(current_page < max_page); // has_more = true
    }

    #[test]
    fn test_has_more_last_page() {
        let max_page = detect_max_page(sample_last_page_html());
        let current_page = 3_u32;
        assert!(current_page >= max_page); // has_more = false
    }

    #[test]
    fn test_search_name() {
        let ext = XTitsExtractor::new();
        assert_eq!(ext.name(), "XTits");
    }

    #[test]
    fn test_supported_filters() {
        let ext = XTitsExtractor::new();
        let filters = SearchExtractor::supported_filters(&ext);
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[1].key, "period");
    }
}
