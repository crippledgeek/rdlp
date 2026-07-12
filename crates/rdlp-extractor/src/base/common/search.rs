//! Shared search-URL construction helpers for API-based search extractors.
//!
//! The *base* search URL (host, fixed query params, `search=` position) is
//! per-site knowledge and stays in each extractor. What is genuinely shared —
//! the same knowledge, changing together across sites — is how the standard
//! filter set is appended to an API search URL. Sites whose API accepts the
//! `ordering` / `period` / `category` / `tags[]` filter vocabulary (PornHub,
//! RedTube) delegate that appending here.

use log::debug;
use rdlp_core::{ExtractionContext, Result};
use rdlp_types::{
    SearchFilter, SearchFilterDescriptor, SearchPageResponse, SearchQuery, SearchResultPreview,
};
use std::time::Duration;
use url::form_urlencoded;

use super::MAX_PLAYLIST_SIZE;
use crate::base::common::BaseExtractor;

/// One parsed search page: the rows, whether another page exists, and an
/// optional total-result estimate. Produced by a site's `parse` hook in a
/// single pass (no re-scan of the body).
pub(crate) struct SearchPage {
    pub results: Vec<SearchResultPreview>,
    pub has_more: bool,
    pub total_estimate: Option<u64>,
}

/// Transitional alias for [`SearchPage`], kept only while [`run_search_page`]
/// and the 7 single-GET `#440` sites that still call it (XNXX, XVideos, XTits,
/// NineAnime, EPorner, SpankBang, HQPorner) reference the old name. Removed in
/// sub-PR 3b together with `run_search_page` and `SearchPageSpec::first_page_index`.
pub(crate) type SearchParse = SearchPage;

/// Per-site configuration for the default [`PagedSearch::fetch_page`] (via
/// [`PagedSearch::fetch_via_spec`]) and the legacy [`run_search_page`]. All
/// behavioral variation is a `fn` pointer (zero-alloc, `Copy`); config is plain
/// data. Sites pass bare `fn` items or non-capturing closures (which coerce to `fn`).
pub(crate) struct SearchPageSpec {
    /// The page a `None` `SearchQuery::page` defaults to (0 or 1 per site).
    ///
    /// Read only by [`run_search_page`]; `fetch_via_spec` takes `page` as an
    /// argument (the first-page index is [`PagedSearch::first_page_index`] there).
    /// Transitional — removed in sub-PR 3b with `run_search_page`.
    pub first_page_index: u32,
    /// Extra request headers; `&[]` for none.
    pub headers: &'static [(&'static str, &'static str)],
    /// Build the page URL from the query and the (site-convention) page number.
    pub build_url: fn(&SearchQuery, u32) -> String,
    /// Parse the fetched body into results + pagination in one pass.
    pub parse: fn(&str, &SearchQuery, u32) -> Result<SearchPage>,
}

/// Shared single-GET search-page skeleton: derive the page, build the URL,
/// fetch once (with optional headers), parse, and assemble the response. Sites
/// with genuinely divergent shapes (two-fetch fallback, termination-based
/// pagination) keep their own `search_page` and do not use this.
pub(crate) async fn run_search_page(
    query: &SearchQuery,
    ctx: &ExtractionContext,
    spec: SearchPageSpec,
) -> Result<SearchPageResponse> {
    let page = query.page.unwrap_or(spec.first_page_index);
    let url = (spec.build_url)(query, page);
    let body = if spec.headers.is_empty() {
        BaseExtractor::fetch_webpage(&url, ctx).await?
    } else {
        BaseExtractor::fetch_webpage_with_headers(&url, spec.headers, ctx).await?
    };
    let parsed = (spec.parse)(&body, query, page)?;
    Ok(SearchPageResponse {
        results: parsed.results,
        page,
        has_more: parsed.has_more,
        total_estimate: parsed.total_estimate,
    })
}

/// How one filter key's value is validated by [`validate_against_descriptors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// share the [`PagedSearch`] scaffold rate-limit at this interval.
pub(crate) const PAGE_RATE_LIMIT_MS: u64 = 500;

/// How a paginated search knows when to stop.
///
/// `Pages(n)` — the site reports a known page count; stop once `page >= n`.
/// `UntilEmpty` — no reliable total; stop only when a page comes back empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Termination {
    Pages(usize),
    /// Constructed by `PagedSearch` adopters that paginate until an empty
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
/// Sites whose search fetches page N and learns whether another page exists
/// from the same response share one pagination loop: fetch pages in order,
/// accumulate previews, and stop at the first of — `max_results` reached, an
/// empty page, `!has_more`, or a fetch error (returning the partial results
/// gathered so far). Implementors supply only the per-site pieces (a single
/// [`fetch_page`](Self::fetch_page), a log tag, filter validation);
/// [`search_all_pages`](Self::search_all_pages) is the shared default and
/// should not be overridden.
///
/// Each `fetch_page` computes its own `has_more` (from a site page count via
/// the [`Termination`] helper, or from result-emptiness). Per-page
/// primary↔fallback fetching is a private concern of each site's `fetch_page`.
pub(crate) trait PagedSearch: Send + Sync {
    /// Bracketed site tag used in log lines, e.g. `"[XHamster]"`.
    fn search_log_tag(&self) -> &'static str;

    /// Validate the query's filters against this site's supported filter set.
    fn validate_search_filters(&self, filters: &[SearchFilter]) -> Result<()>;

    /// Fetch + parse ONE page — the single behavioral hook. REQUIRED (no default).
    /// Single-GET sites implement it as a one-liner delegating to [`fetch_via_spec`];
    /// exotic sites (two-GET fallback, multi-endpoint) implement a custom body.
    /// Requiring it makes "forgot to implement" a compile error, not a runtime panic —
    /// which also lets the crate keep `expect_used`/`unwrap_used` clean (no `.expect()`).
    ///
    /// [`fetch_via_spec`]: Self::fetch_via_spec
    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage>;

    /// Drive a single-GET [`SearchPageSpec`]: build URL → fetch (± headers) → parse.
    /// This is today's `run_search_page` body minus the response assembly (which
    /// lives in [`search_page_response`](Self::search_page_response)). Provided
    /// method — single-GET sites call `self.fetch_via_spec(SPEC, query, page, ctx)`
    /// from their `fetch_page`. `SearchPageSpec` is `Copy`, taken by value.
    ///
    // No caller yet in sub-PR 3a (the 6 migrated sites implement `fetch_page`
    // directly). The first callers land in sub-PR 3b (the single-GET #440
    // sites). `expect` (not `allow`) is deliberate: it is self-cleaning —
    // the moment 3b adds a caller the lint stops firing and `-D warnings`
    // turns the now-unfulfilled expectation into an error, forcing this
    // attribute's removal. See `run_search_page` (deleted in 3b) for today's
    // equivalent body.
    #[expect(dead_code, reason = "first caller lands in Stage 3b, #450")]
    async fn fetch_via_spec(
        &self,
        spec: SearchPageSpec,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
        let url = (spec.build_url)(query, page);
        let body = if spec.headers.is_empty() {
            BaseExtractor::fetch_webpage(&url, ctx).await?
        } else {
            BaseExtractor::fetch_webpage_with_headers(&url, spec.headers, ctx).await?
        };
        (spec.parse)(&body, query, page)
    }

    /// The page a `None` `SearchQuery::page` defaults to (0 or 1 per site).
    fn first_page_index(&self) -> u32 {
        1
    }

    /// Clamp the derived single-page number before it is used for BOTH the fetch
    /// URL and the echoed `SearchPageResponse.page`. Default identity. A site with
    /// a floor (ABXXX: `page.max(1)`) overrides this; a universal
    /// `.max(first_page_index())` is WRONG — it would clamp 1-indexed sites at
    /// `page = Some(0)`, changing their echoed page. Opt-in per site only.
    fn clamp_page(&self, page: u32) -> u32 {
        page
    }

    /// Delay between successive page fetches. Defaults to [`PAGE_RATE_LIMIT_MS`].
    fn page_rate_limit(&self) -> Duration {
        Duration::from_millis(PAGE_RATE_LIMIT_MS)
    }

    /// The `max_results` cap when the query does not specify one. Defaults to
    /// [`MAX_PLAYLIST_SIZE`]; #440 single-GET sites override to their file-local 500.
    fn max_results_default(&self) -> usize {
        MAX_PLAYLIST_SIZE
    }

    /// Collect results across pages until `max_results` / an empty page / `!has_more`
    /// / a fetch error (returning partials). Shared scaffold — do not override.
    async fn search_all_pages(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        self.validate_search_filters(&query.filters)?;

        let tag = self.search_log_tag();
        let max_results = query.max_results.unwrap_or(self.max_results_default());
        let mut all_results: Vec<SearchResultPreview> = Vec::new();
        let mut page = self.first_page_index();

        loop {
            let SearchPage {
                results, has_more, ..
            } = match self.fetch_page(query, page, ctx).await {
                Ok(p) => p,
                Err(e) => {
                    debug!(page; "{tag} Failed to fetch search page, returning partial results: {e}");
                    break;
                }
            };

            if results.is_empty() {
                debug!(page; "{tag} No results on page, stopping pagination");
                break;
            }

            if all_results.is_empty() {
                all_results = results;
            } else {
                all_results.extend(results);
            }

            if all_results.len() >= max_results {
                all_results.truncate(max_results);
                break;
            }

            if !has_more {
                break;
            }

            page += 1;
            tokio::time::sleep(self.page_rate_limit()).await;
        }

        debug!(count = all_results.len(), pages = page; "{tag} Search complete");

        Ok(all_results)
    }

    /// Assemble a single-page `SearchPageResponse` from one [`fetch_page`]. Shared
    /// default; the fallback pair (PornHub, RedTube) override this to keep their
    /// divergent single-page API-fallback semantics.
    ///
    /// [`fetch_page`]: Self::fetch_page
    async fn search_page_response(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        self.validate_search_filters(&query.filters)?;

        let page = self.clamp_page(query.page.unwrap_or(self.first_page_index()));
        let SearchPage {
            results,
            has_more,
            total_estimate,
        } = self.fetch_page(query, page, ctx).await?;

        Ok(SearchPageResponse {
            results,
            page,
            has_more,
            total_estimate,
        })
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

    impl PagedSearch for MockSearch {
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
        async fn fetch_page(
            &self,
            _query: &SearchQuery,
            page: u32,
            _ctx: &ExtractionContext,
        ) -> Result<SearchPage> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            match self.script.get((page - 1) as usize) {
                Some(Page::Ok(results, termination)) => {
                    let has_more = !results.is_empty() && termination.has_more(page as usize);
                    Ok(SearchPage {
                        results: results.clone(),
                        has_more,
                        total_estimate: None,
                    })
                }
                Some(Page::Fail) => Err(RdlpError::extraction(
                    "mock fetch failure",
                    "https://x.test",
                )),
                None => Ok(SearchPage {
                    results: Vec::new(),
                    has_more: false,
                    total_estimate: None,
                }), // past script → empty page
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

    #[test]
    fn paged_search_futures_are_send() {
        // Guards the native-AFIT migration's correctness contract: the outer
        // `#[async_trait] SearchExtractor::search` future must be `Send`, which
        // requires the `PagedSearch` futures it awaits at the concrete call
        // site to be `Send`. Under `#[async_trait]` this was guaranteed by the
        // boxed `dyn Future + Send`; under native AFIT it is inferred, so pin it
        // here. If a future ever goes non-`Send`, this fails at compile time
        // instead of cryptically deep inside a site's `SearchExtractor::search`.
        fn assert_send<T: Send>(_: T) {}

        let mock = MockSearch::new(Vec::new());
        let q = query(None);
        let ctx = test_ctx();

        assert_send(mock.fetch_page(&q, 1, &ctx));
        assert_send(mock.search_all_pages(&q, &ctx));
        assert_send(mock.search_page_response(&q, &ctx));
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

    // ---- PaginatedSearch::search_page_response ----

    fn query_with_page(page: Option<u32>) -> SearchQuery {
        SearchQuery {
            query: "q".to_string(),
            filters: Vec::new(),
            max_results: None,
            page,
        }
    }

    #[tokio::test]
    async fn search_page_response_reports_has_more_when_more_pages() {
        let mock = MockSearch::new(vec![Page::Ok(previews(3), Termination::Pages(5))]);
        let resp = mock
            .search_page_response(&query_with_page(None), &test_ctx())
            .await
            .expect("ok");
        assert_eq!(resp.results.len(), 3);
        assert_eq!(resp.page, 1);
        assert!(resp.has_more);
        assert_eq!(resp.total_estimate, None);
    }

    #[tokio::test]
    async fn search_page_response_no_more_on_last_page() {
        let mock = MockSearch::new(vec![Page::Ok(previews(3), Termination::Pages(1))]);
        let resp = mock
            .search_page_response(&query_with_page(None), &test_ctx())
            .await
            .expect("ok");
        assert!(!resp.has_more);
    }

    #[tokio::test]
    async fn search_page_response_empty_results_forces_has_more_false() {
        let mock = MockSearch::new(vec![Page::Ok(previews(0), Termination::Pages(5))]);
        let resp = mock
            .search_page_response(&query_with_page(None), &test_ctx())
            .await
            .expect("ok");
        assert!(resp.results.is_empty());
        assert!(!resp.has_more);
    }

    #[tokio::test]
    async fn search_page_response_honors_query_page() {
        let mock = MockSearch::new(vec![
            Page::Ok(previews(2), Termination::Pages(5)),
            Page::Ok(previews(4), Termination::Pages(5)),
        ]);
        let resp = mock
            .search_page_response(&query_with_page(Some(2)), &test_ctx())
            .await
            .expect("ok");
        assert_eq!(resp.page, 2);
        assert_eq!(resp.results.len(), 4);
    }

    #[tokio::test]
    async fn search_page_response_validation_error_does_not_fetch() {
        let mock = MockSearch {
            script: vec![Page::Ok(previews(3), Termination::Pages(5))],
            validation_fails: true,
            fetches: AtomicUsize::new(0),
        };
        let result = mock
            .search_page_response(&query_with_page(None), &test_ctx())
            .await;
        assert!(result.is_err(), "validation failure must propagate as Err");
        assert_eq!(
            mock.fetches.load(Ordering::SeqCst),
            0,
            "must not fetch when validation fails"
        );
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

/// Static regression guard for issue #457 (Refs).
///
/// PornHub and RedTube each implement `PagedSearch::fetch_page` (loop
/// semantics) and `PagedSearch::search_page_response` (single-page
/// semantics) with deliberately DIVERGENT fallback logic — see the doc
/// comments on those methods in `extractors/pornhub/mod.rs` and
/// `extractors/redtube/mod.rs`. A real end-to-end behavioral test (mockito
/// driving `search_page_response`/`fetch_page` and asserting which URLs were
/// hit) is infeasible today: the per-site URL builders hardcode `https://`
/// with no injectable base, `mockito` is HTTP-only (cannot terminate TLS),
/// and `ExtractionContext.http_client` is a concrete `Arc<wreq::Client>`
/// with no mock seam — a documented, deliberate project limitation (see
/// `extractors/pornhub/tests/fetch.rs:1-7`, PR #231). A real golden test
/// needs an injectable base-URL seam, deferred to follow-up issue **#457**.
///
/// Until that seam lands, this textual guard is the cheapest deterministic
/// regression protection available: it `include_str!`s each site's source
/// (so cargo recompiles this test whenever the extractor file changes — no
/// runtime `fs` access) and asserts the token that makes each method's
/// fallback behavior diverge from its sibling. If the two paths were ever
/// accidentally unified (e.g. `fetch_page` made to page the fallback on
/// every page, or `search_page_response` stopped paging it), the relevant
/// token moves into the wrong method span and this test fails.
///
/// A future refactor that extracts `fetch_page` / `search_page_response`
/// into shared helpers must update the anchors below (the `find` calls
/// `unwrap_or_else`-panic loudly rather than silently no-op'ing), mirroring
/// `test_extractor_call_order_expand_before_detect` in
/// `hls/expand_in_place.rs`.
#[cfg(test)]
mod fallback_divergence_guard {
    /// Split `src` into the `fetch_page` and `search_page_response` method
    /// spans, using the sibling method / trailing `#[async_trait]` as the
    /// boundary. Panics loudly (naming `label`) if an anchor is missing —
    /// a rename must fail this test, not silently pass it.
    fn method_spans<'a>(label: &str, src: &'a str) -> (&'a str, &'a str) {
        let fetch_start = src
            .find("async fn fetch_page")
            .unwrap_or_else(|| panic!("`async fn fetch_page` not found in {label}"));
        let response_fn_start = src
            .find("async fn search_page_response")
            .unwrap_or_else(|| panic!("`async fn search_page_response` not found in {label}"));
        assert!(
            fetch_start < response_fn_start,
            "{label}: expected `fetch_page` to appear before `search_page_response`"
        );

        // `search_page_response`'s own doc comment sits directly above its
        // `async fn` line, with no blank line separating them from
        // `fetch_page`'s closing brace — back up over that doc comment so
        // its prose (which may itself mention the sibling method's token,
        // e.g. "has_more" wording) is attributed to the RIGHT span. The
        // blank line right before the doc comment is the real boundary.
        let response_start = src[fetch_start..response_fn_start]
            .rfind("\n\n")
            .map(|offset| fetch_start + offset + 2)
            .unwrap_or_else(|| {
                panic!(
                    "no blank line found between `fetch_page` and `search_page_response`'s \
                     doc comment in {label}"
                )
            });

        // Search for `#[async_trait]` starting AFTER `search_page_response` —
        // an earlier `#[async_trait]` (e.g. on a helper trait impl) would
        // wrongly truncate the span before it began.
        let trait_boundary = src[response_fn_start..]
            .find("#[async_trait]")
            .map(|offset| response_fn_start + offset)
            .unwrap_or_else(|| panic!("`#[async_trait]` (impl SearchExtractor) not found in {label} after search_page_response"));

        (
            &src[fetch_start..response_start],
            &src[response_start..trait_boundary],
        )
    }

    /// PornHub: `fetch_page` only falls back to the API on page 1, using the
    /// BASE url builder (`build_api_search_url`, no `_page` suffix).
    /// `search_page_response` falls back to the API on ANY page, using the
    /// PAGED url builder (`build_api_search_url_page`) once `page > 1`. If
    /// the two paths were unified, `build_api_search_url_page` would appear
    /// in (or disappear from) the wrong span.
    #[test]
    fn pornhub_paged_api_fallback_url_builder_stays_in_search_page_response_only() {
        let src = include_str!("../../extractors/pornhub/mod.rs");
        let (fetch_page, search_page_response) = method_spans("extractors/pornhub/mod.rs", src);

        assert!(
            !fetch_page.contains("build_api_search_url_page"),
            "PornHub `fetch_page` (loop) must NOT call the paged API url builder — \
             its API fallback only ever targets page 1 (issue #457). If this fires, the \
             loop and single-page fallback paths have likely been unified."
        );
        assert!(
            search_page_response.contains("build_api_search_url_page"),
            "PornHub `search_page_response` (single-page) must call \
             `build_api_search_url_page` for its any-page API fallback (issue #457)."
        );
    }

    /// RedTube: `fetch_page` computes `has_more` via
    /// `termination_from_count(count).has_more(page)`; `search_page_response`
    /// instead computes it via the `fetched_through < total` window. If the
    /// two paths were unified onto one `has_more` strategy, one of these four
    /// assertions fails.
    #[test]
    fn redtube_has_more_strategy_diverges_between_loop_and_single_page() {
        let src = include_str!("../../extractors/redtube/mod.rs");
        let (fetch_page, search_page_response) = method_spans("extractors/redtube/mod.rs", src);

        assert!(
            fetch_page.contains("termination_from_count"),
            "RedTube `fetch_page` (loop) must compute `has_more` via \
             `termination_from_count` (issue #457)."
        );
        assert!(
            !fetch_page.contains("fetched_through"),
            "RedTube `fetch_page` (loop) must NOT use the `fetched_through` window — \
             that strategy belongs to `search_page_response` only (issue #457)."
        );
        assert!(
            search_page_response.contains("fetched_through"),
            "RedTube `search_page_response` (single-page) must compute `has_more` via \
             the `fetched_through < total` window (issue #457)."
        );
        assert!(
            !search_page_response.contains("termination_from_count"),
            "RedTube `search_page_response` (single-page) must NOT use \
             `termination_from_count` — that strategy belongs to `fetch_page` only \
             (issue #457)."
        );
    }
}
