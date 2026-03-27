//! PornHub search result parsing and filter validation.
//!
//! Provides serde structs for the PornHub Webmaster JSON API response,
//! conversion to `SearchResultPreview`, and filter validation.

use log::debug;
use rdlp_core::{RdlpError, Result};
use rdlp_types::{SearchFilter, SearchFilterDescriptor, SearchResultPreview};
use serde::Deserialize;

use super::search_patterns;

/// Top-level API response from `pornhub.com/webmasters/search`.
#[derive(Debug, PartialEq, Deserialize)]
pub(crate) struct ApiSearchResponse {
    /// List of video objects. May be absent when no results found.
    #[serde(default)]
    pub videos: Vec<ApiVideo>,
}

/// A single video entry from the API.
///
/// Unlike RedTube, PornHub's API returns videos directly (not wrapped in
/// `{ "video": {...} }`).
#[derive(Debug, PartialEq, Deserialize)]
pub(crate) struct ApiVideo {
    /// Video ID (e.g. "ph5a1..." or numeric string).
    pub video_id: String,
    /// Video title.
    pub title: String,
    /// Full URL to the video page.
    pub url: String,
    /// Thumbnail URL.
    #[serde(default)]
    pub thumb: Option<String>,
    /// Fallback thumbnail URL.
    #[serde(default)]
    pub default_thumb: Option<String>,
    /// Duration string in "MM:SS" or "H:MM:SS" format.
    #[serde(default)]
    pub duration: Option<String>,
    /// View count as a number.
    #[serde(default)]
    pub views: Option<u64>,
    /// Rating percentage (0.0 - 100.0).
    #[serde(default)]
    #[allow(dead_code)]
    pub rating: Option<f64>,
    /// Publication date string.
    #[serde(default)]
    pub publish_date: Option<String>,
    /// Tag list.
    #[serde(default)]
    #[allow(dead_code)]
    pub tags: Vec<ApiTag>,
    /// Pornstar list.
    #[serde(default)]
    #[allow(dead_code)]
    pub pornstars: Vec<ApiPornstar>,
    /// Category list.
    #[serde(default)]
    #[allow(dead_code)]
    pub categories: Vec<ApiCategory>,
}

/// A single tag entry.
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct ApiTag {
    #[allow(dead_code)]
    pub tag_name: String,
}

/// A single pornstar entry.
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct ApiPornstar {
    #[allow(dead_code)]
    pub pornstar_name: String,
}

/// A single category entry.
#[derive(Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct ApiCategory {
    #[allow(dead_code)]
    pub category: String,
}

/// Parse the JSON API response into `SearchResultPreview` items.
///
/// # Arguments
/// * `json` - Raw JSON response body from the API.
///
/// # Returns
/// A vector of search result previews.
pub(crate) fn parse_api_search_results(json: &str) -> Result<Vec<SearchResultPreview>> {
    let response: ApiSearchResponse = serde_json::from_str(json).map_err(|e| RdlpError::Extraction {
        message: format!("Failed to parse PornHub API response: {e}"),
        url: None,
    })?;

    let results: Vec<SearchResultPreview> = response
        .videos
        .into_iter()
        .filter_map(api_video_to_preview)
        .collect();

    Ok(results)
}

/// Convert an `ApiVideo` into a `SearchResultPreview`.
///
/// Returns `None` if the URL is empty (skips invalid entries).
fn api_video_to_preview(video: ApiVideo) -> Option<SearchResultPreview> {
    if video.url.is_empty() {
        debug!(
            "[PornHub] Search result missing URL for video_id={}, skipping",
            video.video_id
        );
        return None;
    }

    let duration = video
        .duration
        .as_deref()
        .and_then(parse_duration_string)
        .map(|s| s as f64);

    let thumbnail = video.thumb.or(video.default_thumb);

    Some(SearchResultPreview {
        video_url: video.url,
        title: video.title,
        thumbnail_url: thumbnail,
        duration,
        view_count: video.views,
        upload_date: video.publish_date,
    })
}

/// Parse a duration string like "12:34" or "1:02:34" into total seconds.
///
/// # Examples
/// - "12:34" → 754
/// - "1:02:34" → 3754
/// - "0:45" → 45
/// - "" → None
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

/// Validate search filters against PornHub's known filter descriptors.
///
/// Returns `Ok(())` if all filters are valid, or an error for the first invalid one.
///
/// # Arguments
/// * `filters` - Slice of active search filters to validate.
///
/// # Returns
/// `Ok(())` on success, `Err` describing the first invalid filter.
pub(crate) fn validate_search_filters(filters: &[SearchFilter]) -> Result<()> {
    let descriptors: Vec<SearchFilterDescriptor> = search_patterns::search_filter_descriptors();

    for filter in filters {
        let descriptor = descriptors.iter().find(|d| d.key == filter.key);

        match descriptor {
            None => {
                let valid_keys: Vec<&str> = descriptors.iter().map(|d| d.key.as_str()).collect();
                return Err(RdlpError::Extraction {
                    message: format!(
                        "Unknown filter '{}' for PornHub. Available: {}",
                        filter.key,
                        valid_keys.join(", ")
                    ),
                    url: None,
                });
            }
            Some(desc) => {
                // category and tags accept free-text — API validates server-side
                if filter.key == "category" || filter.key == "tags" {
                    continue;
                }

                let valid = desc.allowed_values.iter().any(|v| v.value == filter.value);
                if !valid {
                    let allowed: Vec<&str> = desc
                        .allowed_values
                        .iter()
                        .map(|v| v.value.as_str())
                        .collect();
                    return Err(RdlpError::Extraction {
                        message: format!(
                            "Invalid value '{}' for filter '{}'. Allowed: {}",
                            filter.value,
                            filter.key,
                            allowed.join(", ")
                        ),
                        url: None,
                    });
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_api_response() -> &'static str {
        r#"{
            "videos": [
                {
                    "video_id": "ph5a1b2c3d",
                    "title": "Test Video One",
                    "url": "https://www.pornhub.com/view_video.php?viewkey=ph5a1b2c3d",
                    "thumb": "https://ci.phncdn.com/thumb1.jpg",
                    "default_thumb": "https://ci.phncdn.com/default1.jpg",
                    "duration": "12:34",
                    "views": 123456,
                    "rating": 95.5,
                    "publish_date": "2025-01-15 10:30:00",
                    "tags": [{"tag_name": "amateur"}, {"tag_name": "hd"}],
                    "pornstars": [{"pornstar_name": "Test Star"}],
                    "categories": [{"category": "amateur"}]
                },
                {
                    "video_id": "ph6b2c3d4e",
                    "title": "Test Video Two",
                    "url": "https://www.pornhub.com/view_video.php?viewkey=ph6b2c3d4e",
                    "thumb": "https://ci.phncdn.com/thumb2.jpg",
                    "duration": "1:02:03",
                    "views": 789012,
                    "rating": 88.0,
                    "publish_date": "2025-02-20 14:00:00",
                    "tags": [],
                    "pornstars": [],
                    "categories": []
                }
            ]
        }"#
    }

    #[test]
    fn test_parse_api_search_results() {
        let results = parse_api_search_results(sample_api_response()).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Test Video One");
        assert_eq!(
            results[0].video_url,
            "https://www.pornhub.com/view_video.php?viewkey=ph5a1b2c3d"
        );
        assert_eq!(
            results[0].thumbnail_url,
            Some("https://ci.phncdn.com/thumb1.jpg".to_string())
        );
        assert_eq!(results[0].duration, Some(754.0));
        assert_eq!(results[0].view_count, Some(123456));
        assert_eq!(
            results[0].upload_date,
            Some("2025-01-15 10:30:00".to_string())
        );
    }

    #[test]
    fn test_parse_api_search_results_second_video() {
        let results = parse_api_search_results(sample_api_response()).unwrap();
        assert_eq!(results[1].title, "Test Video Two");
        assert_eq!(results[1].duration, Some(3723.0));
        assert_eq!(results[1].view_count, Some(789012));
    }

    #[test]
    fn test_parse_api_search_results_empty() {
        let json = r#"{"videos": []}"#;
        let results = parse_api_search_results(json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_api_search_results_missing_videos_key() {
        let json = r#"{}"#;
        let results = parse_api_search_results(json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_api_search_results_invalid_json() {
        let result = parse_api_search_results("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_api_video_missing_url_skipped() {
        let json = r#"{
            "videos": [{
                "video_id": "999",
                "title": "Missing URL",
                "url": "",
                "tags": [],
                "pornstars": [],
                "categories": []
            }]
        }"#;
        let results = parse_api_search_results(json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_api_video_minimal_fields() {
        let json = r#"{
            "videos": [{
                "video_id": "111",
                "title": "Minimal",
                "url": "https://www.pornhub.com/view_video.php?viewkey=111"
            }]
        }"#;
        let results = parse_api_search_results(json).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Minimal");
        assert_eq!(results[0].duration, None);
        assert_eq!(results[0].view_count, None);
        assert_eq!(results[0].thumbnail_url, None);
        assert_eq!(results[0].upload_date, None);
    }

    #[test]
    fn test_api_video_fallback_to_default_thumb() {
        let json = r#"{
            "videos": [{
                "video_id": "222",
                "title": "Default Thumb",
                "url": "https://www.pornhub.com/view_video.php?viewkey=222",
                "default_thumb": "https://ci.phncdn.com/default.jpg"
            }]
        }"#;
        let results = parse_api_search_results(json).unwrap();
        assert_eq!(
            results[0].thumbnail_url,
            Some("https://ci.phncdn.com/default.jpg".to_string())
        );
    }

    #[test]
    fn test_parse_duration_string_mm_ss() {
        assert_eq!(parse_duration_string("12:34"), Some(754));
        assert_eq!(parse_duration_string("0:45"), Some(45));
        assert_eq!(parse_duration_string("59:59"), Some(3599));
    }

    #[test]
    fn test_parse_duration_string_h_mm_ss() {
        assert_eq!(parse_duration_string("1:02:03"), Some(3723));
        assert_eq!(parse_duration_string("0:00:00"), Some(0));
        assert_eq!(parse_duration_string("2:30:00"), Some(9000));
    }

    #[test]
    fn test_parse_duration_string_invalid() {
        assert_eq!(parse_duration_string(""), None);
        assert_eq!(parse_duration_string("abc"), None);
        assert_eq!(parse_duration_string("12"), None);
        assert_eq!(parse_duration_string("1:2:3:4"), None);
    }

    #[test]
    fn test_validate_filters_valid() {
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
        assert!(validate_search_filters(&filters).is_ok());
    }

    #[test]
    fn test_validate_filters_free_text_category() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "any-value-here".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_ok());
    }

    #[test]
    fn test_validate_filters_free_text_tags() {
        let filters = vec![SearchFilter {
            key: "tags".to_string(),
            value: "tag1,tag2,tag3".to_string(),
        }];
        assert!(validate_search_filters(&filters).is_ok());
    }

    #[test]
    fn test_validate_filters_invalid_key() {
        let filters = vec![SearchFilter {
            key: "nonexistent".to_string(),
            value: "foo".to_string(),
        }];
        let err = validate_search_filters(&filters).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
        assert!(err.to_string().contains("Available"));
    }

    #[test]
    fn test_validate_filters_invalid_ordering_value() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "invalid_order".to_string(),
        }];
        let err = validate_search_filters(&filters).unwrap_err();
        assert!(err.to_string().contains("invalid_order"));
        assert!(err.to_string().contains("Allowed"));
    }

    #[test]
    fn test_validate_filters_empty() {
        assert!(validate_search_filters(&[]).is_ok());
    }
}
