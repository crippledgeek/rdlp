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
use rdlp_types::{SearchFilter, SearchFilterDescriptor, SearchQuery, SearchResultPreview};
use std::time::Duration;
use url::form_urlencoded;

use super::MAX_PLAYLIST_SIZE;

/// How one filter key's value is validated by [`validate_against_descriptors`].
///
/// `#[allow(dead_code)]`: only exercised by `validator_tests` until Task 5/6
/// of the search-filter-dedup sprint migrate per-site validators to call
/// `validate_against_descriptors` — not a speculative future-use allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum KeyValidation {
    /// Value must be one of the descriptor's `allowed_values` (the default).
    AllowedValues,
    /// Any value accepted (the site's API validates server-side); skip the check.
    FreeText,
    /// Value must parse as `u32`.
    NumericU32,
}

/// A filter-validation failure — data only, no baked-in wording. Each call site
/// maps this to its own exact `RdlpError` message (producer returns data,
/// consumer formats). `NonNumeric` is distinct from `InvalidValue` because the
/// numeric path's message ("Must be a number.") differs from the allowed-values
/// message ("Allowed: …").
/// `#[allow(dead_code)]`: same rationale as [`KeyValidation`] — consumed by
/// Task 5/6, not yet by production call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FilterValidationError {
    UnknownKey {
        key: String,
        available: Vec<String>,
    },
    InvalidValue {
        key: String,
        value: String,
        allowed: Vec<String>,
    },
    NonNumeric {
        key: String,
        value: String,
    },
}

/// Validate `filters` against a site's `descriptors`. Unknown keys and
/// out-of-set values are rejected. `overrides` lists keys whose value check
/// differs from the default `AllowedValues` (keys absent default to it).
///
/// Returns a typed error; the caller formats it. NOTE the explicit
/// `std::result::Result` — `Result` is shadowed here by `rdlp_core::Result`.
///
/// `#[allow(dead_code)]`: same rationale as [`KeyValidation`] — consumed by
/// Task 5/6, not yet by production call sites.
#[allow(dead_code)]
pub(crate) fn validate_against_descriptors(
    filters: &[SearchFilter],
    descriptors: &[SearchFilterDescriptor],
    overrides: &[(&str, KeyValidation)],
) -> std::result::Result<(), FilterValidationError> {
    for filter in filters {
        let Some(descriptor) = descriptors.iter().find(|d| d.key == filter.key) else {
            return Err(FilterValidationError::UnknownKey {
                key: filter.key.clone(),
                available: descriptors.iter().map(|d| d.key.clone()).collect(),
            });
        };

        let policy = overrides
            .iter()
            .find(|(k, _)| *k == filter.key)
            .map_or(KeyValidation::AllowedValues, |(_, p)| *p);

        match policy {
            KeyValidation::FreeText => {}
            KeyValidation::NumericU32 => {
                if filter.value.parse::<u32>().is_err() {
                    return Err(FilterValidationError::NonNumeric {
                        key: filter.key.clone(),
                        value: filter.value.clone(),
                    });
                }
            }
            KeyValidation::AllowedValues => {
                let ok = descriptor
                    .allowed_values
                    .iter()
                    .any(|v| v.value == filter.value);
                if !ok {
                    return Err(FilterValidationError::InvalidValue {
                        key: filter.key.clone(),
                        value: filter.value.clone(),
                        allowed: descriptor
                            .allowed_values
                            .iter()
                            .map(|v| v.value.clone())
                            .collect(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Delay between successive search-page fetches. All API-paginated sites that
/// share the [`PaginatedSearch`] scaffold rate-limit at this interval.
pub(crate) const PAGE_RATE_LIMIT_MS: u64 = 500;

/// How a paginated search knows when to stop.
///
/// `Pages(n)` — the site reports a known page count; stop once `page >= n`.
/// `UntilEmpty` — no reliable total; stop only when a page comes back empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Termination {
    Pages(usize),
    /// Constructed by `PaginatedSearch` adopters that paginate until an empty
    /// page (PornHub, RedTube). No reliable total page count is available from
    /// these sites' responses.
    UntilEmpty,
}

impl Termination {
    /// True once the loop should stop, given the 1-based page just fetched.
    /// `n.max(1)` treats a zero total as one page (`Pages(0)` == `Pages(1)`);
    /// count==0 is normally caught by the empty-page break first, so do not
    /// "simplify" away the `.max(1)`.
    fn should_stop(self, page: usize) -> bool {
        match self {
            Termination::Pages(n) => page >= n.max(1),
            Termination::UntilEmpty => false,
        }
    }

    /// True while the loop/caller may fetch a further page after `page`
    /// (the inverse of `should_stop`). `pub(crate)` so single-page
    /// `search_page` callers can compute `has_more` without duplicating the
    /// match; `should_stop` itself stays private.
    pub(crate) fn has_more(self, page: usize) -> bool {
        !self.should_stop(page)
    }
}

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
/// Sites that report a total page count return `Termination::Pages(n)`; sites
/// with no reliable total (they paginate until an empty page) return
/// `Termination::UntilEmpty`. Per-page primary↔fallback fetching is a private
/// concern of each site's `fetch_search_page` implementation.
#[async_trait]
pub(crate) trait PaginatedSearch: Send + Sync {
    /// Bracketed site tag used in log lines, e.g. `"[XHamster]"`.
    fn search_log_tag(&self) -> &'static str;

    /// Validate the query's filters against this site's supported filter set.
    fn validate_search_filters(&self, filters: &[SearchFilter]) -> Result<()>;

    /// Fetch a single search page, returning `(results, termination)`.
    async fn fetch_search_page(
        &self,
        query: &SearchQuery,
        page: usize,
        ctx: &ExtractionContext,
    ) -> Result<(Vec<SearchResultPreview>, Termination)>;

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
            let (page_results, termination) = match self.fetch_search_page(query, page, ctx).await {
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

            if termination.should_stop(page) {
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
        Ok(Vec<SearchResultPreview>, Termination),
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
        ) -> Result<(Vec<SearchResultPreview>, Termination)> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            match self.script.get(page - 1) {
                Some(Page::Ok(results, termination)) => Ok((results.clone(), *termination)),
                Some(Page::Fail) => Err(RdlpError::extraction(
                    "mock fetch failure",
                    "https://x.test",
                )),
                None => Ok((Vec::new(), Termination::UntilEmpty)), // past script → empty page
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
            vec![
                Page::Ok(previews(3), Termination::Pages(10)),
                Page::Ok(previews(3), Termination::Pages(10)),
            ],
            Some(4),
        )
        .await;
        assert_eq!(out.len(), 4);
    }

    #[tokio::test]
    async fn stops_at_max_pages_without_fetching_beyond() {
        // page1 reports max_pages=1; page2 (if fetched) would add 5 — must not.
        let out = run(
            vec![
                Page::Ok(previews(2), Termination::Pages(1)),
                Page::Ok(previews(5), Termination::Pages(1)),
            ],
            None,
        )
        .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn stops_on_empty_page() {
        let out = run(
            vec![
                Page::Ok(previews(2), Termination::Pages(10)),
                Page::Ok(Vec::new(), Termination::Pages(10)),
            ],
            None,
        )
        .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn fetch_error_returns_partial_results() {
        let out = run(
            vec![Page::Ok(previews(2), Termination::Pages(10)), Page::Fail],
            None,
        )
        .await;
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn truncates_within_a_single_page() {
        let out = run(vec![Page::Ok(previews(5), Termination::Pages(10))], Some(3)).await;
        assert_eq!(out.len(), 3);
    }

    #[tokio::test]
    async fn accumulates_all_pages_when_uncapped() {
        // 2 pages, no result cap; max_pages=2 stops the loop naturally → all 5,
        // no truncation (the one path the other multi-page tests don't cover).
        let out = run(
            vec![
                Page::Ok(previews(3), Termination::Pages(2)),
                Page::Ok(previews(2), Termination::Pages(2)),
            ],
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
            script: vec![Page::Ok(previews(3), Termination::Pages(10))],
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

    // ---- Termination::should_stop ----

    #[test]
    fn should_stop_pages_stops_at_or_past_n() {
        assert!(
            !Termination::Pages(3).should_stop(2),
            "page 2 of 3: keep going"
        );
        assert!(Termination::Pages(3).should_stop(3), "page 3 of 3: stop");
        assert!(Termination::Pages(3).should_stop(4), "past the end: stop");
    }

    #[test]
    fn should_stop_pages_one_stops_after_page_1() {
        // The common single-page case.
        assert!(Termination::Pages(1).should_stop(1));
    }

    #[test]
    fn should_stop_pages_zero_behaves_as_one() {
        // `n.max(1)`: a zero total is treated as one page (page 1 is still fetched
        // by the loop before this check; count==0 is normally caught by the
        // empty-page break first). Documents Pages(0) == Pages(1).
        assert!(Termination::Pages(0).should_stop(1));
        assert_eq!(
            Termination::Pages(0).should_stop(1),
            Termination::Pages(1).should_stop(1)
        );
    }

    #[test]
    fn should_stop_until_empty_never_stops_on_count() {
        assert!(!Termination::UntilEmpty.should_stop(1));
        assert!(!Termination::UntilEmpty.should_stop(9_999));
    }

    #[tokio::test]
    async fn until_empty_accumulates_until_empty_page() {
        // No page-count bound; only the empty page stops it.
        let out = run(
            vec![
                Page::Ok(previews(3), Termination::UntilEmpty),
                Page::Ok(previews(2), Termination::UntilEmpty),
                Page::Ok(Vec::new(), Termination::UntilEmpty),
            ],
            None,
        )
        .await;
        assert_eq!(out.len(), 5);
    }

    #[tokio::test]
    async fn until_empty_respects_max_results() {
        // max_results caps before any empty page under UntilEmpty (all existing
        // truncation tests use Pages).
        let out = run(
            vec![
                Page::Ok(previews(4), Termination::UntilEmpty),
                Page::Ok(previews(4), Termination::UntilEmpty),
            ],
            Some(6),
        )
        .await;
        assert_eq!(out.len(), 6);
    }

    #[tokio::test]
    async fn pages_recompute_follows_latest_count() {
        // The §4.1 accepted semantic: a later page reporting a LARGER page count
        // (live-dataset drift) makes the loop follow the latest total. page-1 says
        // Pages(2), page-2 says Pages(3) → page 3 IS fetched. Would fail against a
        // freeze-first loop that locked max_pages=2 from page 1.
        let out = run(
            vec![
                Page::Ok(previews(2), Termination::Pages(2)),
                Page::Ok(previews(2), Termination::Pages(3)),
                Page::Ok(previews(2), Termination::Pages(3)),
            ],
            None,
        )
        .await;
        assert_eq!(out.len(), 6, "all 3 pages fetched (latest count = 3)");
    }

    mod validator_tests {
        use super::super::{FilterValidationError, KeyValidation, validate_against_descriptors};
        use rdlp_types::{SearchFilter, SearchFilterDescriptor, SearchFilterValue};

        fn descriptors() -> Vec<SearchFilterDescriptor> {
            vec![
                SearchFilterDescriptor::new(
                    "ordering",
                    "Sort by",
                    SearchFilterValue::list([("newest", "Newest"), ("rating", "Top")]),
                    Some("newest"),
                ),
                SearchFilterDescriptor::new("category", "Category", vec![], None),
                SearchFilterDescriptor::new("dur", "Duration", vec![], None),
            ]
        }
        fn f(k: &str, v: &str) -> SearchFilter {
            SearchFilter {
                key: k.into(),
                value: v.into(),
            }
        }

        #[test]
        fn ok_when_value_allowed() {
            let r = validate_against_descriptors(&[f("ordering", "rating")], &descriptors(), &[]);
            assert!(r.is_ok());
        }

        #[test]
        fn unknown_key() {
            let r = validate_against_descriptors(&[f("bogus", "x")], &descriptors(), &[]);
            assert_eq!(
                r.unwrap_err(),
                FilterValidationError::UnknownKey {
                    key: "bogus".into(),
                    available: vec!["ordering".into(), "category".into(), "dur".into()],
                }
            );
        }

        #[test]
        fn invalid_value_reports_allowed() {
            let r = validate_against_descriptors(&[f("ordering", "nope")], &descriptors(), &[]);
            assert_eq!(
                r.unwrap_err(),
                FilterValidationError::InvalidValue {
                    key: "ordering".into(),
                    value: "nope".into(),
                    allowed: vec!["newest".into(), "rating".into()],
                }
            );
        }

        #[test]
        fn free_text_skips_check() {
            let r = validate_against_descriptors(
                &[f("category", "anything-goes")],
                &descriptors(),
                &[("category", KeyValidation::FreeText)],
            );
            assert!(r.is_ok());
        }

        #[test]
        fn numeric_accepts_digits_rejects_non_digits() {
            let ov = &[("dur", KeyValidation::NumericU32)];
            assert!(validate_against_descriptors(&[f("dur", "42")], &descriptors(), ov).is_ok());
            assert_eq!(
                validate_against_descriptors(&[f("dur", "x")], &descriptors(), ov).unwrap_err(),
                FilterValidationError::NonNumeric {
                    key: "dur".into(),
                    value: "x".into()
                }
            );
        }
    }
}
