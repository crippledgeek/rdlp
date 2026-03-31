//! XHamster search result parsing and filter validation.

use anyhow::Context as _;
use log::debug;
use rdlp_core::{RdlpError, Result};
use rdlp_types::{SearchFilter, SearchResultPreview};
use serde_json::Value;

use super::patterns;

/// Parse `window.initials` JSON from the search page HTML.
pub fn extract_initials_json(html: &str) -> Result<Value> {
    extract_initials_json_impl(html).map_err(|e| RdlpError::Extraction {
        message: format!("{e:#}"),
        url: None,
    })
}

fn extract_initials_json_impl(html: &str) -> anyhow::Result<Value> {
    let json_str = [
        &*patterns::INITIALS_PATTERN,
        &*patterns::INITIALS_FALLBACK_PATTERN,
    ]
    .iter()
    .find_map(|pat| pat.captures(html))
    .and_then(|caps| caps.get(1))
    .map(|m| m.as_str())
    .ok_or_else(|| anyhow::anyhow!("could not find window.initials in search page"))?;

    serde_json::from_str(json_str)
        .context("failed to parse window.initials JSON")
}

/// Parse search result previews from the `window.initials` JSON.
pub fn parse_search_results_json(initials: &Value) -> Result<Vec<SearchResultPreview>> {
    parse_search_results_json_impl(initials).map_err(|e| RdlpError::Extraction {
        message: format!("{e:#}"),
        url: None,
    })
}

fn parse_search_results_json_impl(initials: &Value) -> anyhow::Result<Vec<SearchResultPreview>> {
    let data = initials
        .pointer("/searchResult/videoThumbProps")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow::anyhow!("missing searchResult.videoThumbProps array in initials JSON"))?;

    let mut results = Vec::with_capacity(data.len());
    for item in data {
        let video_url = match item.get("pageURL").and_then(|v| v.as_str()) {
            Some(url) => url.to_string(),
            None => {
                debug!("[XHamster] Search result missing pageURL, skipping");
                continue;
            }
        };
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let thumbnail_url = item
            .get("thumbURL")
            .and_then(|v| v.as_str())
            .map(String::from);
        let duration = item.get("duration").and_then(|v| v.as_f64());
        let view_count = item.get("views").and_then(|v| v.as_u64());
        // `created` is a Unix timestamp on xHamster
        let upload_date = item
            .get("created")
            .and_then(|v| v.as_u64())
            .map(|ts| ts.to_string());

        results.push(SearchResultPreview {
            video_url,
            title,
            thumbnail_url,
            duration,
            uploader: None,
            view_count,
            upload_date,
        });
    }

    Ok(results)
}

/// Extract the max page count from the initials JSON.
pub fn parse_max_pages(initials: &Value) -> Option<usize> {
    // Primary: top-level pagination object
    initials
        .pointer("/pagination/maxPages")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
}

/// Validate search filters against the known xHamster filter descriptors.
pub fn validate_search_filters(filters: &[SearchFilter]) -> Result<()> {
    let descriptors = patterns::search_filter_descriptors();

    for filter in filters {
        let descriptor = descriptors.iter().find(|d| d.key == filter.key);

        match descriptor {
            None => {
                let valid_keys: Vec<&str> = descriptors.iter().map(|d| d.key.as_str()).collect();
                return Err(RdlpError::Extraction {
                    message: format!(
                        "Unknown filter '{}' for XHamster. Available: {}",
                        filter.key,
                        valid_keys.join(", ")
                    ),
                    url: None,
                });
            }
            Some(desc) => {
                // Duration min/max are numeric, allow any reasonable value
                if filter.key == "min-duration" || filter.key == "max-duration" {
                    if filter.value.parse::<u32>().is_err() {
                        return Err(RdlpError::Extraction {
                            message: format!(
                                "Invalid value '{}' for filter '{}'. Must be a number.",
                                filter.value, filter.key
                            ),
                            url: None,
                        });
                    }
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

    fn sample_initials_json() -> &'static str {
        r#"{
            "searchResult": {
                "videoThumbProps": [
                    {
                        "id": 12345,
                        "title": "Test Video One",
                        "pageURL": "https://xhamster.com/videos/test-video-one-xh12345",
                        "thumbURL": "https://thumb1.jpg",
                        "duration": 180,
                        "views": 5000,
                        "created": 1705276800
                    },
                    {
                        "id": 67890,
                        "title": "Test Video Two",
                        "pageURL": "https://xhamster.com/videos/test-video-two-xh67890",
                        "thumbURL": "https://thumb2.jpg",
                        "duration": 360,
                        "views": 12000,
                        "created": 1706745600
                    }
                ]
            },
            "pagination": {
                "active": 1,
                "next": 2,
                "maxPages": 5
            }
        }"#
    }

    #[test]
    fn test_parse_search_results_from_json() {
        let json: serde_json::Value = serde_json::from_str(sample_initials_json()).unwrap();
        let results = parse_search_results_json(&json).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Test Video One");
        assert_eq!(
            results[0].video_url,
            "https://xhamster.com/videos/test-video-one-xh12345"
        );
        assert_eq!(results[0].duration, Some(180.0));
        assert_eq!(results[0].view_count, Some(5000));
        assert_eq!(results[1].title, "Test Video Two");
    }

    #[test]
    fn test_parse_search_results_empty() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"searchResult": {"videoThumbProps": []}, "pagination": {"maxPages": 0}}"#,
        )
        .unwrap();
        let results = parse_search_results_json(&json).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_max_pages() {
        let json: serde_json::Value = serde_json::from_str(sample_initials_json()).unwrap();
        let max_pages = parse_max_pages(&json);
        assert_eq!(max_pages, Some(5));
    }

    #[test]
    fn test_extract_initials_json_from_html() {
        let html = r#"<script>window.initials={"searchResult":{"videoThumbProps":[]},"pagination":{"maxPages":0}};</script>"#;
        let json = extract_initials_json(html).unwrap();
        assert!(json.get("searchResult").is_some());
    }

    #[test]
    fn test_validate_filters_valid() {
        let filters = vec![
            SearchFilter {
                key: "quality".to_string(),
                value: "1080p".to_string(),
            },
            SearchFilter {
                key: "sort".to_string(),
                value: "newest".to_string(),
            },
        ];
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
    }

    #[test]
    fn test_validate_filters_invalid_value() {
        let filters = vec![SearchFilter {
            key: "quality".to_string(),
            value: "ultra".to_string(),
        }];
        let err = validate_search_filters(&filters).unwrap_err();
        assert!(err.to_string().contains("ultra"));
    }
}
