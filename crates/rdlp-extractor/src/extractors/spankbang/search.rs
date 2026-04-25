//! Search result parsing and `SearchExtractor` implementation for SpankBang.
//!
//! URL shape: `https://spankbang.com/s/<query>/[<page>/]?o=<ordering>`
//! - Page is path-based (NOT query string); page 1 omits the segment
//! - Ordering is query-string: `featured` (default), `new`, `popular`
//! - Spaces in the query become `+`
//! - Cookie `country=US` matches the live extractor

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result, SearchExtractor};
use rdlp_types::{
    SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};

use super::SpankBangExtractor;
use super::patterns;
use crate::base::common::BaseExtractor;

const SPANKBANG_BASE_URL: &str = "https://spankbang.com";
const SPANKBANG_NAME_STR: &str = "SpankBang";

/// Approximate result-cards-per-page on a typical query. Used as a coarse
/// `total_estimate` for the paginated response shape; not authoritative.
const RESULTS_PER_PAGE: u64 = 36;

/// Hard cap on full-search collection.
const MAX_PLAYLIST_SIZE: usize = 500;

/// Delay between paginated requests (ms).
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Build the search URL for the given query and **0-indexed** external page.
///
/// External page `0` → no path segment (page 1 of results).
/// External page `1` → `/2/` (page 2), etc. — SpankBang's paths are 1-indexed.
pub(crate) fn build_search_url(query: &SearchQuery, page: u32) -> String {
    let kw: String = query
        .query
        .chars()
        .map(|c| if c == ' ' { '+' } else { c })
        .collect();

    let ordering = query
        .filters
        .iter()
        .find(|f| f.key == "ordering")
        .map(|f| f.value.as_str())
        .filter(|v| !v.is_empty());

    let path_page = if page == 0 {
        String::new()
    } else {
        format!("{}/", page + 1)
    };

    let qs = match ordering {
        Some(o) => format!("?o={o}"),
        None => String::new(),
    };

    format!("{SPANKBANG_BASE_URL}/s/{kw}/{path_page}{qs}")
}

/// Parse a SpankBang duration label (e.g. "3m", "1h23m", "45s") into seconds.
/// Returns `None` if the label is empty or unparseable.
pub(crate) fn parse_duration_label(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut current = 0u64;
    let mut consumed_any = false;
    for ch in s.chars() {
        match ch {
            '0'..='9' => {
                current = current.saturating_mul(10) + (ch as u64 - '0' as u64);
                consumed_any = true;
            }
            'h' | 'H' => {
                total = total.saturating_add(current.saturating_mul(3600));
                current = 0;
            }
            'm' | 'M' => {
                total = total.saturating_add(current.saturating_mul(60));
                current = 0;
            }
            's' | 'S' => {
                total = total.saturating_add(current);
                current = 0;
            }
            _ => {}
        }
    }
    // Bare digits with no unit aren't a known SpankBang shape — discard.
    if !consumed_any || total == 0 {
        return None;
    }
    Some(total as f64)
}

/// Parse a SpankBang view-count label ("940K", "1.3K", "1.5M", "12") into
/// an absolute count. Returns `None` for empty / unparseable input.
pub(crate) fn parse_view_count(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_part, multiplier) = match s.chars().last() {
        Some('K') | Some('k') => (&s[..s.len() - 1], 1_000.0_f64),
        Some('M') | Some('m') => (&s[..s.len() - 1], 1_000_000.0_f64),
        Some('B') | Some('b') => (&s[..s.len() - 1], 1_000_000_000.0_f64),
        Some(c) if c.is_ascii_digit() => (s, 1.0_f64),
        _ => return None,
    };
    let cleaned: String = num_part.chars().filter(|c| *c != ',').collect();
    cleaned.parse::<f64>().ok().map(|n| (n * multiplier) as u64)
}

/// Aggregate per-card extras keyed by video ID: thumbnail, duration, view
/// count, optional uploader (slug + display name).
#[derive(Default, Clone)]
struct CardExtras {
    thumbnail: Option<String>,
    duration: Option<f64>,
    view_count: Option<u64>,
    uploader_slug: Option<String>,
    uploader_name: Option<String>,
}

fn collect_card_metadata(html: &str) -> HashMap<String, CardExtras> {
    let mut map: HashMap<String, CardExtras> = HashMap::new();

    for caps in patterns::SEARCH_CARD_THUMB_DURATION.captures_iter(html) {
        let Some(id) = caps.get(1).map(|m| m.as_str().to_string()) else {
            continue;
        };
        let entry = map.entry(id).or_default();
        if entry.thumbnail.is_none() {
            entry.thumbnail = caps.get(2).map(|m| m.as_str().to_string());
        }
        if entry.duration.is_none() {
            entry.duration = caps.get(3).and_then(|m| parse_duration_label(m.as_str()));
        }
    }

    for caps in patterns::SEARCH_CARD_VIEWS.captures_iter(html) {
        let Some(id) = caps.get(2).map(|m| m.as_str().to_string()) else {
            continue;
        };
        let entry = map.entry(id).or_default();
        if entry.view_count.is_none() {
            entry.view_count = caps.get(1).and_then(|m| parse_view_count(m.as_str()));
        }
    }

    for caps in patterns::SEARCH_CARD_CHANNEL.captures_iter(html) {
        let Some(id) = caps.get(3).map(|m| m.as_str().to_string()) else {
            continue;
        };
        let entry = map.entry(id).or_default();
        if entry.uploader_slug.is_none() {
            entry.uploader_slug = caps.get(1).map(|m| m.as_str().to_string());
        }
        if entry.uploader_name.is_none() {
            entry.uploader_name = caps.get(2).map(|m| m.as_str().trim().to_string());
        }
    }

    map
}

/// Extract search-result anchors from a SpankBang search-page HTML.
///
/// Each video appears in two anchors per card (image wrapper + title link);
/// only the form carrying `title="..."` is captured by `SEARCH_RESULT`, which
/// already de-duplicates the image-wrapper form. We additionally de-duplicate
/// by video ID across the whole page since some pages echo the same video in
/// `recommended` / `editor's pick` rails alongside the main result grid.
///
/// Thumbnail and duration are joined in from the image-wrapper anchor via
/// [`collect_card_metadata`]. SpankBang search pages do not surface uploader
/// or view counts in the card markup; those remain `None` and require an
/// individual video-page fetch to populate.
pub(crate) fn parse_results(html: &str) -> Vec<SearchResultPreview> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut results = Vec::new();
    let card_meta = collect_card_metadata(html);

    for caps in patterns::SEARCH_RESULT.captures_iter(html) {
        let id = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let slug = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let title = caps.get(3).map(|m| m.as_str().trim()).unwrap_or_default();

        if id.is_empty() || slug.is_empty() {
            continue;
        }
        if !seen.insert(id.to_string()) {
            continue;
        }

        let extras = card_meta.get(id).cloned().unwrap_or_default();

        let video_url = format!("{SPANKBANG_BASE_URL}/{id}/video/{slug}");
        results.push(SearchResultPreview {
            video_url,
            title: title.to_string(),
            thumbnail_url: extras.thumbnail,
            duration: extras.duration,
            uploader: extras.uploader_name,
            actors: Vec::new(),
            view_count: extras.view_count,
            upload_date: None,
        });
    }

    results
}

/// Heuristic: a search page exposes a "next" page when the next page link
/// (`/<query>/<n+1>/`) is referenced anywhere in the rendered HTML, OR when
/// the result grid is "full" (≥ RESULTS_PER_PAGE entries on this page).
fn has_more_pages(html: &str, query: &SearchQuery, page: u32) -> bool {
    let next_path = {
        let kw: String = query
            .query
            .chars()
            .map(|c| if c == ' ' { '+' } else { c })
            .collect();
        format!("/s/{kw}/{}/", page + 2)
    };
    if html.contains(&next_path) {
        return true;
    }
    parse_results(html).len() as u64 >= RESULTS_PER_PAGE
}

#[async_trait]
impl SearchExtractor for SpankBangExtractor {
    fn name(&self) -> &str {
        SPANKBANG_NAME_STR
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "ordering".to_string(),
            display_name: "Ordering".to_string(),
            allowed_values: vec![
                SearchFilterValue {
                    value: "featured".to_string(),
                    label: "Featured (default)".to_string(),
                },
                SearchFilterValue {
                    value: "new".to_string(),
                    label: "Newest".to_string(),
                },
                SearchFilterValue {
                    value: "popular".to_string(),
                    label: "Most popular".to_string(),
                },
            ],
            default: Some("featured".to_string()),
        }]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();
        let mut page = 0_u32;

        loop {
            let page_url = build_search_url(query, page);
            let sanitized = rdlp_security::sanitize_for_logging(&page_url);
            debug!("[spankbang] fetching search page {}: {sanitized}", page + 1);

            let webpage = BaseExtractor::fetch_webpage_with_headers(
                &page_url,
                &[("Cookie", "country=US")],
                ctx,
            )
            .await?;

            let mut new_this_page = 0usize;
            for r in parse_results(&webpage) {
                let id_seg: String = r
                    .video_url
                    .trim_start_matches(SPANKBANG_BASE_URL)
                    .trim_start_matches('/')
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                if !seen_ids.insert(id_seg) {
                    continue;
                }
                all_results.push(r);
                new_this_page += 1;
                if all_results.len() >= max_results {
                    break;
                }
            }

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if new_this_page == 0 {
                break;
            }

            if !has_more_pages(&webpage, query, page) {
                break;
            }

            page += 1;
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        debug!(
            "[spankbang] search complete: {} results across {} page(s)",
            all_results.len(),
            page + 1
        );
        Ok(all_results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let page = query.page.unwrap_or(0);
        let page_url = build_search_url(query, page);

        let webpage = BaseExtractor::fetch_webpage_with_headers(
            &page_url,
            &[("Cookie", "country=US")],
            ctx,
        )
        .await?;

        let results = parse_results(&webpage);
        let more = has_more_pages(&webpage, query, page);

        Ok(SearchPageResponse {
            results,
            page,
            has_more: more,
            total_estimate: Some(RESULTS_PER_PAGE * 100),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    const SEARCH_PAGE: &str = include_str!("tests/spankbang_search_page.html");

    fn make_query(q: &str, filters: Vec<SearchFilter>) -> SearchQuery {
        SearchQuery {
            query: q.to_string(),
            filters,
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn url_composition_default_page1() {
        let q = make_query("blonde", vec![]);
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://spankbang.com/s/blonde/");
    }

    #[test]
    fn url_composition_page_2() {
        let q = make_query("blonde", vec![]);
        let url = build_search_url(&q, 1);
        assert_eq!(url, "https://spankbang.com/s/blonde/2/");
    }

    #[test]
    fn url_composition_with_ordering() {
        let q = make_query(
            "blonde",
            vec![SearchFilter {
                key: "ordering".to_string(),
                value: "new".to_string(),
            }],
        );
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://spankbang.com/s/blonde/?o=new");
    }

    #[test]
    fn url_composition_spaces_become_plus() {
        let q = make_query("two words", vec![]);
        let url = build_search_url(&q, 0);
        assert_eq!(url, "https://spankbang.com/s/two+words/");
    }

    #[test]
    fn parses_results_from_fixture() {
        let results = parse_results(SEARCH_PAGE);
        assert!(
            results.len() >= 30,
            "expected ≥ 30 deduped results from a full search page, got {}",
            results.len()
        );

        // Every URL must be on spankbang.com and follow the /<id>/video/<slug> shape.
        for r in &results {
            assert!(
                r.video_url.starts_with("https://spankbang.com/"),
                "unexpected URL: {}",
                r.video_url
            );
            assert!(
                r.video_url.contains("/video/"),
                "URL missing /video/ segment: {}",
                r.video_url
            );
            assert!(!r.title.is_empty(), "result has empty title");
        }

        // De-duplication holds: every video ID appears at most once.
        let mut ids = HashSet::new();
        for r in &results {
            let id = r
                .video_url
                .trim_start_matches("https://spankbang.com/")
                .split('/')
                .next()
                .unwrap_or("")
                .to_string();
            assert!(ids.insert(id.clone()), "duplicate id in results: {id}");
        }
    }

    #[test]
    fn parses_thumbnail_and_duration_from_fixture() {
        let results = parse_results(SEARCH_PAGE);
        // Most cards in the SpankBang search grid expose both fields; require
        // a strong majority rather than 100% (recommendation rails or
        // ad-marked cards may render without the wrapper anchor we key off).
        let with_thumb = results.iter().filter(|r| r.thumbnail_url.is_some()).count();
        let with_dur = results.iter().filter(|r| r.duration.is_some()).count();
        assert!(
            with_thumb >= results.len() * 3 / 4,
            "expected ≥75% of {} results to carry a thumbnail; got {with_thumb}",
            results.len()
        );
        assert!(
            with_dur >= results.len() * 3 / 4,
            "expected ≥75% of {} results to carry a duration; got {with_dur}",
            results.len()
        );

        // Spot-check: at least one result has an sb-cd.com thumbnail URL and a
        // positive duration.
        let sample = results
            .iter()
            .find(|r| r.thumbnail_url.is_some() && r.duration.is_some())
            .expect("at least one result with both fields");
        let thumb = sample.thumbnail_url.as_deref().unwrap();
        assert!(
            thumb.starts_with("https://") && thumb.contains("sb-cd.com"),
            "unexpected thumbnail host: {thumb}"
        );
        assert!(
            sample.duration.unwrap() > 0.0,
            "duration must be positive"
        );
    }

    #[test]
    fn parses_view_count_and_uploader_when_present_in_fixture() {
        let results = parse_results(SEARCH_PAGE);

        // View count is rendered for every card in the live grid; require a
        // strong majority (some recommendation rails / ad-tagged cards may
        // skip the views span).
        let with_views = results.iter().filter(|r| r.view_count.is_some()).count();
        assert!(
            with_views >= results.len() * 3 / 4,
            "expected ≥75% of {} results to carry a view count; got {with_views}",
            results.len()
        );

        // Uploader is only present on cards whose primary badge is a channel
        // (vs a tag); not every card will have one. Just assert at least one
        // card surfaces the uploader so the join path is exercised.
        let with_uploader = results
            .iter()
            .filter(|r| r.uploader.is_some())
            .count();
        assert!(
            with_uploader >= 1,
            "expected at least one channel-tagged result to carry an uploader; got {with_uploader}"
        );

        // Spot-check a card that has all three enriched fields.
        let sample = results
            .iter()
            .find(|r| r.view_count.is_some() && r.duration.is_some())
            .expect("at least one result with view count + duration");
        assert!(sample.view_count.unwrap() > 0);
    }

    #[test]
    fn view_count_parser_handles_known_shapes() {
        assert_eq!(parse_view_count("940K"), Some(940_000));
        assert_eq!(parse_view_count("1.3K"), Some(1_300));
        assert_eq!(parse_view_count("1.5M"), Some(1_500_000));
        assert_eq!(parse_view_count("4K"), Some(4_000));
        assert_eq!(parse_view_count("12"), Some(12));
        assert_eq!(parse_view_count("1,234"), Some(1_234));
        assert_eq!(parse_view_count("1.2B"), Some(1_200_000_000));
        // Empty / unparseable
        assert_eq!(parse_view_count(""), None);
        assert_eq!(parse_view_count("   "), None);
        assert_eq!(parse_view_count("nope"), None);
    }

    #[test]
    fn duration_label_parser_handles_known_shapes() {
        assert_eq!(parse_duration_label("3m"), Some(180.0));
        assert_eq!(parse_duration_label("10m"), Some(600.0));
        assert_eq!(parse_duration_label("45s"), Some(45.0));
        assert_eq!(parse_duration_label("1h23m"), Some(3600.0 + 23.0 * 60.0));
        assert_eq!(parse_duration_label("2h"), Some(7200.0));
        assert_eq!(parse_duration_label("1h2m3s"), Some(3600.0 + 120.0 + 3.0));
        // Unparseable / empty / unitless → None
        assert_eq!(parse_duration_label(""), None);
        assert_eq!(parse_duration_label("   "), None);
        assert_eq!(parse_duration_label("123"), None);
        assert_eq!(parse_duration_label("nope"), None);
    }

    #[test]
    fn supported_filters_advertises_ordering() {
        let ext = SpankBangExtractor::new();
        let filters = ext.supported_filters();
        assert_eq!(filters.len(), 1);
        let ordering = &filters[0];
        assert_eq!(ordering.key, "ordering");
        assert_eq!(ordering.default.as_deref(), Some("featured"));
        let labels: Vec<&str> = ordering
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(labels.contains(&"featured"));
        assert!(labels.contains(&"new"));
        assert!(labels.contains(&"popular"));
    }

    #[test]
    fn name_matches_info_extractor() {
        // Search and InfoExtractor names must agree so the registry's
        // case-insensitive lookup routes correctly.
        let ext = SpankBangExtractor::new();
        assert_eq!(SearchExtractor::name(&ext), "SpankBang");
    }
}
