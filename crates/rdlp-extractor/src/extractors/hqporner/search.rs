//! Search result parsing for HQPorner.
//!
//! Parses search result HTML into `SearchResultPreview` items.
//! Each result card contains a title, video URL, thumbnail, and duration.

use rdlp_core::SearchResultPreview;
use regex::Regex;
use scraper::{Html, Selector};
use std::sync::LazyLock;

use super::parse_duration;

/// Pattern to extract total result count from "1850 HD movies" text.
static TOTAL_COUNT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)\s+HD movies").expect("Valid total count pattern"));

/// Selector for search result title links.
static LINK_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("h3.meta-data-title a").expect("Valid link selector"));

/// Selector for search result thumbnail images.
static THUMB_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a.image img, a.atfib img").expect("Valid thumb selector"));

/// Selector for search result duration spans.
static DURATION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("span.fa-clock-o.meta-data").expect("Valid duration selector")
});

/// Parse search/listing page HTML into search result previews.
///
/// # Arguments
/// * `html` - Raw HTML of the search/listing page.
///
/// # Returns
/// Vector of search result previews extracted from the page.
pub(crate) fn parse_search_results(html: &str) -> Vec<SearchResultPreview> {
    let document = Html::parse_document(html);

    let titles: Vec<_> = document.select(&LINK_SELECTOR).collect();
    let thumbs: Vec<_> = document.select(&THUMB_SELECTOR).collect();
    let durations: Vec<_> = document.select(&DURATION_SELECTOR).collect();

    let mut results = Vec::new();

    for (i, title_el) in titles.iter().enumerate() {
        let href = match title_el.value().attr("href") {
            Some(h) if h.starts_with("/hdporn/") => h,
            _ => continue,
        };

        let title = title_el.text().collect::<String>().trim().to_string();
        if title.is_empty() {
            continue;
        }

        let video_url = format!("https://hqporner.com{href}");

        let thumbnail_url = thumbs.get(i).and_then(|el| {
            el.value().attr("src").map(|s| {
                if s.starts_with("//") {
                    format!("https:{s}")
                } else {
                    s.to_string()
                }
            })
        });

        let duration = durations.get(i).and_then(|el| {
            let text = el.text().collect::<String>();
            parse_duration(&text)
        });

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

/// Extract the total result count from "N HD movies" text on the page.
pub(crate) fn extract_total_count(html: &str) -> Option<u64> {
    TOTAL_COUNT_PATTERN
        .captures(html)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Check whether the page has a "Next" pagination link.
pub(crate) fn has_next_page(html: &str) -> bool {
    html.contains(">Next<")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_search_html() -> &'static str {
        r#"<html><body>
<li class="icon fa-clock-o">1850 HD movies</li>
<div class="4u">
    <section class="box feature">
        <a href="/hdporn/81203-full_body_massage.html" class="image featured atfib">
            <div><img id="cover_81203" src="//fastporndelivery.hqporner.com/imgs/ab/cd/main.jpg" alt="full body massage" /></div>
        </a>
        <div id="span-case">
            <h3 class="meta-data-title"><a href="/hdporn/81203-full_body_massage.html" class="click-trigger">full body massage</a></h3>
            <span class="icon fa-clock-o meta-data">26m 52s</span>
        </div>
    </section>
</div>
<div class="4u">
    <section class="box feature">
        <a href="/hdporn/81221-same_sex_oral_massage.html" class="image featured atfib">
            <div><img id="cover_81221" src="//fastporndelivery.hqporner.com/imgs/ef/gh/main.jpg" alt="same sex oral massage" /></div>
        </a>
        <div id="span-case">
            <h3 class="meta-data-title"><a href="/hdporn/81221-same_sex_oral_massage.html" class="click-trigger">same sex oral massage</a></h3>
            <span class="icon fa-clock-o meta-data">18m 28s</span>
        </div>
    </section>
</div>
<ul class="actions pagination">
<li><span class="pagi-btn-alt">1</span></li>
<li><a href="/?q=massage&p=2" class="pagi-btn">2</a></li>
<li><a href="/?q=massage&p=2" class="pagi-btn">Next</a></li>
</ul>
</body></html>"#
    }

    fn sample_last_page_html() -> &'static str {
        r#"<html><body>
<li class="icon fa-clock-o">50 HD movies</li>
<div class="4u">
    <section class="box feature">
        <a href="/hdporn/123-test.html" class="image featured atfib">
            <div><img src="//fastporndelivery.hqporner.com/imgs/xx/yy/main.jpg" /></div>
        </a>
        <div id="span-case">
            <h3 class="meta-data-title"><a href="/hdporn/123-test.html" class="click-trigger">test video</a></h3>
            <span class="icon fa-clock-o meta-data">5m 30s</span>
        </div>
    </section>
</div>
<ul class="actions pagination">
<li><a href="/?q=test&p=1" class="pagi-btn">Prev</a></li>
<li><span class="pagi-btn-alt">2</span></li>
</ul>
</body></html>"#
    }

    #[test]
    fn test_parse_search_results() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "full body massage");
        assert_eq!(
            results[0].video_url,
            "https://hqporner.com/hdporn/81203-full_body_massage.html"
        );
        assert_eq!(results[0].duration, Some(1612.0));
        assert!(
            results[0]
                .thumbnail_url
                .as_ref()
                .unwrap()
                .starts_with("https://")
        );
    }

    #[test]
    fn test_parse_search_results_second_item() {
        let results = parse_search_results(sample_search_html());
        assert_eq!(results[1].title, "same sex oral massage");
        assert_eq!(results[1].duration, Some(1108.0));
    }

    #[test]
    fn test_parse_search_results_empty() {
        let results = parse_search_results("<html><body></body></html>");
        assert!(results.is_empty());
    }

    #[test]
    fn test_extract_total_count() {
        assert_eq!(extract_total_count("1850 HD movies"), Some(1850));
        assert_eq!(extract_total_count("no count here"), None);
    }

    #[test]
    fn test_has_next_page_true() {
        assert!(has_next_page(sample_search_html()));
    }

    #[test]
    fn test_has_next_page_false() {
        assert!(!has_next_page(sample_last_page_html()));
    }
}
