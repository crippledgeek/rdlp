//! HTML search result parsing for TNAFlix.
//!
//! Provides scraper-based extraction of search result thumbnails,
//! pagination detection, and filter validation.
//!
//! ## Real HTML Structure (as of 2026-02)
//!
//! Each video item is a Bootstrap grid column with `data-vid` attribute:
//! ```html
//! <div data-vid="123" class="col-xs-6 col-md-4 col-xl-3 mb-3">
//!   <a class="thumb video-thumb bg-dark" href="/category/title/video123">
//!     <img class="lazyload" data-src="thumb.jpg" />
//!     <div class="thumb-icon video-duration">12:34</div>
//!   </a>
//!   <a class="video-title text-break" href="...">Title</a>
//!   <div class="d-flex">
//!     <div class="text-small d-flex">
//!       <div><i class="icon-eye"></i>1.2K</div>
//!     </div>
//!   </div>
//! </div>
//! ```

use rdlp_core::{RdlpError, Result};
use rdlp_types::{SearchFilter, SearchResultPreview};
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::sync::LazyLock;

use super::search_patterns;

/// Video item container: `<div data-vid="...">` grid columns.
static VIDEO_ITEM_SELECTOR: LazyLock<Selector> = crate::static_selector!("div[data-vid]");

/// Thumbnail anchor: `<a class="video-thumb">` inside each item.
static VIDEO_THUMB_SELECTOR: LazyLock<Selector> = crate::static_selector!("a.video-thumb");

/// Title anchor: `<a class="video-title">` with text content as title.
static VIDEO_TITLE_SELECTOR: LazyLock<Selector> = crate::static_selector!("a.video-title");

/// Duration overlay: `<div class="video-duration">`.
static DURATION_SELECTOR: LazyLock<Selector> = crate::static_selector!(".video-duration");

/// View count icon: `<i class="icon-eye">` — views text is in the parent div.
static VIEWS_ICON_SELECTOR: LazyLock<Selector> = crate::static_selector!("i.icon-eye");

/// Uploader: `<a class="badge ..." href="/profile/{user}">` inside each
/// card. The first matching anchor's trimmed text is the display name.
static UPLOADER_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"a.badge[href*="/profile/"]"#);

/// Pagination links: Bootstrap `.pagination .page-link` anchors.
static PAGE_LINK_SELECTOR: LazyLock<Selector> = crate::static_selector!(".pagination a.page-link");

/// `<img>` elements (used to find lazy-loaded thumbnails and alt-text titles).
static IMG_SELECTOR: LazyLock<Selector> = crate::static_selector!("img");

/// Parse search results from a TNAFlix search results HTML page.
///
/// Returns a `Vec` of [`SearchResultPreview`] items; an empty vec indicates
/// no results were found.
///
/// # Arguments
/// * `html` - Raw HTML string of the search results page.
pub fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let document = Html::parse_document(html);

    let mut results = Vec::new();
    let mut seen_urls = HashSet::new();

    for item in document.select(&VIDEO_ITEM_SELECTOR) {
        // Video URL from the thumbnail anchor (<a class="video-thumb">)
        let thumb_link = item.select(&VIDEO_THUMB_SELECTOR).next();

        let video_url = thumb_link
            .and_then(|a| a.value().attr("href"))
            .filter(|href| !href.is_empty())
            .map(str::to_string);

        let Some(video_url) = video_url else {
            continue;
        };

        // Category pages repeat the same videos in multiple page sections;
        // deduplicate by URL so each video appears only once.
        if !seen_urls.insert(video_url.clone()) {
            continue;
        }

        // Title from the separate title anchor (<a class="video-title">)
        // Falls back to the img alt attribute or thumbnail anchor title
        let title = item
            .select(&VIDEO_TITLE_SELECTOR)
            .next()
            .map(|a| a.text().collect::<String>())
            .map(|s| s.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                thumb_link
                    .and_then(|a| a.select(&IMG_SELECTOR).next())
                    .and_then(|img| img.value().attr("alt"))
                    .filter(|a| !a.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Unknown".to_string());

        // Thumbnail URL from <img> inside the thumb link (prefers data-src for lazy-loaded)
        let thumbnail_url = thumb_link
            .and_then(|a| a.select(&IMG_SELECTOR).next())
            .and_then(|img| {
                img.value()
                    .attr("data-src")
                    .or_else(|| img.value().attr("src"))
            })
            .filter(|url| !url.contains("placeholder"))
            .map(str::to_string);

        // Duration from <div class="video-duration">
        let duration = item
            .select(&DURATION_SELECTOR)
            .next()
            .map(|el| el.text().collect::<String>())
            .and_then(|s| parse_duration_secs(s.trim()));

        // View count: text next to <i class="icon-eye"> (e.g. "11.7K")
        let view_count = item
            .select(&VIEWS_ICON_SELECTOR)
            .next()
            .and_then(|icon| icon.parent())
            .and_then(scraper::ElementRef::wrap)
            .map(|el| el.text().collect::<String>())
            .and_then(|s| crate::base::common::BaseExtractor::parse_human_count(s.trim()));

        // Uploader: first /profile/ link inside the card.
        let uploader = item
            .select(&UPLOADER_SELECTOR)
            .next()
            .map(|a| a.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty());

        results.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration,
            uploader,
            uploader_url: None,
            actors: vec![],
            view_count,
            upload_date: None,
        });
    }

    results
}

/// Detect the maximum page number available from a TNAFlix pagination bar.
///
/// Returns `None` when no pagination links are found (single-page results).
///
/// # Arguments
/// * `html` - Raw HTML string of the search results page.
pub fn parse_pagination(html: &str) -> Option<usize> {
    let document = Html::parse_document(html);
    document
        .select(&PAGE_LINK_SELECTOR)
        .filter_map(|a| a.text().collect::<String>().trim().parse::<usize>().ok())
        .max()
}

/// Validate that all supplied search filters are recognised TNAFlix keys and values.
///
/// # Arguments
/// * `filters` - Slice of [`SearchFilter`] items to validate.
///
/// # Errors
/// Returns [`RdlpError::Extraction`] if an unrecognised key or value is encountered.
pub fn validate_search_filters(filters: &[SearchFilter]) -> Result<()> {
    const VALID_ORDERINGS: &[&str] = &["featured", "newest", "duration", "rating"];

    for filter in filters {
        match filter.key.as_str() {
            "ordering" => {
                if !VALID_ORDERINGS.contains(&filter.value.as_str()) {
                    return Err(RdlpError::Extraction {
                        message: format!(
                            "Invalid TNAFlix ordering value '{}'. Valid values: {}",
                            filter.value,
                            VALID_ORDERINGS.join(", ")
                        ),
                        url: None,
                    });
                }
            }
            "category" => {
                if !search_patterns::is_valid_category(&filter.value) {
                    return Err(RdlpError::Extraction {
                        message: format!(
                            "Invalid TNAFlix category '{}'. Use search_filters() to see valid values.",
                            filter.value
                        ),
                        url: None,
                    });
                }
            }
            other => {
                return Err(RdlpError::Extraction {
                    message: format!("Unknown TNAFlix search filter key '{other}'"),
                    url: None,
                });
            }
        }
    }

    Ok(())
}

/// Parse a duration string like `"12:34"` or `"1:23:45"` into seconds.
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

    /// Build a test HTML page mimicking the real TNAFlix search layout.
    fn make_search_html(items: &[&str], with_pagination: bool) -> String {
        let items_html = items.join("\n");
        let pagination = if with_pagination {
            r#"<ul class="pagination justify-content-center mt-4">
                <li class="page-item active"><span class="page-link">1</span></li>
                <li class="page-item"><a class="page-link" href="?page=2">2</a></li>
                <li class="page-item"><a class="page-link" href="?page=3">3</a></li>
            </ul>"#
        } else {
            ""
        };
        format!(
            r#"<html><body>
            <div class="row">
                {items_html}
            </div>
            {pagination}
            </body></html>"#
        )
    }

    /// Build a single video item matching the real TNAFlix HTML structure.
    fn sample_item(url: &str, title: &str, duration: &str, views: &str) -> String {
        format!(
            r#"<div data-vid="123" class="col-xs-6 col-md-4 col-xl-3 mb-3">
                <a class="thumb video-thumb bg-dark" href="{url}">
                    <img class="lazyload" data-src="thumb.jpg"
                         alt="{title}" width="300" height="150" />
                    <div class="thumb-icon video-duration">{duration}</div>
                </a>
                <a href="{url}" class="video-title text-break">{title}</a>
                <div class="d-flex">
                    <div class="text-small d-flex">
                        <div><i class="icon-eye"></i>{views}</div>
                    </div>
                </div>
            </div>"#
        )
    }

    /// Regression: prior to this fix every result had `uploader = None`
    /// because the card-level `/profile/` link was not extracted. The
    /// live fixture (captured 2026-04-28 from `/search.php?what=teen`)
    /// has 60 cards, most carrying a `<a class="badge..." href="/profile/...">`
    /// link.
    #[test]
    fn parse_search_results_extracts_uploader_from_live_fixture() {
        const LIVE: &str = include_str!("tests/tnaflix_search_live.html");
        let results = parse_search_results(LIVE);
        assert!(!results.is_empty(), "live fixture should yield results");

        let with_uploader = results.iter().filter(|r| r.uploader.is_some()).count();
        assert!(
            with_uploader >= results.len() / 2,
            "expected most rows to carry uploader; got {with_uploader}/{}",
            results.len()
        );

        // Sanity: uploader text must not contain HTML or whitespace artefacts.
        for u in results.iter().filter_map(|r| r.uploader.as_deref()) {
            assert!(!u.contains('<'), "uploader contains HTML: {u:?}");
            assert_eq!(u, u.trim(), "uploader has unstripped whitespace: {u:?}");
        }
    }

    #[test]
    fn test_parse_search_results_basic() {
        let item = sample_item(
            "https://www.tnaflix.com/category/title/video123",
            "Test Video",
            "12:34",
            "1,234",
        );
        let html = make_search_html(&[&item], false);
        let results = parse_search_results(&html);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Video");
        assert_eq!(
            results[0].video_url,
            "https://www.tnaflix.com/category/title/video123"
        );
    }

    #[test]
    fn test_parse_search_results_empty_no_items() {
        let html = make_search_html(&[], false);
        let results = parse_search_results(&html);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_search_results_duration() {
        let item = sample_item(
            "https://www.tnaflix.com/category/title/video123",
            "Test",
            "1:23:45",
            "0",
        );
        let html = make_search_html(&[&item], false);
        let results = parse_search_results(&html);
        assert_eq!(results[0].duration, Some(5025.0));
    }

    #[test]
    fn test_parse_search_results_multiple() {
        let item1 = sample_item(
            "https://www.tnaflix.com/a/b/video1",
            "Title 1",
            "01:00",
            "100",
        );
        let item2 = sample_item(
            "https://www.tnaflix.com/a/b/video2",
            "Title 2",
            "02:00",
            "200",
        );
        let html = make_search_html(&[&item1, &item2], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Title 1");
        assert_eq!(results[1].title, "Title 2");
    }

    #[test]
    fn test_parse_search_results_deduplicates_by_url() {
        let item = sample_item(
            "https://www.tnaflix.com/a/b/video1",
            "Same Video",
            "01:00",
            "100",
        );
        // Simulate category pages that repeat the same items in multiple sections
        let html = make_search_html(&[&item, &item, &item], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Same Video");
    }

    #[test]
    fn test_parse_search_results_thumbnail() {
        let item = sample_item("https://www.tnaflix.com/a/b/video1", "Test", "01:00", "100");
        let html = make_search_html(&[&item], false);
        let results = parse_search_results(&html);
        assert_eq!(results[0].thumbnail_url, Some("thumb.jpg".to_string()));
    }

    #[test]
    fn test_parse_search_results_view_count() {
        let item = sample_item(
            "https://www.tnaflix.com/a/b/video1",
            "Test",
            "01:00",
            "11.7K",
        );
        let html = make_search_html(&[&item], false);
        let results = parse_search_results(&html);
        assert_eq!(results[0].view_count, Some(11700));
    }

    #[test]
    fn test_parse_search_results_title_from_alt_fallback() {
        // Item where video-title anchor is missing, falls back to img alt
        let item = r#"<div data-vid="456" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="thumb video-thumb bg-dark" href="https://www.tnaflix.com/a/b/video456">
                <img alt="Alt Title" data-src="thumb.jpg" />
                <div class="thumb-icon video-duration">05:00</div>
            </a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Alt Title");
    }

    #[test]
    fn test_parse_pagination() {
        let html = make_search_html(&[], true);
        let max_pages = parse_pagination(&html);
        assert_eq!(max_pages, Some(3));
    }

    #[test]
    fn test_parse_pagination_no_pager() {
        let html = "<html><body><p>No pagination</p></body></html>";
        let max_pages = parse_pagination(html);
        assert_eq!(max_pages, None);
    }

    #[test]
    fn test_validate_search_filters_valid() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "newest".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_ok());
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
        assert_eq!(parse_duration_secs("12:34"), Some(754.0));
    }

    #[test]
    fn test_parse_duration_secs_hh_mm_ss() {
        assert_eq!(parse_duration_secs("1:23:45"), Some(5025.0));
    }

    #[test]
    fn test_parse_duration_secs_invalid() {
        assert_eq!(parse_duration_secs("not-a-duration"), None);
    }

    // View-count parsing is covered by the canonical
    // `BaseExtractor::parse_human_count` tests in `base::common::tests`.

    #[test]
    fn test_validate_search_filters_valid_category() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "teen-porn".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_ok());
    }

    #[test]
    fn test_validate_search_filters_valid_category_section() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "new".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_ok());
    }

    #[test]
    fn test_validate_search_filters_invalid_category() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "not-a-real-category".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    #[test]
    fn test_validate_search_filters_category_and_ordering() {
        let filters = vec![
            SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            },
            SearchFilter {
                key: "category".to_string(),
                value: "milf-porn".to_string(),
            },
        ];
        assert!(validate_search_filters(&filters).is_ok());
    }

    // ---- Negative tests ----

    #[test]
    fn test_parse_results_missing_thumb_link() {
        // Item with no a.video-thumb → should skip
        let item = r#"<div data-vid="789" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="video-title text-break" href="https://tnaflix.com/video/123">Title</a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_results_empty_href() {
        // Thumb link with empty href → should skip
        let item = r#"<div data-vid="789" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="thumb video-thumb bg-dark" href="">
                <img data-src="thumb.jpg" alt="Test" />
            </a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_results_missing_img_thumbnail_none() {
        // No img inside thumb link → thumbnail should be None
        let item = r#"<div data-vid="789" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="thumb video-thumb bg-dark" href="https://tnaflix.com/v/test/video789">
            </a>
            <a class="video-title text-break" href="https://tnaflix.com/v/test/video789">Test</a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].thumbnail_url, None);
    }

    #[test]
    fn test_parse_results_placeholder_img_filtered() {
        // Placeholder image src → thumbnail should be None
        let item = r#"<div data-vid="789" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="thumb video-thumb bg-dark" href="https://tnaflix.com/v/test/video789">
                <img src="/assets/img/video_cover_placeholder.jpg" alt="Test" />
            </a>
            <a class="video-title text-break" href="https://tnaflix.com/v/test/video789">Test</a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].thumbnail_url, None);
    }

    #[test]
    fn test_parse_results_missing_duration() {
        // No .video-duration div → duration should be None
        let item = r#"<div data-vid="789" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="thumb video-thumb bg-dark" href="https://tnaflix.com/v/test/video789">
                <img data-src="thumb.jpg" alt="Test" />
            </a>
            <a class="video-title text-break" href="https://tnaflix.com/v/test/video789">Test</a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].duration, None);
    }

    #[test]
    fn test_parse_results_missing_views() {
        // No icon-eye element → view_count should be None
        let item = r#"<div data-vid="789" class="col-xs-6 col-md-4 col-xl-3 mb-3">
            <a class="thumb video-thumb bg-dark" href="https://tnaflix.com/v/test/video789">
                <img data-src="thumb.jpg" alt="Test" />
                <div class="thumb-icon video-duration">05:00</div>
            </a>
            <a class="video-title text-break" href="https://tnaflix.com/v/test/video789">Test</a>
        </div>"#;
        let html = make_search_html(&[item], false);
        let results = parse_search_results(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].view_count, None);
    }

    #[test]
    fn test_parse_results_malformed_html_no_panic() {
        let html = "<div data-vid=\"123\" class=\"col\"><a class=\"video-thumb\" href=\"";
        let results = parse_search_results(html);
        let _ = results; // Should not panic
    }

    #[test]
    fn test_parse_results_empty_html() {
        assert!(parse_search_results("").is_empty());
    }

    #[test]
    fn test_parse_results_no_data_vid_divs() {
        let html = "<html><body><p>No results</p></body></html>";
        assert!(parse_search_results(html).is_empty());
    }

    #[test]
    fn test_parse_pagination_next_link_ignored() {
        let html = r#"<html><body>
            <ul class="pagination justify-content-center mt-4">
                <li class="page-item active"><span class="page-link">1</span></li>
                <li class="page-item"><a class="page-link" href="?page=2">2</a></li>
                <li class="page-item"><a class="page-link" href="?page=2">Next</a></li>
            </ul>
        </body></html>"#;
        // "Next" is not numeric → should be ignored, max = 2
        assert_eq!(parse_pagination(html), Some(2));
    }

    #[test]
    fn test_parse_pagination_empty_html() {
        assert_eq!(parse_pagination(""), None);
    }

    #[test]
    fn test_validate_filters_empty_key() {
        let filters = vec![SearchFilter {
            key: String::new(),
            value: "newest".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    #[test]
    fn test_validate_filters_empty_ordering_value() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: String::new(),
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    #[test]
    fn test_validate_filters_case_sensitive_ordering() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "Newest".to_string(), // uppercase N
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    // ---- Additional negative tests (round 2) ----

    #[test]
    fn test_parse_duration_four_parts() {
        // "1:2:3:4" — too many parts for TNAFlix parser
        assert_eq!(parse_duration_secs("1:2:3:4"), None);
    }

    #[test]
    fn test_parse_duration_single_number() {
        assert_eq!(parse_duration_secs("123"), None);
    }

    #[test]
    fn test_parse_duration_zero() {
        assert_eq!(parse_duration_secs("0:00"), Some(0.0));
    }

    #[test]
    fn test_parse_duration_leading_zeros() {
        assert_eq!(parse_duration_secs("01:02:03"), Some(3723.0));
    }

    #[test]
    fn test_validate_filters_empty_category_value() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: String::new(),
        }];
        assert!(validate_search_filters(&filters).is_err());
    }

    #[test]
    fn test_validate_filters_multiple_different_keys() {
        let filters = vec![
            SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            },
            SearchFilter {
                key: "category".to_string(),
                value: "teen-porn".to_string(),
            },
        ];
        assert!(validate_search_filters(&filters).is_ok());
    }
}
