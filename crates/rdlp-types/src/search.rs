//! Search query and result types for site-specific search extractors.

use serde::{Deserialize, Serialize};

/// A search request with query text and optional filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// The free-text search term.
    pub query: String,
    /// Active filter key-value pairs.
    pub filters: Vec<SearchFilter>,
    /// Maximum number of results to return. `None` uses the site default / `MAX_PLAYLIST_SIZE` cap.
    pub max_results: Option<usize>,
    /// Specific page to fetch. None = fetch all pages (CLI behavior).
    pub page: Option<u32>,
}

/// A concrete filter key-value applied to a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilter {
    /// Filter key (e.g. "quality", "sort").
    pub key: String,
    /// Filter value (e.g. "1080p", "newest").
    pub value: String,
}

/// Describes a filter a site supports (for CLI help / UI construction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilterDescriptor {
    /// Machine-readable key used in `SearchFilter::key`.
    pub key: String,
    /// Human-readable name for display.
    pub display_name: String,
    /// Allowed values for this filter.
    pub allowed_values: Vec<SearchFilterValue>,
    /// Default value (if any).
    pub default: Option<String>,
}

/// A single selectable value inside a filter descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilterValue {
    /// Machine-readable value used in `SearchFilter::value`.
    pub value: String,
    /// Human-readable label.
    pub label: String,
}

/// A lightweight preview of a single search result.
///
/// Does NOT contain full format/download information. Callers that need
/// full metadata should call `InfoExtractor::extract()` on `video_url`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResultPreview {
    /// Direct URL to the video page (suitable for `find_extractor()` + `extract()`).
    pub video_url: String,
    /// Video title.
    pub title: String,
    /// Thumbnail image URL.
    pub thumbnail_url: Option<String>,
    /// Duration in seconds.
    pub duration: Option<f64>,
    /// Uploader / channel name.
    pub uploader: Option<String>,
    /// Uploader / channel page URL (absolute). Site-specific namespace —
    /// e.g. `PornHub` exposes `/model/<slug>`, `/channels/<slug>`, and
    /// `/pornstar/<slug>` under this field.
    #[serde(default)]
    pub uploader_url: Option<String>,
    /// Actors / performers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<String>,
    /// View count.
    pub view_count: Option<u64>,
    /// Upload date string (site-specific format).
    pub upload_date: Option<String>,
}

/// Information about a site that supports search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSiteInfo {
    /// Machine-readable name (e.g. "xhamster").
    pub name: String,
    /// Human-readable display name (e.g. "`XHamster`").
    pub display_name: String,
}

/// Response for a single search page (used for paginated frontends).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchPageResponse {
    /// Results on this page.
    pub results: Vec<SearchResultPreview>,
    /// The page number that was fetched.
    pub page: u32,
    /// Whether more pages exist after this one.
    pub has_more: bool,
    /// Optional estimate of total results across all pages.
    pub total_estimate: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_query_default() {
        let q = SearchQuery {
            query: "test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        assert_eq!(q.query, "test");
        assert!(q.filters.is_empty());
        assert_eq!(q.max_results, None);
        assert_eq!(q.page, None);
    }

    #[test]
    fn test_search_filter_descriptor_round_trip() {
        let desc = SearchFilterDescriptor {
            key: "quality".to_string(),
            display_name: "Minimum quality".to_string(),
            allowed_values: vec![SearchFilterValue {
                value: "720p".to_string(),
                label: "720p+".to_string(),
            }],
            default: None,
        };
        let json = serde_json::to_string(&desc).unwrap();
        let parsed: SearchFilterDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(desc, parsed);
    }

    #[test]
    fn test_search_result_preview_fields() {
        let preview = SearchResultPreview {
            video_url: "https://xhamster.com/videos/test-123".to_string(),
            title: "Test Video".to_string(),
            thumbnail_url: Some("https://thumb.jpg".to_string()),
            duration: Some(120.0),
            uploader: None,
            uploader_url: None,
            actors: vec![],
            view_count: Some(1000),
            upload_date: None,
        };
        assert_eq!(preview.title, "Test Video");
        assert_eq!(preview.duration, Some(120.0));
    }

    #[test]
    fn test_search_filter_equality() {
        let f1 = SearchFilter {
            key: "sort".to_string(),
            value: "newest".to_string(),
        };
        let f2 = SearchFilter {
            key: "sort".to_string(),
            value: "newest".to_string(),
        };
        assert_eq!(f1, f2);
    }
}
