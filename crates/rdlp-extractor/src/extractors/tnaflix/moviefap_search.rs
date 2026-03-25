//! HTML search result parsing for MovieFap.
//!
//! Provides scraper-based extraction of search result thumbnails,
//! pagination detection, and filter validation.
//!
//! ## Real HTML Structure (as of 2026-02)
//!
//! Each video item is a `<div class="videothumb">` element (note: `<a>` with the
//! same class name also exists — selectors must be specific to `div`):
//!
//! ```html
//! <div class="videothumb">
//!   <span class="thumbtitle">
//!     <a href="https://www.moviefap.com/videos/{hex_id}/{title}.html" title="Title">Title</a>
//!   </span>
//!   <a href="..." class="videothumb">
//!     <img src="https://imgh.moviefap.com/w162h122/..." alt="Title" />
//!   </a>
//!   <div class="videoleft">21:00<br />3 years ago</div>
//!   <div class="videoright">
//!     <div class="rating">...</div>
//!   </div>
//! </div>
//! ```
//!
//! ## Pagination
//!
//! ```html
//! <div class="pagination">
//!   <span class="current">1</span>
//!   <a href="/search/query/sort/2">2</a>
//!   ...
//! </div>
//! ```

use rdlp_core::{RdlpError, Result};
use rdlp_types::{SearchFilter, SearchResultPreview};
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::sync::LazyLock;

use super::moviefap_search_patterns;

/// Container for each video result: `<div class="videothumb">`.
/// We target only `div` elements to avoid matching the `<a class="videothumb">` anchor.
static VIDEO_ITEM_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("div.videothumb").expect("Valid MovieFap video item selector")
});

/// Title anchor inside `.thumbtitle`: `<span class="thumbtitle"> <a href="...">Title</a> </span>`.
static THUMB_TITLE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(".thumbtitle a[href]").expect("Valid MovieFap title selector")
});

/// Thumbnail image inside the `<a class="videothumb">` anchor.
static THUMB_IMG_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("a.videothumb img").expect("Valid MovieFap thumbnail img selector")
});

/// Duration/upload-date container: `<div class="videoleft">`.
static VIDEO_LEFT_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".videoleft").expect("Valid MovieFap videoleft selector"));

/// Pagination links: `<div class="pagination"> <a href="...">N</a> </div>`.
static PAGINATION_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".pagination a").expect("Valid MovieFap pagination selector"));

/// Parse search results from a MovieFap search results HTML page.
///
/// Returns a `Vec` of [`SearchResultPreview`] items; an empty vec indicates
/// no results were found.
///
/// # Arguments
/// * `html` - Raw HTML string of the search results page.
pub(crate) fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let document = Html::parse_document(html);

    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for item in document.select(&VIDEO_ITEM_SELECTOR) {
        // Title and video URL from `.thumbtitle a[href]`
        let title_link = item.select(&THUMB_TITLE_SELECTOR).next();

        let video_url = title_link
            .and_then(|a| a.value().attr("href"))
            .filter(|href| !href.is_empty())
            .map(str::to_string);

        let Some(video_url) = video_url else {
            continue;
        };

        // Deduplicate by URL
        if !seen_urls.insert(video_url.clone()) {
            continue;
        }

        // Title text from the link
        let title = title_link
            .map(|a| a.text().collect::<String>())
            .map(|s| s.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Unknown".to_string());

        // Thumbnail URL from `a.videothumb img` — prefer `src` attribute
        let thumbnail_url = item
            .select(&THUMB_IMG_SELECTOR)
            .next()
            .and_then(|img| img.value().attr("src"))
            .filter(|url| !url.is_empty())
            .map(str::to_string);

        // Duration from `.videoleft` — first text node before the `<br>` (e.g. "21:00")
        let (duration, _upload_date) = parse_video_left(&item);

        results.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration,
            view_count: None,
            upload_date: None,
        });
    }

    results
}

/// Detect the maximum page number available from a MovieFap pagination bar.
///
/// Returns `None` when no pagination links are found (single-page results).
///
/// # Arguments
/// * `html` - Raw HTML string of the search results page.
pub(crate) fn parse_pagination(html: &str) -> Option<usize> {
    let document = Html::parse_document(html);
    document
        .select(&PAGINATION_SELECTOR)
        .filter_map(|a| a.text().collect::<String>().trim().parse::<usize>().ok())
        .max()
}

/// Validate that all supplied search filters are recognised MovieFap keys and values.
///
/// # Arguments
/// * `filters` - Slice of [`SearchFilter`] items to validate.
///
/// # Errors
/// Returns [`RdlpError::Extraction`] if an unrecognised key or value is encountered.
pub(crate) fn validate_search_filters(filters: &[SearchFilter]) -> Result<()> {
    for filter in filters {
        match filter.key.as_str() {
            "ordering" => {
                if !moviefap_search_patterns::VALID_ORDERINGS.contains(&filter.value.as_str()) {
                    return Err(RdlpError::Extraction(format!(
                        "Invalid MovieFap ordering value '{}'. Valid values: {}",
                        filter.value,
                        moviefap_search_patterns::VALID_ORDERINGS.join(", ")
                    )));
                }
            }
            other => {
                return Err(RdlpError::Extraction(format!(
                    "Unknown MovieFap search filter key '{other}'"
                )));
            }
        }
    }

    Ok(())
}

/// Parse the `.videoleft` element to extract duration and upload date strings.
///
/// The element contains text like `"21:00\n3 years ago"` where the first
/// part is the duration and the second part is the upload date. The two
/// pieces of text are separated by a `<br>` element.
///
/// Returns `(duration_secs, upload_date_string)` — either can be `None` when
/// the text is absent or unparseable.
fn parse_video_left(item: &scraper::ElementRef<'_>) -> (Option<f64>, Option<String>) {
    let video_left = item.select(&VIDEO_LEFT_SELECTOR).next();
    let Some(el) = video_left else {
        return (None, None);
    };

    // Collect all text nodes; the first is duration, the second is date
    let texts: Vec<String> = el
        .text()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let duration = texts.first().and_then(|s| parse_duration_secs(s));
    let upload_date = texts.get(1).cloned();

    (duration, upload_date)
}

/// Parse a duration string like `"21:00"` or `"1:23:45"` into seconds.
fn parse_duration_secs(s: &str) -> Option<f64> {
    let parts: Vec<u64> = s
        .split(':')
        .map(|p| p.trim().parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;

    let secs = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };

    Some(secs as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal MovieFap search results page.
    fn make_search_html(items: &[&str], pagination_html: Option<&str>) -> String {
        let items_html = items.join("\n");
        let pagination = pagination_html.unwrap_or("");
        format!(
            r#"<html><body>
            <div id="searchresults">
                {items_html}
            </div>
            {pagination}
            </body></html>"#
        )
    }

    /// Build a single video item matching the real MovieFap HTML structure.
    fn sample_item(url: &str, title: &str, duration: &str, thumb: &str) -> String {
        format!(
            r#"<div class="videothumb">
                <span class="thumbtitle">
                    <a href="{url}" title="{title}">{title}</a>
                </span>
                <a href="{url}" class="videothumb">
                    <img src="{thumb}" alt="{title}" />
                </a>
                <div class="videoleft">{duration}<br />3 years ago</div>
            </div>"#
        )
    }

    #[test]
    fn test_parse_search_results_basic() {
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test-video.html",
            "Test Video",
            "21:00",
            "https://imgh.moviefap.com/w162h122/thumb.jpg",
        );
        let html = make_search_html(&[&item], None);
        let results = parse_search_results(&html);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Video");
        assert_eq!(
            results[0].video_url,
            "https://www.moviefap.com/videos/abc123/test-video.html"
        );
    }

    #[test]
    fn test_parse_search_results_thumbnail() {
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test.html",
            "Test",
            "05:00",
            "https://imgh.moviefap.com/w162h122/thumb.jpg",
        );
        let html = make_search_html(&[&item], None);
        let results = parse_search_results(&html);

        assert_eq!(
            results[0].thumbnail_url,
            Some("https://imgh.moviefap.com/w162h122/thumb.jpg".to_string())
        );
    }

    #[test]
    fn test_parse_search_results_duration_mm_ss() {
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test.html",
            "Test",
            "21:00",
            "thumb.jpg",
        );
        let html = make_search_html(&[&item], None);
        let results = parse_search_results(&html);
        assert_eq!(results[0].duration, Some(1260.0));
    }

    #[test]
    fn test_parse_search_results_duration_hh_mm_ss() {
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test.html",
            "Test",
            "1:23:45",
            "thumb.jpg",
        );
        let html = make_search_html(&[&item], None);
        let results = parse_search_results(&html);
        assert_eq!(results[0].duration, Some(5025.0));
    }

    #[test]
    fn test_parse_search_results_multiple() {
        let item1 = sample_item(
            "https://www.moviefap.com/videos/aaa/title-1.html",
            "Title 1",
            "01:00",
            "thumb1.jpg",
        );
        let item2 = sample_item(
            "https://www.moviefap.com/videos/bbb/title-2.html",
            "Title 2",
            "02:00",
            "thumb2.jpg",
        );
        let html = make_search_html(&[&item1, &item2], None);
        let results = parse_search_results(&html);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Title 1");
        assert_eq!(results[1].title, "Title 2");
    }

    #[test]
    fn test_parse_search_results_empty() {
        let html = make_search_html(&[], None);
        let results = parse_search_results(&html);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_search_results_deduplicates_by_url() {
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test.html",
            "Same Video",
            "01:00",
            "thumb.jpg",
        );
        let html = make_search_html(&[&item, &item, &item], None);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Same Video");
    }

    #[test]
    fn test_parse_search_results_no_view_count() {
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test.html",
            "Test",
            "05:00",
            "thumb.jpg",
        );
        let html = make_search_html(&[&item], None);
        let results = parse_search_results(&html);
        // MovieFap search results don't include view counts
        assert_eq!(results[0].view_count, None);
    }

    #[test]
    fn test_parse_search_results_upload_date_not_in_preview() {
        // upload_date is currently always None in the preview (not surfaced)
        let item = sample_item(
            "https://www.moviefap.com/videos/abc123/test.html",
            "Test",
            "05:00",
            "thumb.jpg",
        );
        let html = make_search_html(&[&item], None);
        let results = parse_search_results(&html);
        // upload_date field is not populated from search results
        assert_eq!(results[0].upload_date, None);
    }

    #[test]
    fn test_parse_pagination() {
        let pagination = r#"<div class="pagination">
            <span class="current">1</span>
            <a href="/search/query/relevance/2">2</a>
            <a href="/search/query/relevance/3">3</a>
            <a href="/search/query/relevance/4">4</a>
        </div>"#;
        let html = make_search_html(&[], Some(pagination));
        let max_pages = parse_pagination(&html);
        assert_eq!(max_pages, Some(4));
    }

    #[test]
    fn test_parse_pagination_no_pager() {
        let html = "<html><body><p>No pagination</p></body></html>";
        let max_pages = parse_pagination(html);
        assert_eq!(max_pages, None);
    }

    #[test]
    fn test_parse_pagination_single_page() {
        let pagination = r#"<div class="pagination">
            <span class="current">1</span>
        </div>"#;
        let html = make_search_html(&[], Some(pagination));
        let max_pages = parse_pagination(&html);
        // Only a <span> with no numeric <a> links — returns None
        assert_eq!(max_pages, None);
    }

    #[test]
    fn test_validate_search_filters_valid() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "adddate".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_ok());
    }

    #[test]
    fn test_validate_search_filters_all_valid_orderings() {
        for ordering in ["relevance", "adddate", "viewnum", "rate", "duration"] {
            let filters = vec![SearchFilter {
                key: "ordering".to_string(),
                value: ordering.to_string(),
            }];
            assert!(
                validate_search_filters(&filters).is_ok(),
                "ordering '{ordering}' should be valid"
            );
        }
    }

    #[test]
    fn test_validate_search_filters_invalid_value() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "invalid_value".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    #[test]
    fn test_validate_search_filters_unknown_key() {
        let filters = vec![SearchFilter {
            key: "unknown_key".to_string(),
            value: "value".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    #[test]
    fn test_validate_search_filters_empty() {
        assert!(validate_search_filters(&[]).is_ok());
    }

    #[test]
    fn test_parse_duration_secs_mm_ss() {
        assert_eq!(parse_duration_secs("21:00"), Some(1260.0));
        assert_eq!(parse_duration_secs("05:30"), Some(330.0));
    }

    #[test]
    fn test_parse_duration_secs_hh_mm_ss() {
        assert_eq!(parse_duration_secs("1:23:45"), Some(5025.0));
    }

    #[test]
    fn test_parse_duration_secs_invalid() {
        assert_eq!(parse_duration_secs("not-a-duration"), None);
        assert_eq!(parse_duration_secs(""), None);
    }
}
