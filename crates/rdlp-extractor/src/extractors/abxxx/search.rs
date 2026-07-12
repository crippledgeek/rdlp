//! Search support for ABXXX.
//!
//! ABXXX exposes search through `/api/videos2.php?params={schema}&s={query}`
//! where the `params` blob is a slash-separated string the site's `videos()`
//! helper builds in `app.js`:
//!
//! ```text
//! {lifetime}/{gender}/{sort}/{count}/{section}.{object_id}.{page}.{type}.{duration}.{date}
//! ```
//!
//! For search, `section` is `search` and `sort` is one of `relevance`,
//! `latest-updates`, `most-popular`, `top-rated`. The JSON response includes
//! a `videos[]` array with full metadata (title, duration `MM:SS`, view count,
//! thumbnail, models, post date, …) so no per-result page fetch is needed.

use async_trait::async_trait;
use log::{debug, warn};
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilter, SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};
use serde_json::Value;

use super::{ABXXX_BASE_URL, AbxxxExtractor};
use crate::base::common::BaseExtractor;
use crate::base::kvs::api as kvs_api;

const SEARCH_REFERER: &str = "https://abxxx.com/";
const MAX_PLAYLIST_SIZE: usize = 500;

/// Allowed values for the `sort` filter. Mirrors the sort modes the live site
/// itself sends to `/api/videos2.php`.
const SORT_VALUES: &[(&str, &str)] = &[
    ("relevance", "Relevance"),
    ("latest-updates", "Latest"),
    ("most-popular", "Most Popular"),
    ("top-rated", "Top Rated"),
];

/// Read the `sort` filter from the query, validating against `SORT_VALUES`.
fn resolved_sort(filters: &[SearchFilter]) -> &'static str {
    for f in filters {
        if f.key == "sort" {
            for (v, _) in SORT_VALUES {
                if *v == f.value {
                    return v;
                }
            }
        }
    }
    "relevance"
}

/// Return `true` if `s` is safe to embed as a URL path segment: only
/// alphanumeric characters, hyphens, and underscores are allowed.
///
/// This guards against path-traversal injections such as `../../evil` that a
/// malicious or compromised API response could smuggle in via `video_id` or
/// `dir` fields.
fn is_safe_path_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Convert one `videos[]` JSON entry into a `SearchResultPreview`.
///
/// Returns `None` (and logs a warning) when `video_id` or `dir` contain
/// characters that are not safe to embed as URL path segments.
fn entry_to_preview(entry: &Value) -> Option<SearchResultPreview> {
    let video_id = entry.get("video_id").and_then(|v| v.as_str())?;
    let dir = entry.get("dir").and_then(|v| v.as_str()).unwrap_or("");

    // Validate path components to prevent path-traversal via API response (M3).
    if !is_safe_path_segment(video_id) {
        warn!(
            "[ABXXX] dropping entry: video_id contains unsafe characters: {:?}",
            video_id
        );
        return None;
    }
    if !dir.is_empty() && !is_safe_path_segment(dir) {
        warn!(
            "[ABXXX] dropping entry: dir contains unsafe characters: {:?}",
            dir
        );
        return None;
    }

    let title = entry
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let video_url = if dir.is_empty() {
        format!("{ABXXX_BASE_URL}/video/{video_id}/")
    } else {
        format!("{ABXXX_BASE_URL}/video/{video_id}/{dir}/")
    };

    let thumbnail_url = entry
        .get("scr")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let duration = entry
        .get("duration")
        .and_then(|v| v.as_str())
        .and_then(kvs_api::parse_kvs_duration);

    let actors: Vec<String> = entry
        .get("models")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let uploader = entry
        .get("display_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            entry
                .get("content_source_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });

    let view_count = entry
        .get("video_viewed")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());

    let upload_date = entry
        .get("post_date")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Some(SearchResultPreview {
        video_url,
        title,
        thumbnail_url,
        duration,
        uploader,
        uploader_url: None,
        actors,
        view_count,
        upload_date,
    })
}

/// Truncate a response body to the first N chars (not bytes) for safe
/// inclusion in error messages — slicing on a byte boundary would panic
/// on multi-byte UTF-8.
fn truncate_chars(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map(|(i, _)| &s[..i]).unwrap_or(s)
}

/// Parse the full JSON response into previews + pagination metadata.
fn parse_response(body: &str) -> Result<(Vec<SearchResultPreview>, Option<u64>, u32)> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| rdlp_core::RdlpError::Extraction {
            message: format!(
                "ABXXX search returned non-JSON ({e}): {}",
                truncate_chars(body, 200)
            ),
            url: None,
        })?;

    let previews: Vec<SearchResultPreview> = parsed
        .get("videos")
        .and_then(|v| v.as_array())
        .map(|videos| videos.iter().filter_map(entry_to_preview).collect())
        .unwrap_or_default();

    let total: Option<u64> = parsed
        .get("total_count")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .or_else(|| parsed.get("total_count").and_then(|v| v.as_u64()));
    let max_page: u32 = parsed
        .get("pages")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(1);

    Ok((previews, total, max_page))
}

#[async_trait]
impl SearchExtractor for AbxxxExtractor {
    fn name(&self) -> &str {
        "ABXXX"
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "sort".to_string(),
            display_name: "Sort by".to_string(),
            allowed_values: SORT_VALUES
                .iter()
                .map(|(v, l)| SearchFilterValue {
                    value: (*v).to_string(),
                    label: (*l).to_string(),
                })
                .collect(),
            default: Some("relevance".to_string()),
        }]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let cap = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let sort = resolved_sort(&query.filters);
        let mut out: Vec<SearchResultPreview> = Vec::new();
        let mut page: u32 = 1;

        while out.len() < cap {
            let url = kvs_api::videos2_search_endpoint(
                ABXXX_BASE_URL,
                &query.query,
                sort,
                page,
                kvs_api::KVS_VIDEOS2_DEFAULT_PAGE_SIZE,
            );
            debug!(
                "[ABXXX] search page {page}: {}",
                rdlp_security::sanitize_for_logging(&url)
            );
            let body = BaseExtractor::fetch_webpage_with_headers(
                &url,
                &[
                    ("X-Requested-With", "XMLHttpRequest"),
                    ("Accept", "application/json"),
                    ("Referer", SEARCH_REFERER),
                ],
                ctx,
            )
            .await?;

            let (mut previews, _total, max_page) = parse_response(&body)?;
            if previews.is_empty() {
                break;
            }
            out.append(&mut previews);
            if page >= max_page {
                break;
            }
            page += 1;
        }

        out.truncate(cap);
        Ok(out)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        // Bespoke: the `.max(1)` clamp is reflected in the returned SearchPageResponse.page,
        // which run_search_page cannot reproduce (it drives page from an unclamped unwrap_or). See #450.
        let page = query.page.unwrap_or(1).max(1);
        let sort = resolved_sort(&query.filters);
        let url = kvs_api::videos2_search_endpoint(
            ABXXX_BASE_URL,
            &query.query,
            sort,
            page,
            kvs_api::KVS_VIDEOS2_DEFAULT_PAGE_SIZE,
        );
        debug!(
            "[ABXXX] search_page page {page}: {}",
            rdlp_security::sanitize_for_logging(&url)
        );
        let body = BaseExtractor::fetch_webpage_with_headers(
            &url,
            &[
                ("X-Requested-With", "XMLHttpRequest"),
                ("Accept", "application/json"),
                ("Referer", SEARCH_REFERER),
            ],
            ctx,
        )
        .await?;
        let (results, total_estimate, max_page) = parse_response(&body)?;
        Ok(SearchPageResponse {
            results,
            page,
            has_more: page < max_page,
            total_estimate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_respects_utf8_boundaries() {
        // Latin: byte-len equals char-len, ordinary truncation
        assert_eq!(truncate_chars("hello world", 5), "hello");
        // Multi-byte: each char is 2 bytes; truncating at 3 chars gives 6 bytes,
        // which would have been a UTF-8-boundary panic with byte-index slicing.
        assert_eq!(truncate_chars("ÅÄÖÅÄÖ", 3), "ÅÄÖ");
        // n larger than length: returns the whole string
        assert_eq!(truncate_chars("hi", 10), "hi");
    }

    #[test]
    fn resolved_sort_falls_back_to_relevance() {
        assert_eq!(resolved_sort(&[]), "relevance");
        assert_eq!(
            resolved_sort(&[SearchFilter {
                key: "sort".to_string(),
                value: "garbage".to_string()
            }]),
            "relevance"
        );
        assert_eq!(
            resolved_sort(&[SearchFilter {
                key: "sort".to_string(),
                value: "top-rated".to_string()
            }]),
            "top-rated"
        );
    }

    #[test]
    fn entry_to_preview_extracts_known_fields() {
        let entry: Value = serde_json::json!({
            "video_id": "157044",
            "dir": "katie-gets-kinky",
            "title": "Katie gets kinky",
            "duration": "06:15",
            "video_viewed": "2896",
            "post_date": "2023-08-28 01:17:16",
            "scr": "https://ii.abxxx.com/contents/videos_screenshots/157000/157044/480x270/4.jpg",
            "models": "Katie,Kush",
            "display_name": "Hairy Girls Videos"
        });
        let p = entry_to_preview(&entry).expect("preview built");
        assert_eq!(
            p.video_url,
            "https://abxxx.com/video/157044/katie-gets-kinky/"
        );
        assert_eq!(p.title, "Katie gets kinky");
        assert_eq!(p.duration, Some(375.0));
        assert_eq!(p.view_count, Some(2896));
        assert_eq!(p.actors, vec!["Katie".to_string(), "Kush".to_string()]);
        assert_eq!(p.uploader.as_deref(), Some("Hairy Girls Videos"));
        assert!(p.thumbnail_url.is_some());
    }

    #[test]
    fn entry_to_preview_skips_entry_without_id() {
        let entry: Value = serde_json::json!({"title": "no id"});
        assert!(entry_to_preview(&entry).is_none());
    }

    /// Regression guard for M3: API path components containing path-traversal
    /// sequences must be rejected so a malicious API response cannot construct
    /// URLs like `https://abxxx.com/video/1/../../evil/`.
    ///
    /// Before the fix `dir` and `video_id` were interpolated directly into the
    /// URL without validation.
    #[test]
    fn entry_to_preview_rejects_path_traversal_in_dir() {
        let entry: Value = serde_json::json!({
            "video_id": "157044",
            "dir": "../../evil",
            "title": "Malicious"
        });
        assert!(
            entry_to_preview(&entry).is_none(),
            "entry with path-traversal dir must be dropped"
        );
    }

    #[test]
    fn entry_to_preview_rejects_path_traversal_in_video_id() {
        let entry: Value = serde_json::json!({
            "video_id": "../admin",
            "dir": "",
            "title": "Malicious"
        });
        assert!(
            entry_to_preview(&entry).is_none(),
            "entry with path-traversal video_id must be dropped"
        );
    }

    #[test]
    fn entry_to_preview_accepts_valid_segments() {
        let entry: Value = serde_json::json!({
            "video_id": "157044",
            "dir": "katie-gets-kinky",
            "title": "Katie gets kinky"
        });
        assert!(
            entry_to_preview(&entry).is_some(),
            "entry with valid path segments must be kept"
        );
    }

    #[test]
    fn is_safe_path_segment_rejects_traversal() {
        assert!(!is_safe_path_segment("../../evil"));
        assert!(!is_safe_path_segment("../foo"));
        assert!(!is_safe_path_segment("foo/bar"));
        assert!(!is_safe_path_segment("foo bar"));
        assert!(!is_safe_path_segment(""));
    }

    #[test]
    fn is_safe_path_segment_accepts_valid() {
        assert!(is_safe_path_segment("157044"));
        assert!(is_safe_path_segment("katie-gets-kinky"));
        assert!(is_safe_path_segment("some_video"));
        assert!(is_safe_path_segment("abc123"));
    }

    /// Regression guard: parse_response must drop entries with unsafe path
    /// components but keep valid ones in the same array.
    #[test]
    fn parse_response_drops_unsafe_entries() {
        let body = serde_json::json!({
            "total_count": "3",
            "pages": 1,
            "videos": [
                {"video_id": "1", "dir": "good-dir", "title": "Good"},
                {"video_id": "2", "dir": "../../evil", "title": "Bad dir"},
                {"video_id": "../admin", "dir": "", "title": "Bad id"}
            ]
        })
        .to_string();
        let (previews, _total, _max) = parse_response(&body).expect("parse ok");
        assert_eq!(previews.len(), 1, "only the safe entry should survive");
        assert_eq!(previews[0].title, "Good");
    }

    #[test]
    fn parse_response_extracts_videos_and_pagination() {
        let body = serde_json::json!({
            "total_count": "1844",
            "pages": 31,
            "videos": [
                {"video_id": "1", "dir": "a", "title": "T1", "duration": "1:00"},
                {"video_id": "2", "dir": "b", "title": "T2", "duration": "2:00"}
            ]
        })
        .to_string();
        let (previews, total, max) = parse_response(&body).expect("parse ok");
        assert_eq!(previews.len(), 2);
        assert_eq!(total, Some(1844));
        assert_eq!(max, 31);
    }

    #[test]
    fn parse_response_handles_empty_videos_array() {
        let body = serde_json::json!({"total_count": 0, "pages": 1, "videos": []}).to_string();
        let (previews, _total, _max) = parse_response(&body).expect("parse ok");
        assert!(previews.is_empty());
    }

    #[test]
    fn supported_filters_lists_sort() {
        let f = AbxxxExtractor::new().supported_filters();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].key, "sort");
        assert_eq!(f[0].default.as_deref(), Some("relevance"));
        assert!(f[0].allowed_values.iter().any(|v| v.value == "top-rated"));
    }
}
