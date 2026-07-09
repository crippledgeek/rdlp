//! Shared search-URL construction helpers for API-based search extractors.
//!
//! The *base* search URL (host, fixed query params, `search=` position) is
//! per-site knowledge and stays in each extractor. What is genuinely shared —
//! the same knowledge, changing together across sites — is how the standard
//! filter set is appended to an API search URL. Sites whose API accepts the
//! `ordering` / `period` / `category` / `tags[]` filter vocabulary (PornHub,
//! RedTube) delegate that appending here.

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, Result};
use rdlp_types::{SearchFilter, SearchQuery, SearchResultPreview};
use std::time::Duration;
use url::form_urlencoded;

use super::MAX_PLAYLIST_SIZE;

/// Delay between successive search-page fetches. All API-paginated sites that
/// share the [`PaginatedSearch`] scaffold rate-limit at this interval.
pub(crate) const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Append the standard API search filters to an in-progress search URL.
///
/// Recognizes four filter keys; unknown keys are silently ignored (value
/// validation happens at the CLI/descriptor boundary, not here):
/// - `ordering`, `period` — appended verbatim as `&{key}={value}`.
/// - `category` — appended as `&category={encoded}` (form-urlencoded).
/// - `tags` — comma-separated; each non-empty, trimmed tag is appended as a
///   separate `&tags[]={encoded}` pair.
///
/// The URL must already contain a `?` and its base query params; this only
/// appends `&`-prefixed pairs.
pub(crate) fn append_search_filters(url: &mut String, filters: &[SearchFilter]) {
    for filter in filters {
        match filter.key.as_str() {
            "ordering" => {
                url.push_str("&ordering=");
                url.push_str(&filter.value);
            }
            "period" => {
                url.push_str("&period=");
                url.push_str(&filter.value);
            }
            "category" => {
                let encoded: String =
                    form_urlencoded::byte_serialize(filter.value.as_bytes()).collect();
                url.push_str("&category=");
                url.push_str(&encoded);
            }
            "tags" => {
                for tag in filter.value.split(',') {
                    let trimmed = tag.trim();
                    if !trimmed.is_empty() {
                        let encoded: String =
                            form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();
                        url.push_str("&tags[]=");
                        url.push_str(&encoded);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Shared multi-page search pagination for API-style search extractors.
///
/// Sites whose search fetches page N and learns the total page count from the
/// same response (XHamster, TNAFlix, EMPFlix, MovieFap) share one pagination
/// loop: fetch pages in order, accumulate previews, and stop at the first of —
/// `max_results` reached, `max_pages` reached, an empty page, or a fetch error
/// (returning the partial results gathered so far). Implementors supply only
/// the per-site pieces (a single-page fetch, a log tag, filter validation);
/// [`search_all_pages`](Self::search_all_pages) is the shared default and
/// should not be overridden.
///
/// Sites with a different pagination shape (e.g. PornHub/RedTube, which chain
/// an HTML-vs-API fallback per page) intentionally do NOT implement this trait.
#[async_trait]
pub(crate) trait PaginatedSearch: Send + Sync {
    /// Bracketed site tag used in log lines, e.g. `"[XHamster]"`.
    fn search_log_tag(&self) -> &'static str;

    /// Validate the query's filters against this site's supported filter set.
    fn validate_search_filters(&self, filters: &[SearchFilter]) -> Result<()>;

    /// Fetch a single search page, returning `(results, max_pages)`.
    async fn fetch_search_page(
        &self,
        query: &SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<SearchResultPreview>, usize)>;

    /// Delay between successive page fetches. Defaults to [`PAGE_RATE_LIMIT_MS`].
    fn page_rate_limit(&self) -> Duration {
        Duration::from_millis(PAGE_RATE_LIMIT_MS)
    }

    /// Collect results across pages until `max_results` / `max_pages` / an empty
    /// page / a fetch error. Shared scaffold — do not override.
    async fn search_all_pages(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        self.validate_search_filters(&query.filters)?;

        let tag = self.search_log_tag();
        let max_results = query.max_results.unwrap_or(MAX_PLAYLIST_SIZE);
        let mut all_results = Vec::new();
        let mut page = 1usize;

        loop {
            let (page_results, max_pages) = match self.fetch_search_page(query, page, ctx).await {
                Ok(result) => result,
                Err(e) => {
                    debug!(page; "{tag} Failed to fetch search page, returning partial results: {e}");
                    break;
                }
            };

            if page_results.is_empty() {
                debug!(page; "{tag} No results on page, stopping pagination");
                break;
            }

            all_results.extend(page_results);

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if page >= max_pages {
                break;
            }

            page += 1;
            tokio::time::sleep(self.page_rate_limit()).await;
        }

        debug!(count = all_results.len(), pages = page; "{tag} Search complete");

        Ok(all_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(key: &str, value: &str) -> SearchFilter {
        SearchFilter {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn append(filters: &[SearchFilter]) -> String {
        let mut url = String::from("https://x.test/?output=json");
        append_search_filters(&mut url, filters);
        url
    }

    #[test]
    fn no_filters_is_noop() {
        assert_eq!(append(&[]), "https://x.test/?output=json");
    }

    #[test]
    fn ordering_and_period_appended_verbatim() {
        let out = append(&[filter("ordering", "newest"), filter("period", "weekly")]);
        assert!(out.ends_with("&ordering=newest&period=weekly"), "got {out}");
    }

    #[test]
    fn category_is_url_encoded() {
        let out = append(&[filter("category", "big ass")]);
        assert!(out.contains("&category=big+ass"), "got {out}");
    }

    #[test]
    fn empty_category_value_appends_empty() {
        let out = append(&[filter("category", "")]);
        assert!(out.ends_with("&category="), "got {out}");
    }

    #[test]
    fn ordering_and_period_are_not_encoded() {
        // Asymmetry lock: ordering/period append VERBATIM (values are
        // enum-constrained upstream), unlike category/tags which are
        // form-urlencoded. A future "helpful" encode of these would be a
        // behavior change — this pins it.
        let out = append(&[filter("ordering", "a b&c"), filter("period", "x y")]);
        assert!(out.contains("&ordering=a b&c"), "got {out}");
        assert!(out.contains("&period=x y"), "got {out}");
    }

    #[test]
    fn tags_split_trim_encode_each() {
        let out = append(&[filter("tags", " red head , milf ")]);
        assert!(out.contains("&tags[]=red+head"), "got {out}");
        assert!(out.contains("&tags[]=milf"), "got {out}");
    }

    #[test]
    fn empty_tag_entries_are_skipped() {
        // Leading/trailing/double commas must not emit empty tags[] pairs.
        let out = append(&[filter("tags", ",a,,b,")]);
        assert_eq!(out.matches("&tags[]=").count(), 2, "got {out}");
        assert!(
            out.contains("&tags[]=a") && out.contains("&tags[]=b"),
            "got {out}"
        );
    }

    #[test]
    fn unknown_filter_key_is_ignored() {
        let out = append(&[filter("bogus", "value"), filter("ordering", "top")]);
        assert!(!out.contains("bogus"), "got {out}");
        assert!(out.ends_with("&ordering=top"), "got {out}");
    }

    // ---- PaginatedSearch scaffold ----

    use crate::hls::test_support::test_ctx;
    use rdlp_core::RdlpError;
    use rdlp_types::SearchQuery;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn preview(i: usize) -> SearchResultPreview {
        SearchResultPreview {
            video_url: format!("https://x.test/v{i}"),
            title: format!("v{i}"),
            thumbnail_url: None,
            duration: None,
            uploader: None,
            uploader_url: None,
            actors: Vec::new(),
            view_count: None,
            upload_date: None,
        }
    }

    fn previews(n: usize) -> Vec<SearchResultPreview> {
        (0..n).map(preview).collect()
    }

    enum Page {
        Ok(Vec<SearchResultPreview>, usize),
        Fail,
    }

    struct MockSearch {
        script: Vec<Page>,
        validation_fails: bool,
        fetches: AtomicUsize,
    }

    impl MockSearch {
        fn new(script: Vec<Page>) -> Self {
            Self {
                script,
                validation_fails: false,
                fetches: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl PaginatedSearch for MockSearch {
        fn search_log_tag(&self) -> &'static str {
            "[Mock]"
        }
        fn validate_search_filters(&self, _filters: &[SearchFilter]) -> Result<()> {
            if self.validation_fails {
                Err(RdlpError::extraction(
                    "mock validation failure",
                    "https://x.test",
                ))
            } else {
                Ok(())
            }
        }
        // No real sleeps in tests.
        fn page_rate_limit(&self) -> Duration {
            Duration::ZERO
        }
        async fn fetch_search_page(
            &self,
            _query: &SearchQuery,
            page: usize,
            _ctx: &ExtractionContext,
        ) -> Result<(Vec<SearchResultPreview>, usize)> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            match self.script.get(page - 1) {
                Some(Page::Ok(results, max_pages)) => Ok((results.clone(), *max_pages)),
                Some(Page::Fail) => Err(RdlpError::extraction(
                    "mock fetch failure",
                    "https://x.test",
                )),
                None => Ok((Vec::new(), page)), // past the script → empty page
            }
        }
    }

    fn query(max_results: Option<usize>) -> SearchQuery {
        SearchQuery {
            query: "q".to_string(),
            filters: Vec::new(),
            max_results,
            page: None,
        }
    }

    async fn run(script: Vec<Page>, max_results: Option<usize>) -> Vec<SearchResultPreview> {
        MockSearch::new(script)
            .search_all_pages(&query(max_results), &test_ctx())
            .await
            .expect("scaffold returns partial results, never errors")
    }

    #[tokio::test]
    async fn accumulates_across_pages_then_truncates_to_max_results() {
        // page1: 3, page2: 3, cap 4 → extend to 6, truncate to 4.
        let out = run(
            vec![Page::Ok(previews(3), 10), Page::Ok(previews(3), 10)],
            Some(4),
        )
        .await;
        assert_eq!(out.len(), 4);
    }

    #[tokio::test]
    async fn stops_at_max_pages_without_fetching_beyond() {
        // page1 reports max_pages=1; page2 (if fetched) would add 5 — must not.
        let out = run(
            vec![Page::Ok(previews(2), 1), Page::Ok(previews(5), 1)],
            None,
        )
        .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn stops_on_empty_page() {
        let out = run(
            vec![Page::Ok(previews(2), 10), Page::Ok(Vec::new(), 10)],
            None,
        )
        .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn fetch_error_returns_partial_results() {
        let out = run(vec![Page::Ok(previews(2), 10), Page::Fail], None).await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn truncates_within_a_single_page() {
        let out = run(vec![Page::Ok(previews(5), 10)], Some(3)).await;
        assert_eq!(out.len(), 3);
    }

    #[tokio::test]
    async fn accumulates_all_pages_when_uncapped() {
        // 2 pages, no result cap; max_pages=2 stops the loop naturally → all 5,
        // no truncation (the one path the other multi-page tests don't cover).
        let out = run(
            vec![Page::Ok(previews(3), 2), Page::Ok(previews(2), 2)],
            None,
        )
        .await;
        assert_eq!(out.len(), 5);
    }

    #[tokio::test]
    async fn validation_error_returns_err_without_fetching() {
        // The only scaffold path that returns Err (not partial results): a
        // failed filter validation must short-circuit BEFORE any page fetch.
        let mock = MockSearch {
            script: vec![Page::Ok(previews(3), 10)],
            validation_fails: true,
            fetches: AtomicUsize::new(0),
        };
        let result = mock.search_all_pages(&query(None), &test_ctx()).await;
        assert!(result.is_err(), "validation failure must propagate as Err");
        assert_eq!(
            mock.fetches.load(Ordering::SeqCst),
            0,
            "must not fetch any page when validation fails"
        );
    }
}
