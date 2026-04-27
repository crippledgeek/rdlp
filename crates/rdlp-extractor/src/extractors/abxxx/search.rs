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
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilter, SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};
use serde_json::Value;

use super::AbxxxExtractor;
use crate::base::common::BaseExtractor;

const ABXXX_BASE_URL: &str = "https://abxxx.com";
const SEARCH_REFERER: &str = "https://abxxx.com/";
const RESULTS_PER_PAGE: u32 = 60;
const MAX_PLAYLIST_SIZE: usize = 500;

/// Allowed values for the `sort` filter. Mirrors the sort modes the live site
/// itself sends to `/api/videos2.php`.
const SORT_VALUES: &[(&str, &str)] = &[
    ("relevance", "Relevance"),
    ("latest-updates", "Latest"),
    ("most-popular", "Most Popular"),
    ("top-rated", "Top Rated"),
];

/// Build the search URL for one page.
fn build_search_url(query: &str, sort: &str, page: u32) -> String {
    let q = urlencoding::encode(query);
    format!(
        "{ABXXX_BASE_URL}/api/videos2.php?params=0/str/{sort}/{count}/search..{page}.all..&s={q}",
        sort = sort,
        count = RESULTS_PER_PAGE,
        page = page,
        q = q,
    )
}

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

/// Parse a `MM:SS` or `HH:MM:SS` duration string into seconds.
fn parse_duration(s: &str) -> Option<f64> {
    let parts: Vec<u64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [s] => Some(*s as f64),
        [m, s] => Some((m * 60 + s) as f64),
        [h, m, s] => Some((h * 3600 + m * 60 + s) as f64),
        _ => None,
    }
}

/// Convert one `videos[]` JSON entry into a `SearchResultPreview`.
fn entry_to_preview(entry: &Value) -> Option<SearchResultPreview> {
    let video_id = entry.get("video_id").and_then(|v| v.as_str())?;
    let dir = entry.get("dir").and_then(|v| v.as_str()).unwrap_or("");
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
        .and_then(parse_duration);

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
        actors,
        view_count,
        upload_date,
    })
}

/// Parse the full JSON response into previews + pagination metadata.
fn parse_response(body: &str) -> Result<(Vec<SearchResultPreview>, Option<u64>, u32)> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| rdlp_core::RdlpError::Extraction {
            message: format!(
                "ABXXX search returned non-JSON ({e}): {}",
                &body[..body.len().min(200)]
            ),
            url: None,
        })?;

    let videos = parsed
        .get("videos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let previews: Vec<SearchResultPreview> = videos.iter().filter_map(entry_to_preview).collect();

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
        let mut last_max_page: u32 = 1;

        while out.len() < cap {
            let url = build_search_url(&query.query, sort, page);
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
            last_max_page = max_page;
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
        let _ = last_max_page;
        Ok(out)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let page = query.page.unwrap_or(1).max(1);
        let sort = resolved_sort(&query.filters);
        let url = build_search_url(&query.query, sort, page);
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
    fn build_url_encodes_query_and_page() {
        let url = build_search_url("katie carmine", "relevance", 1);
        assert_eq!(
            url,
            "https://abxxx.com/api/videos2.php?params=0/str/relevance/60/search..1.all..&s=katie%20carmine"
        );
    }

    #[test]
    fn build_url_paginates() {
        let url = build_search_url("test", "latest-updates", 5);
        assert!(url.contains("/latest-updates/"));
        assert!(url.contains("search..5.all.."));
    }

    #[test]
    fn parse_duration_handles_mm_ss_and_hh_mm_ss() {
        assert_eq!(parse_duration("06:15"), Some(375.0));
        assert_eq!(parse_duration("55:20"), Some(3320.0));
        assert_eq!(parse_duration("1:05:42"), Some(3942.0));
        assert_eq!(parse_duration(""), None);
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
