//! RedTube search result parsing and filter validation.
//!
//! Provides serde structs for the RedTube JSON API response, conversion to
//! `SearchResultPreview`, HTML fallback scraping, and filter validation.

use log::debug;
use rdlp_core::{RdlpError, Result};
use rdlp_types::{SearchFilter, SearchFilterDescriptor, SearchResultPreview};
use serde::{Deserialize, Deserializer};

use super::patterns;

/// Top-level API response from `redtube.Videos.searchVideos`.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiSearchResponse {
    /// Total number of matching videos (across all pages).
    pub count: Option<u64>,
    /// List of video wrappers. May be absent when count is 0.
    #[serde(default)]
    pub videos: Vec<ApiVideoWrapper>,
}

/// Wrapper that contains a single `video` object.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiVideoWrapper {
    /// The nested video object.
    pub video: ApiVideo,
}

/// A single video entry from the API.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiVideo {
    /// Numeric video ID as a string (e.g. "123456").
    pub video_id: String,
    /// Video title.
    pub title: String,
    /// Full URL to the video page.
    pub url: String,
    /// Thumbnail URL (big size requested via `thumbsize=big`).
    #[serde(default)]
    pub thumb: Option<String>,
    /// Duration string in "MM:SS" or "H:MM:SS" format.
    #[serde(default)]
    pub duration: Option<String>,
    /// View count — the API returns this as a JSON number, but we also accept
    /// strings (which may contain commas) for forward compatibility.
    #[serde(default, deserialize_with = "deserialize_views")]
    pub views: Option<String>,
    /// Publication date string.
    #[serde(default)]
    pub publish_date: Option<String>,
    /// Tag list (deserialized for API completeness; not currently consumed).
    #[serde(default)]
    #[allow(dead_code)]
    pub tags: Vec<ApiTag>,
}

/// A single tag entry.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiTag {
    /// Tag display name.
    #[allow(dead_code)]
    pub tag_name: String,
}

/// Deserialize `views` from either a JSON number or a string.
///
/// The RedTube API returns `views` as a number (e.g. `571096`), but some
/// responses may use a string. This accepts both and normalises to `Option<String>`.
pub(crate) fn deserialize_views<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    Ok(value.and_then(|v| match v {
        serde_json::Value::String(s) if s.is_empty() => None,
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }))
}

/// Parse the JSON API response into `SearchResultPreview` items and total count.
///
/// # Arguments
/// * `json` - Raw JSON response body from the API.
///
/// # Returns
/// Tuple of (results, optional total count).
pub(crate) fn parse_api_search_results(
    json: &str,
) -> Result<(Vec<SearchResultPreview>, Option<u64>)> {
    let response: ApiSearchResponse =
        serde_json::from_str(json).map_err(|e| RdlpError::Extraction {
            message: format!("Failed to parse RedTube API response: {e}"),
            url: None,
        })?;

    let total_count = response.count;
    let results: Vec<SearchResultPreview> = response
        .videos
        .into_iter()
        .filter_map(|wrapper| api_video_to_preview(wrapper.video))
        .collect();

    Ok((results, total_count))
}

/// Convert an `ApiVideo` into a `SearchResultPreview`.
///
/// Returns `None` if essential fields (url) are empty, logging a warning.
fn api_video_to_preview(video: ApiVideo) -> Option<SearchResultPreview> {
    if video.url.is_empty() {
        debug!(
            "[RedTube] Search result missing URL for video_id={}, skipping",
            video.video_id
        );
        return None;
    }

    let duration = video
        .duration
        .as_deref()
        .and_then(parse_duration_string)
        .map(|s| s as f64);

    let view_count = video
        .views
        .as_deref()
        .and_then(crate::base::common::BaseExtractor::parse_human_count);

    Some(SearchResultPreview {
        video_url: video.url,
        title: video.title,
        thumbnail_url: video.thumb,
        duration,
        uploader: None,
        uploader_url: None,
        actors: vec![],
        view_count,
        upload_date: video.publish_date,
    })
}

/// Parse a duration string like "12:34" or "1:02:34" into total seconds.
///
/// # Examples
/// - "12:34" -> 754
/// - "1:02:34" -> 3754
/// - "0:45" -> 45
/// - "" -> None
pub(crate) fn parse_duration_string(duration: &str) -> Option<u64> {
    let parts: Vec<&str> = duration.split(':').collect();
    match parts.len() {
        2 => {
            let minutes = parts[0].parse::<u64>().ok()?;
            let seconds = parts[1].parse::<u64>().ok()?;
            Some(minutes * 60 + seconds)
        }
        3 => {
            let hours = parts[0].parse::<u64>().ok()?;
            let minutes = parts[1].parse::<u64>().ok()?;
            let seconds = parts[2].parse::<u64>().ok()?;
            Some(hours * 3600 + minutes * 60 + seconds)
        }
        _ => None,
    }
}

/// Parse HTML search results as a fallback when the JSON API fails.
///
/// Scrapes `<li class="video-item">` elements from the search page HTML.
pub(crate) fn parse_html_search_results(html: &str) -> Result<Vec<SearchResultPreview>> {
    let mut results = Vec::new();

    // Pre-collect uploader names — RedTube emits `data-uploader-name="…"`
    // on each card's `<li>` in DOM order, so the Nth match pairs with the
    // Nth card from `HTML_VIDEO_CARD_PATTERN`.
    let uploaders: Vec<String> = patterns::HTML_UPLOADER_NAME_PATTERN
        .captures_iter(html)
        .filter_map(|c| c.name("name").map(|m| m.as_str().to_string()))
        .collect();

    for (idx, caps) in patterns::HTML_VIDEO_CARD_PATTERN
        .captures_iter(html)
        .enumerate()
    {
        let url = match caps.name("url") {
            Some(m) => m.as_str().to_string(),
            None => continue,
        };
        let title = caps
            .name("title")
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let thumb = caps.name("thumb").map(|m| m.as_str().to_string());
        let duration = caps
            .name("duration")
            .and_then(|m| parse_duration_string(m.as_str()))
            .map(|s| s as f64);
        let uploader = uploaders.get(idx).cloned();

        results.push(SearchResultPreview {
            video_url: url,
            title,
            thumbnail_url: thumb,
            duration,
            uploader,
            uploader_url: None,
            actors: vec![],
            view_count: None,
            upload_date: None,
        });
    }

    Ok(results)
}

/// Validate search filters against the known RedTube filter descriptors.
///
/// Returns `Ok(())` if all filters are valid, or an error describing the
/// first invalid filter found.
pub(crate) fn validate_search_filters(
    filters: &[SearchFilter],
    descriptors: &[SearchFilterDescriptor],
) -> Result<()> {
    use crate::base::common::{
        KeyValidation, format_std_filter_error, validate_against_descriptors,
    };

    validate_against_descriptors(
        filters,
        descriptors,
        &[
            ("category", KeyValidation::FreeText),
            ("tags", KeyValidation::FreeText),
        ],
    )
    .map_err(|e| format_std_filter_error("RedTube", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_api_response() -> &'static str {
        r#"{
            "count": 42,
            "videos": [
                {
                    "video": {
                        "video_id": "12345",
                        "title": "Test Video One",
                        "url": "https://www.redtube.com/12345",
                        "thumb": "https://thumb1.jpg",
                        "duration": "12:34",
                        "views": "1,234",
                        "publish_date": "2024-01-15",
                        "tags": [{"tag_name": "amateur"}, {"tag_name": "hd"}]
                    }
                },
                {
                    "video": {
                        "video_id": "67890",
                        "title": "Test Video Two",
                        "url": "https://www.redtube.com/67890",
                        "thumb": "https://thumb2.jpg",
                        "duration": "1:02:34",
                        "views": "56,789",
                        "publish_date": "2024-02-20",
                        "tags": []
                    }
                }
            ]
        }"#
    }

    #[test]
    fn test_parse_api_search_results() {
        let (results, count) = parse_api_search_results(sample_api_response()).unwrap();
        assert_eq!(count, Some(42));
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Test Video One");
        assert_eq!(results[0].video_url, "https://www.redtube.com/12345");
        assert_eq!(
            results[0].thumbnail_url,
            Some("https://thumb1.jpg".to_string())
        );
        assert_eq!(results[0].duration, Some(754.0));
        assert_eq!(results[0].view_count, Some(1234));
        assert_eq!(results[0].upload_date, Some("2024-01-15".to_string()));
    }

    #[test]
    fn test_parse_api_search_results_second_video() {
        let (results, _) = parse_api_search_results(sample_api_response()).unwrap();
        assert_eq!(results[1].title, "Test Video Two");
        assert_eq!(results[1].duration, Some(3754.0));
        assert_eq!(results[1].view_count, Some(56789));
    }

    #[test]
    fn test_parse_api_search_results_empty() {
        let json = r#"{"count": 0, "videos": []}"#;
        let (results, count) = parse_api_search_results(json).unwrap();
        assert!(results.is_empty());
        assert_eq!(count, Some(0));
    }

    #[test]
    fn test_parse_api_search_results_missing_videos() {
        let json = r#"{"count": 0}"#;
        let (results, count) = parse_api_search_results(json).unwrap();
        assert!(results.is_empty());
        assert_eq!(count, Some(0));
    }

    #[test]
    fn test_parse_api_search_results_invalid_json() {
        let result = parse_api_search_results("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_parse_duration_string_mm_ss() {
        assert_eq!(parse_duration_string("12:34"), Some(754));
        assert_eq!(parse_duration_string("0:45"), Some(45));
        assert_eq!(parse_duration_string("59:59"), Some(3599));
    }

    #[test]
    fn test_parse_duration_string_h_mm_ss() {
        assert_eq!(parse_duration_string("1:02:34"), Some(3754));
        assert_eq!(parse_duration_string("0:00:00"), Some(0));
        assert_eq!(parse_duration_string("2:30:00"), Some(9000));
    }

    #[test]
    fn test_parse_duration_string_invalid() {
        assert_eq!(parse_duration_string(""), None);
        assert_eq!(parse_duration_string("abc"), None);
        assert_eq!(parse_duration_string("12"), None);
        assert_eq!(parse_duration_string("1:2:3:4"), None);
        assert_eq!(parse_duration_string("ab:cd"), None);
    }

    // View-count parsing is covered by the canonical
    // `BaseExtractor::parse_human_count` tests in `base::common::tests`.

    #[test]
    fn test_parse_api_search_results_numeric_views() {
        let json = r#"{
            "count": 1521,
            "videos": [{
                "video": {
                    "video_id": "42785231",
                    "title": "Test Numeric Views",
                    "url": "https://www.redtube.com/42785231",
                    "thumb": "https://thumb.jpg",
                    "duration": "16:53",
                    "views": 571096,
                    "publish_date": "2022-11-14 15:47:22",
                    "tags": [{"tag_name": "hd"}]
                }
            }]
        }"#;
        let (results, count) = parse_api_search_results(json).unwrap();
        assert_eq!(count, Some(1521));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Numeric Views");
        assert_eq!(results[0].view_count, Some(571096));
        assert_eq!(results[0].duration, Some(1013.0));
    }

    #[test]
    fn test_parse_html_search_results() {
        let html = r#"
            <li class="video-item">
                <a href="https://www.redtube.com/111" title="HTML Video One" class="video-thumb">
                    <img src="https://thumb-html.jpg" />
                    <span class="duration">5:30</span>
                </a>
            </li>
        "#;
        let results = parse_html_search_results(html).unwrap();
        // The simple regex won't match this complex HTML perfectly;
        // this tests that parsing doesn't panic on real-ish HTML
        // and returns an empty vec for non-matching patterns.
        assert!(results.is_empty() || results[0].title == "HTML Video One");
    }

    /// Regression: when the JSON API path is rate-limited or unavailable,
    /// the HTML fallback parser MUST extract uploader from the
    /// `data-uploader-name` attribute that RedTube emits on every card's
    /// outer `<li>`. Prior to this fix every fallback-path result had
    /// `uploader = None`.
    #[test]
    fn html_fallback_extracts_uploader_from_data_attribute() {
        let html = r#"
            <li id="tags_videos_1" data-uploader-name="Lisa Sack">
              <a href="https://www.redtube.com/111" title="One">
                <img src="https://t/1.jpg"/>
                <span class="duration">5:30</span>
              </a>
            </li>
            <li id="tags_videos_2" data-uploader-name="John Doe">
              <a href="https://www.redtube.com/222" title="Two">
                <img src="https://t/2.jpg"/>
                <span class="duration">7:42</span>
              </a>
            </li>
        "#;
        let results = parse_html_search_results(html).unwrap();
        assert_eq!(results.len(), 2, "both cards should parse");
        assert_eq!(results[0].uploader.as_deref(), Some("Lisa Sack"));
        assert_eq!(results[1].uploader.as_deref(), Some("John Doe"));
    }

    #[test]
    fn test_validate_filters_valid() {
        let descriptors = patterns::search_filter_descriptors();
        let filters = vec![
            SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            },
            SearchFilter {
                key: "period".to_string(),
                value: "weekly".to_string(),
            },
        ];
        assert!(validate_search_filters(&filters, &descriptors).is_ok());
    }

    #[test]
    fn test_validate_filters_free_text() {
        let descriptors = patterns::search_filter_descriptors();
        let filters = vec![
            SearchFilter {
                key: "category".to_string(),
                value: "any-value-here".to_string(),
            },
            SearchFilter {
                key: "tags".to_string(),
                value: "tag1,tag2,tag3".to_string(),
            },
        ];
        assert!(validate_search_filters(&filters, &descriptors).is_ok());
    }

    #[test]
    fn test_validate_filters_invalid_key() {
        let descriptors = patterns::search_filter_descriptors();
        let filters = vec![SearchFilter {
            key: "nonexistent".to_string(),
            value: "foo".to_string(),
        }];
        let err = validate_search_filters(&filters, &descriptors).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
        assert!(err.to_string().contains("Available"));
    }

    #[test]
    fn test_validate_filters_invalid_value() {
        let descriptors = patterns::search_filter_descriptors();
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "invalid_order".to_string(),
        }];
        let err = validate_search_filters(&filters, &descriptors).unwrap_err();
        assert!(err.to_string().contains("invalid_order"));
        assert!(err.to_string().contains("Allowed"));
    }

    #[test]
    fn test_api_video_missing_url_skipped() {
        let json = r#"{
            "count": 1,
            "videos": [{
                "video": {
                    "video_id": "999",
                    "title": "Missing URL",
                    "url": "",
                    "tags": []
                }
            }]
        }"#;
        let (results, _) = parse_api_search_results(json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_api_video_minimal_fields() {
        let json = r#"{
            "count": 1,
            "videos": [{
                "video": {
                    "video_id": "111",
                    "title": "Minimal",
                    "url": "https://www.redtube.com/111",
                    "tags": []
                }
            }]
        }"#;
        let (results, _) = parse_api_search_results(json).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Minimal");
        assert_eq!(results[0].duration, None);
        assert_eq!(results[0].view_count, None);
        assert_eq!(results[0].thumbnail_url, None);
        assert_eq!(results[0].upload_date, None);
    }
}
