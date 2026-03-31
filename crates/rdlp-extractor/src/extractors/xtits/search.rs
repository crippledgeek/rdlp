//! Search result parsing and SearchExtractor implementation for XTits.
//!
//! XTits uses KVS AJAX pagination. All pages (including page 1) are fetched
//! from the same async endpoint that returns HTML fragments.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{SearchPageResponse, SearchQuery, SearchResultPreview};
use std::time::Duration;

use super::XTitsExtractor;
use super::search_patterns;
use crate::base::common::BaseExtractor;

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
        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut page = 1_u32;

        loop {
            let page_url = search_patterns::build_search_url(query, page);
            let sanitized = rdlp_security::sanitize_for_logging(&page_url);
            debug!("[XTits] Fetching search page {page}: {sanitized}");

            let webpage = BaseExtractor::fetch_webpage_with_headers(
                &page_url,
                &[
                    ("X-Requested-With", "XMLHttpRequest"),
                    ("Referer", "https://www.xtits.com/search/"),
                ],
                ctx,
            )
            .await?;

            let page_results = parse_search_results(&webpage);
            if page_results.is_empty() {
                break;
            }

            let max_page = detect_max_page(&webpage);
            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if page >= max_page {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(search_patterns::PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(
            "[XTits] Search complete: {} results across {page} pages",
            all_results.len()
        );
        Ok(all_results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let page = query.page.unwrap_or(1);
        let page_url = search_patterns::build_search_url(query, page);

        let webpage = BaseExtractor::fetch_webpage_with_headers(
            &page_url,
            &[
                ("X-Requested-With", "XMLHttpRequest"),
                ("Referer", "https://www.xtits.com/search/"),
            ],
            ctx,
        )
        .await?;

        let page_results = parse_search_results(&webpage);
        let max_page = detect_max_page(&webpage);
        let has_more = page < max_page && !page_results.is_empty();
        let total_estimate = Some(max_page as u64 * search_patterns::RESULTS_PER_PAGE);

        Ok(SearchPageResponse {
            results: page_results,
            page,
            has_more,
            total_estimate,
        })
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
