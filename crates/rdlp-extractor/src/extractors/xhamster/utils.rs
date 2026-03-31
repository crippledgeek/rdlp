//! Utility functions for xHamster extractor.
//!
//! Error detection, metadata extraction from `videoModel` JSON,
//! and legacy HTML fallback helpers.

use log::debug;
use rdlp_types::InfoDict;
use regex::Regex;
use scraper::Html;
use serde_json::Value;
use std::sync::LazyLock;

use crate::base::common::BaseExtractor;

use super::patterns::VIDEO_CLOSED_PATTERN;

/// Pattern to extract RTA age verification meta tag content.
static RTA_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+name=["']rating["']\s+content=["']RTA-5042-1996-1400-1577-RTA["']"#)
        .expect("Valid RTA pattern")
});

// --- Legacy HTML fallback patterns ---

static LEGACY_TITLE_PATTERNS: [&str; 3] = [
    r"<h1[^>]*>([^<]+)</h1>",
    r#"<meta[^>]+itemprop=".*?caption.*?"[^>]+content="(.+?)""#,
    r"<title[^>]*>(.+?)(?:,\s*[^,]*?\s*Porn\s*[^,]*?:\s*xHamster[^<]*| - xHamster\.com)</title>",
];

static LEGACY_DESCRIPTION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<span>Description: </span>([^<]+)").expect("Valid description pattern")
});

static LEGACY_UPLOAD_DATE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"hint=["'](\d{4}-\d{2}-\d{2}) \d{2}:\d{2}:\d{2} [A-Z]{3,4}"#)
        .expect("Valid upload date pattern")
});

static LEGACY_UPLOADER_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span[^>]+itemprop=["']author[^>]+><a[^>]+><span[^>]+>([^<]+)"#)
        .expect("Valid uploader pattern")
});

static LEGACY_THUMBNAIL_PATTERNS: [&str; 2] = [
    r#"["']thumbUrl["']\s*:\s*(?P<q>["'])(?P<url>.+?)(?P=q)"#,
    r#"<video[^>]+"poster"=(?P<q>["'])(?P<url>.+?)(?P=q)[^>]*>"#,
];

static LEGACY_DURATION_PATTERNS: [&str; 2] = [
    r#"<[^<]+\bitemprop=["']duration["'][^<]+\bcontent=["'](.+?)["']"#,
    r"Runtime:\s*</span>\s*([\d:]+)",
];

static LEGACY_VIEW_COUNT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"content=["']User(?:View|Play)s:(\d+)"#).expect("Valid view count pattern")
});

static LEGACY_LIKES_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"hint=['"](?P<likes>\d+) Likes / (?P<dislikes>\d+) Dislikes"#)
        .expect("Valid likes pattern")
});

static LEGACY_COMMENT_COUNT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"</label>Comments \((?P<count>\d+)\)</div>").expect("Valid comment count pattern")
});

static LEGACY_CATEGORIES_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<table.+?(<span>Categories:.+?)</table>").expect("Valid categories pattern")
});

static CATEGORY_LINK_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<a[^>]+>(.+?)</a>").expect("Valid category link pattern"));

/// Detect if a video page indicates the video is unavailable.
///
/// Returns an error message if the video is closed/removed, `None` otherwise.
pub fn detect_video_unavailable(webpage: &str) -> Option<String> {
    VIDEO_CLOSED_PATTERN.captures(webpage).and_then(|caps| {
        caps.get(1).map(|m| {
            let cleaned = BaseExtractor::clean_html_tags(m.as_str(), Some(200));
            format!("Video unavailable: {cleaned}")
        })
    })
}

/// Extract age limit from RTA meta tag.
///
/// Returns 18 if the RTA tag is present, `None` otherwise.
/// The caller should default to 18 for xHamster regardless.
pub fn extract_age_limit(webpage: &str) -> Option<u8> {
    RTA_PATTERN.is_match(webpage).then_some(18)
}

/// Extract metadata from `videoModel` JSON and build an `InfoDict`.
///
/// This is the modern layout path used when `window.initials` is found.
pub fn extract_metadata_from_json(
    video_model: &Value,
    video_id: &str,
    display_id: Option<&str>,
    url: &str,
    extractor_name: &str,
    age_limit: Option<u8>,
) -> InfoDict {
    let title = video_model
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled")
        .to_string();

    let mut info = InfoDict::new(video_id, title, extractor_name, url);

    info.description = video_model
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    info.thumbnail = video_model
        .get("thumbURL")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    info.duration = video_model
        .get("duration")
        .and_then(|v| v.as_u64())
        .map(|d| d as f64);

    info.view_count = video_model.get("views").and_then(|v| v.as_u64());

    info.like_count = video_model
        .pointer("/rating/likes")
        .and_then(|v| v.as_u64());

    info.dislike_count = video_model
        .pointer("/rating/dislikes")
        .and_then(|v| v.as_u64());

    info.comment_count = video_model.get("comments").and_then(|v| v.as_u64());

    info.upload_date = video_model
        .get("created")
        .and_then(|v| v.as_i64())
        .and_then(timestamp_to_date);

    // Uploader info from author object
    if let Some(author) = video_model.get("author") {
        info.uploader = author
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        info.uploader_url = author
            .get("pageURL")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if let Some(ref u_url) = info.uploader_url {
            info.uploader_id = u_url.split('/').next_back().map(|s| s.to_string());
        }
    }

    // Categories
    if let Some(categories_arr) = video_model.get("categories").and_then(|v| v.as_array()) {
        let cats: Vec<String> = categories_arr
            .iter()
            .filter_map(|c| {
                c.get("name")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        if !cats.is_empty() {
            info.categories = Some(cats);
        }
    }

    info.age_limit = age_limit.or(Some(18));

    if let Some(display_id) = display_id {
        debug!(display_id; "[XHamster] Display ID");
    }

    info
}

/// Extract metadata from legacy HTML page (fallback when `window.initials` is absent).
pub fn extract_metadata_from_html(
    webpage: &str,
    video_id: &str,
    display_id: Option<&str>,
    url: &str,
    extractor_name: &str,
    age_limit: Option<u8>,
) -> InfoDict {
    // Title: try multiple patterns
    let title = LEGACY_TITLE_PATTERNS
        .iter()
        .find_map(|pattern| {
            Regex::new(pattern)
                .ok()
                .and_then(|re| re.captures(webpage))
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().trim().to_string())
        })
        .or_else(|| {
            let html = Html::parse_document(webpage);
            BaseExtractor::extract_title_multi_strategy(&html)
        })
        .unwrap_or_else(|| "Untitled".to_string());

    let mut info = InfoDict::new(video_id, title, extractor_name, url);

    // Description
    info.description = LEGACY_DESCRIPTION_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string());

    // Upload date (YYYY-MM-DD → YYYYMMDD)
    info.upload_date = LEGACY_UPLOAD_DATE_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().replace('-', ""));

    // Uploader
    let uploader = LEGACY_UPLOADER_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "anonymous".to_string());
    info.uploader_id = Some(uploader.to_lowercase());
    info.uploader = Some(uploader);

    // Thumbnail
    info.thumbnail = LEGACY_THUMBNAIL_PATTERNS.iter().find_map(|pattern| {
        Regex::new(pattern)
            .ok()
            .and_then(|re| re.captures(webpage))
            .and_then(|caps| caps.name("url"))
            .map(|m| m.as_str().to_string())
    });

    // Duration (parse "PT1M30S" or "1:30" formats)
    info.duration = LEGACY_DURATION_PATTERNS.iter().find_map(|pattern| {
        Regex::new(pattern)
            .ok()
            .and_then(|re| re.captures(webpage))
            .and_then(|caps| caps.get(1))
            .and_then(|m| BaseExtractor::parse_duration(m.as_str().trim()))
    });

    // View count
    info.view_count = LEGACY_VIEW_COUNT_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok());

    // Like/dislike counts
    if let Some(caps) = LEGACY_LIKES_PATTERN.captures(webpage) {
        info.like_count = caps.name("likes").and_then(|m| m.as_str().parse().ok());
    }

    // Comment count
    info.comment_count = LEGACY_COMMENT_COUNT_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.name("count"))
        .and_then(|m| m.as_str().parse().ok());

    // Categories
    if let Some(cats_html) = LEGACY_CATEGORIES_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str())
    {
        let cats: Vec<String> = CATEGORY_LINK_PATTERN
            .captures_iter(cats_html)
            .filter_map(|caps| {
                caps.get(1)
                    .map(|m| BaseExtractor::clean_html_tags(m.as_str(), None))
            })
            .filter(|s| !s.is_empty())
            .collect();
        if !cats.is_empty() {
            info.categories = Some(cats);
        }
    }

    info.age_limit = age_limit.or(Some(18));

    if let Some(display_id) = display_id {
        debug!(display_id; "[XHamster] Legacy display ID");
    }

    info
}

/// Convert a Unix timestamp to `YYYYMMDD` string without chrono.
///
/// Uses a simple day-counting algorithm. Good enough for dates after 1970.
fn timestamp_to_date(timestamp: i64) -> Option<String> {
    if timestamp < 0 {
        return None;
    }
    let mut days = (timestamp / 86400) as i32;
    let mut year = 1970;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i + 1;
            break;
        }
        days -= md;
    }
    let day = days + 1;

    Some(format!("{year:04}{month:02}{day:02}"))
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_video_unavailable() {
        let html = r#"<div id="videoClosed" class="error">This video has been removed</div>"#;
        let result = detect_video_unavailable(html);
        assert!(result.is_some());
        assert!(result.unwrap().contains("removed"));
    }

    #[test]
    fn test_detect_video_available() {
        let html = r#"<div class="video-player">Normal content</div>"#;
        assert!(detect_video_unavailable(html).is_none());
    }

    #[test]
    fn test_extract_age_limit() {
        let html = r#"<meta name="rating" content="RTA-5042-1996-1400-1577-RTA">"#;
        assert_eq!(extract_age_limit(html), Some(18));

        let html = r#"<meta name="description" content="normal">"#;
        assert_eq!(extract_age_limit(html), None);
    }

    #[test]
    fn test_extract_metadata_from_json() {
        let json: Value = serde_json::json!({
            "title": "Test Video",
            "description": "A test",
            "thumbURL": "https://example.com/thumb.jpg",
            "duration": 893,
            "views": 12345,
            "comments": 42,
            "rating": {"likes": 100, "dislikes": 5},
            "author": {
                "name": "TestUser",
                "pageURL": "https://xhamster.com/users/testuser"
            },
            "categories": [
                {"name": "Amateur"},
                {"name": "HD"}
            ]
        });

        let info = extract_metadata_from_json(
            &json,
            "123",
            Some("test-video"),
            "https://xhamster.com/videos/test-video-123",
            "XHamster",
            Some(18),
        );

        assert_eq!(info.title, "Test Video");
        assert_eq!(info.description, Some("A test".to_string()));
        assert_eq!(
            info.thumbnail,
            Some("https://example.com/thumb.jpg".to_string())
        );
        assert_eq!(info.duration, Some(893.0));
        assert_eq!(info.view_count, Some(12345));
        assert_eq!(info.like_count, Some(100));
        assert_eq!(info.comment_count, Some(42));
        assert_eq!(info.uploader, Some("TestUser".to_string()));
        assert_eq!(
            info.uploader_url,
            Some("https://xhamster.com/users/testuser".to_string())
        );
        assert_eq!(info.uploader_id, Some("testuser".to_string()));
        assert_eq!(
            info.categories,
            Some(vec!["Amateur".to_string(), "HD".to_string()])
        );
        assert_eq!(info.age_limit, Some(18));
    }
}

// ============================================================================
// Actor extraction
// ============================================================================

static PORNSTAR_LINK_SELECTOR: LazyLock<scraper::Selector> = LazyLock::new(|| {
    scraper::Selector::parse(r#"a[href*="/pornstars/"]"#).expect("valid selector")
});

/// Extract pornstar/actor names from the video page HTML.
///
/// Looks for `<a href="/pornstars/name">Name</a>` links in the video info
/// section. Filters out navigation links ("By Countries", empty text).
pub fn extract_actors(webpage: &str) -> Vec<String> {
    let html = Html::parse_document(webpage);
    let mut seen = std::collections::HashSet::new();
    let mut actors = Vec::new();

    for link in html.select(&PORNSTAR_LINK_SELECTOR) {
        let name = link.text().collect::<String>();
        let name = name.trim().to_string();

        // Skip empty, navigation, and duplicate entries
        if name.is_empty()
            || name == "By Countries"
            || name.starts_with('#')
            || !seen.insert(name.clone())
        {
            continue;
        }

        actors.push(name);
    }

    actors
}

#[cfg(test)]
mod actor_tests {
    use super::*;

    #[test]
    fn extract_actors_from_html() {
        let html = r#"<html><body>
            <a href="/pornstars/mia-malkova" class="item-50dd2">Mia Malkova</a>
            <a href="/pornstars/jodi-taylor" class="item-50dd2">Jodi Taylor</a>
            <a href="/pornstars/all/countries">By Countries</a>
            <a href="/pornstars/mia-malkova">Mia Malkova</a>
        </body></html>"#;
        let actors = extract_actors(html);
        assert_eq!(actors, vec!["Mia Malkova", "Jodi Taylor"]);
    }

    #[test]
    fn extract_actors_none() {
        let html = r#"<html><body><p>No pornstar links here</p></body></html>"#;
        let actors = extract_actors(html);
        assert!(actors.is_empty());
    }

    #[test]
    fn extract_actors_filters_navigation() {
        let html = r#"<html><body>
            <a href="/pornstars/all/countries">By Countries</a>
            <a href="/pornstars/kelsi-monroe">#697</a>
            <a href="/pornstars/kelsi-monroe">Kelsi Monroe</a>
        </body></html>"#;
        let actors = extract_actors(html);
        assert_eq!(actors, vec!["Kelsi Monroe"]);
    }
}
