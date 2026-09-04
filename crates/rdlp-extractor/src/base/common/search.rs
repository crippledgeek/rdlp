//! Shared search-URL construction helpers for API-based search extractors.
//!
//! The *base* search URL (host, fixed query params, `search=` position) is
//! per-site knowledge and stays in each extractor. What is genuinely shared —
//! the same knowledge, changing together across sites — is how the standard
//! filter set is appended to an API search URL. Sites whose API accepts the
//! `ordering` / `period` / `category` / `tags[]` filter vocabulary (PornHub,
//! RedTube) delegate that appending here.

use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result};
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
#[derive(Debug)]
pub(crate) struct SearchPage {
    pub results: Vec<SearchResultPreview>,
    pub has_more: bool,
    pub total_estimate: Option<u64>,
}

/// A search/API base origin: `scheme://authority` (http or https), with no
/// path, query, fragment, or trailing slash. Concatenated with a path+query
/// template by the URL builders via `format!("{origin}{PATH}?…")`, so the
/// no-trailing-slash invariant keeps that concatenation deterministic.
///
/// Construction-time and trusted — the value is a compile-time literal in
/// production and a `mockito::Server::url()` in tests, never attacker-derived
/// (the SSRF invariant). Validation is shape-only, checked once at construction.
/// Mirrors `http::HeaderValue::from_static` / `http::Uri::from_static`
/// (validated, panic-on-bad "known-good constant" entry) paired with a fallible
/// `TryFrom`/`new` for dynamic input.
///
/// Consumed by the PornHub search-URL builders (issue #457 task 2); the
/// RedTube seam lands in task 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchOrigin(String);

/// Why a candidate origin string is not a valid [`SearchOrigin`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvalidOriginError {
    /// Missing or non-`http(s)` scheme.
    Scheme,
    /// Scheme present but no authority (host) follows.
    MissingAuthority,
    /// Contains a path, query, fragment, or trailing slash — not a bare origin.
    NotBareOrigin,
}

impl std::fmt::Display for InvalidOriginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::Scheme => "origin must start with http:// or https://",
            Self::MissingAuthority => "origin has no authority (host)",
            Self::NotBareOrigin => {
                "origin must be scheme://authority with no path, query, or trailing slash"
            }
        };
        f.write_str(msg)
    }
}

impl SearchOrigin {
    /// Validate the shape of a candidate origin. Shared by `from_static` and `new`.
    fn validate(src: &str) -> std::result::Result<(), InvalidOriginError> {
        let authority = src
            .strip_prefix("https://")
            .or_else(|| src.strip_prefix("http://"))
            .ok_or(InvalidOriginError::Scheme)?;
        if authority.is_empty() {
            return Err(InvalidOriginError::MissingAuthority);
        }
        // A bare authority (host[:port]) contains none of these. A trailing
        // slash is caught by the '/' check.
        if authority.contains('/') || authority.contains('?') || authority.contains('#') {
            return Err(InvalidOriginError::NotBareOrigin);
        }
        Ok(())
    }

    /// Known-good compile-time literal. **Panics** on a malformed origin — a
    /// construction-time contract breach, not a runtime error. The panic
    /// message names the error kind only (no URL) to satisfy the redaction gate.
    pub(crate) fn from_static(src: &'static str) -> Self {
        match Self::validate(src) {
            Ok(()) => Self(src.to_owned()),
            Err(e) => panic!("SearchOrigin::from_static: {e}"),
        }
    }

    /// Fallible constructor for runtime/dynamic input (mockito `Server::url()`).
    ///
    /// Validation is shape-only (scheme + authority + no path/query/fragment).
    /// It does NOT reject embedded userinfo credentials or backslashes. All
    /// current inputs are trusted (compile-time literals and loopback test
    /// servers), so this is safe today. Any future caller that feeds this from
    /// operator config or external/attacker-influenced input MUST additionally
    /// enforce a host allowlist and route the resulting fetch through
    /// `rdlp-security`'s `validate_url_security` before use.
    pub(crate) fn new(src: &str) -> std::result::Result<Self, InvalidOriginError> {
        Self::validate(src)?;
        Ok(Self(src.to_owned()))
    }
}

impl TryFrom<&str> for SearchOrigin {
    type Error = InvalidOriginError;
    /// See the trust/validation caveat on [`SearchOrigin::new`], which this delegates to.
    fn try_from(src: &str) -> std::result::Result<Self, Self::Error> {
        Self::new(src)
    }
}

impl std::fmt::Display for SearchOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SearchOrigin {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Per-site configuration for the default [`PagedSearch::fetch_page`] (via
/// [`PagedSearch::fetch_via_spec`]). All behavioral variation is a `fn`
/// pointer (zero-alloc, `Copy`); config is plain data. Sites pass bare `fn`
/// items or non-capturing closures (which coerce to `fn`).
pub(crate) struct SearchPageSpec {
    /// Extra request headers; `&[]` for none.
    pub headers: &'static [(&'static str, &'static str)],
    /// Build the page URL from the query and the (site-convention) page number.
    pub build_url: fn(&SearchQuery, u32) -> String,
    /// Parse the fetched body into results + pagination in one pass.
    pub parse: fn(&str, &SearchQuery, u32) -> Result<SearchPage>,
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

/// The value of the filter named `key`, or `None` when it was not supplied.
///
/// Borrows from `filters` rather than cloning: every caller either compares the
/// value or feeds it to a URL builder that takes `&str`.
///
/// Shared because looking a filter up by key is the same knowledge everywhere:
/// nine of the ten `PagedSearch` sites resolve an `ordering` / `period` /
/// `category` / `browse` / `sort` / `route` filter this way.
///
/// **First match wins, and duplicates are reachable.** `rdlp-cli` pushes every
/// `--search-filter key=value` verbatim with no de-duplication
/// (`crates/rdlp-cli/src/main.rs`), so `--search-filter sort=x --search-filter
/// sort=top` yields two `sort` filters. This returns the first, which is why
/// the `.any(|f| f.key == K && f.value == V)` predicates in xvideos/xnxx/abxxx
/// are deliberately NOT written in terms of this helper — they ask a different
/// question ("is there any such filter") and would change behaviour.
pub(crate) fn filter_value<'a>(filters: &'a [SearchFilter], key: &str) -> Option<&'a str> {
    filters
        .iter()
        .find(|f| f.key == key)
        .map(|f| f.value.as_str())
}

/// Format a [`FilterValidationError`] into an `RdlpError::Extraction` using the
/// shared "Family-1" wording, where the site name is the **only** per-site
/// variant. Used by PornHub / RedTube / XHamster, whose three validator error
/// arms are byte-identical apart from that literal.
///
/// Family-2 sites (TNAFlix / MovieFap) phrase these errors differently
/// (`Unknown {Site} search filter key '{key}'`, etc.) and MUST NOT use this
/// helper — their wording legitimately diverges (see #442).
pub(crate) fn format_std_filter_error(site: &str, error: FilterValidationError) -> RdlpError {
    let message = match error {
        FilterValidationError::UnknownKey { key, available } => format!(
            "Unknown filter '{key}' for {site}. Available: {}",
            available.join(", ")
        ),
        FilterValidationError::InvalidValue {
            key,
            value,
            allowed,
        } => format!(
            "Invalid value '{value}' for filter '{key}'. Allowed: {}",
            allowed.join(", ")
        ),
        FilterValidationError::NonNumeric { key, value } => {
            format!("Invalid value '{value}' for filter '{key}'. Must be a number.")
        }
    };
    RdlpError::Extraction { message, url: None }
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
    /// Used when no reliable total page count is available from a site's
    /// response, so pagination stops only on an empty page. RedTube's
    /// `termination_from_count` constructs this variant (via its `fetch_page`)
    /// when the API response carries no `count`. PornHub folds the same
    /// semantic inline in `fetch_page` (`has_more: true` on any non-empty
    /// page) rather than constructing this variant. Otherwise reachable via
    /// unit tests in this module.
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
/// empty page, `!has_more`, or a fetch error. A fetch error on the FIRST page
/// is returned to the caller, because there is nothing collected to salvage
/// and reporting a hard failure as "no results" hides each site's actionable
/// message; a later-page error returns the results gathered so far.
/// Implementors supply only the per-site pieces (a single
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
    /// Provided method — single-GET sites call
    /// `self.fetch_via_spec(SPEC, query, page, ctx)` from their `fetch_page`.
    /// `SearchPageSpec` is `Copy`, taken by value.
    ///
    /// 4 params: `spec` is a parameter-object, `ctx` the threaded extraction
    /// context, `(query, page)` the trait's established pair — no same-type
    /// ambiguity.
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

    /// Collect results across pages until `max_results` / an empty page /
    /// `!has_more` / a fetch error. Shared scaffold — do not override.
    ///
    /// Error handling is deliberately asymmetric, and a new site wiring itself
    /// onto this scaffold should not re-introduce the swallow it replaced: a
    /// failure on the FIRST page propagates as `Err`, so the site's own mapped
    /// error (a Cloudflare challenge, a refused status) reaches the operator
    /// instead of being reported as zero results. A failure on a later page
    /// returns the partial results already gathered. An empty-but-SUCCESSFUL
    /// first page is not an error and yields `Ok(vec![])`.
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
                // Nothing collected yet on the very first page: the request
                // never got off the ground, so there are no partial results to
                // salvage and "return what we have" would report a hard
                // failure as "no results found". Each site maps its own
                // actionable error here (a Cloudflare challenge naming
                // `--cookies-from-browser`, a refusal to parse a 404 that
                // carries a full grid of filler); swallowing it into a
                // `debug!` the operator has not enabled discards the only
                // thing that tells them what to do. An empty-but-SUCCESSFUL
                // first page is a different case and still returns `Ok`.
                Err(e) if all_results.is_empty() && page == self.first_page_index() => {
                    debug!(page; "{tag} First search page failed, no partial results to return: {e}");
                    return Err(e);
                }
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
    fn filter_value_reads_a_present_key_and_reports_an_absent_one() {
        let filters = [filter("ordering", "newest"), filter("period", "weekly")];
        assert_eq!(filter_value(&filters, "ordering"), Some("newest"));
        assert_eq!(filter_value(&filters, "period"), Some("weekly"));
        assert_eq!(filter_value(&filters, "category"), None);
        assert_eq!(filter_value(&[], "ordering"), None);
    }

    /// First match wins. This is not academic: `rdlp-cli` pushes every
    /// `--search-filter key=value` verbatim with no de-duplication, so a
    /// duplicated key reaches this helper. Nine extractors resolve their
    /// filters through it, and they previously hand-rolled `.find(...)` —
    /// which is first-match. Anything else here would silently change all of
    /// them.
    #[test]
    fn filter_value_returns_the_first_of_duplicate_keys() {
        let filters = [filter("sort", "first"), filter("sort", "second")];
        assert_eq!(filter_value(&filters, "sort"), Some("first"));
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
            // True of every script `run` is called with (all succeed on page 1),
            // NOT of the scaffold in general — a first-page failure returns `Err`.
            // Use `run_result` for scripts that fail on page 1.
            .expect("these scripts all succeed on page 1, so partials are returned")
    }

    /// `run` unwraps, so a script whose FIRST page fails needs the raw result.
    async fn run_result(script: Vec<Page>) -> Result<Vec<SearchResultPreview>> {
        MockSearch::new(script)
            .search_all_pages(&query(None), &test_ctx())
            .await
    }

    /// A first-page failure must REACH THE OPERATOR, not become "no results".
    ///
    /// Every site wires `SearchExtractor::search` straight to this scaffold, so
    /// swallowing here discards each site's carefully-mapped error: PornoXO's
    /// Cloudflare guidance naming `--cookies-from-browser`, and its refusal to
    /// parse a 404 that carries a full grid of filler. The operator saw "no
    /// results found" for a site that was merely gated.
    #[tokio::test]
    async fn first_page_error_propagates_instead_of_reporting_no_results() {
        let err = run_result(vec![Page::Fail])
            .await
            .expect_err("a first-page failure must not be reported as zero results");
        assert!(
            err.to_string().contains("mock fetch failure"),
            "the site's own message must survive intact: {err}"
        );
    }

    /// The case that must NOT change: a legitimately empty search returns a
    /// successful, empty page and still yields `Ok(vec![])`. Only a genuine
    /// fetch/HTTP error propagates.
    #[tokio::test]
    async fn an_empty_but_successful_first_page_is_still_ok() {
        let out = run_result(vec![Page::Ok(Vec::new(), Termination::Pages(10))])
            .await
            .expect("an empty result set is not an error");
        assert!(out.is_empty());
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
        // One of the scaffold's two Err paths (the other is a first-page
        // fetch failure): a failed filter validation must short-circuit
        // BEFORE any page fetch.
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

    // ---- SearchOrigin ----

    #[test]
    fn search_origin_from_static_accepts_bare_https_origin() {
        let o = SearchOrigin::from_static("https://www.pornhub.com");
        assert_eq!(o.as_ref(), "https://www.pornhub.com");
        assert_eq!(format!("{o}"), "https://www.pornhub.com");
    }

    #[test]
    fn search_origin_new_accepts_http_loopback_with_port() {
        // mockito Server::url() shape
        let o = SearchOrigin::new("http://127.0.0.1:1234").unwrap();
        assert_eq!(o.as_ref(), "http://127.0.0.1:1234");
    }

    #[test]
    fn search_origin_rejects_non_http_scheme() {
        assert_eq!(
            SearchOrigin::new("ftp://x").unwrap_err(),
            InvalidOriginError::Scheme
        );
        assert_eq!(
            SearchOrigin::new("www.x.com").unwrap_err(),
            InvalidOriginError::Scheme
        );
    }

    #[test]
    fn search_origin_rejects_missing_authority() {
        assert_eq!(
            SearchOrigin::new("https://").unwrap_err(),
            InvalidOriginError::MissingAuthority
        );
    }

    #[test]
    fn search_origin_rejects_trailing_slash_and_path_and_query() {
        // trailing slash would double-slash in format!("{origin}{PATH}")
        assert_eq!(
            SearchOrigin::new("https://x.com/").unwrap_err(),
            InvalidOriginError::NotBareOrigin
        );
        assert_eq!(
            SearchOrigin::new("https://x.com/path").unwrap_err(),
            InvalidOriginError::NotBareOrigin
        );
        assert_eq!(
            SearchOrigin::new("https://x.com?q=1").unwrap_err(),
            InvalidOriginError::NotBareOrigin
        );
    }

    #[test]
    fn search_origin_try_from_matches_new() {
        let o: SearchOrigin = "https://api.redtube.com".try_into().unwrap();
        assert_eq!(o.as_ref(), "https://api.redtube.com");
    }

    #[test]
    #[should_panic(expected = "SearchOrigin::from_static")]
    fn search_origin_from_static_panics_on_trailing_slash() {
        let _ = SearchOrigin::from_static("https://x.com/");
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

    /// Byte-exact wording guard for the shared Family-1 formatter. The per-site
    /// validator tests only assert `.contains(substr)`, so these exact-string
    /// checks are the sole guard that PornHub / RedTube / XHamster error
    /// messages are reproduced verbatim across the dedup.
    mod format_std_filter_error_tests {
        use super::super::{FilterValidationError, format_std_filter_error};
        use rdlp_core::RdlpError;

        fn extraction_message(site: &str, error: FilterValidationError) -> String {
            match format_std_filter_error(site, error) {
                RdlpError::Extraction { message, url } => {
                    assert!(url.is_none(), "Family-1 filter errors carry no URL");
                    message
                }
                other => panic!("expected RdlpError::Extraction, got {other:?}"),
            }
        }

        #[test]
        fn unknown_key_arm_exact_wording() {
            let error = FilterValidationError::UnknownKey {
                key: "bogus".into(),
                available: vec!["ordering".into(), "category".into()],
            };
            assert_eq!(
                extraction_message("PornHub", error),
                "Unknown filter 'bogus' for PornHub. Available: ordering, category"
            );
        }

        #[test]
        fn invalid_value_arm_exact_wording() {
            let error = FilterValidationError::InvalidValue {
                key: "ordering".into(),
                value: "nope".into(),
                allowed: vec!["newest".into(), "rating".into()],
            };
            assert_eq!(
                extraction_message("RedTube", error),
                "Invalid value 'nope' for filter 'ordering'. Allowed: newest, rating"
            );
        }

        #[test]
        fn non_numeric_arm_exact_wording() {
            let error = FilterValidationError::NonNumeric {
                key: "dur".into(),
                value: "x".into(),
            };
            assert_eq!(
                extraction_message("XHamster", error),
                "Invalid value 'x' for filter 'dur'. Must be a number."
            );
        }

        /// The site name is the ONLY per-site variant: the same `UnknownKey`
        /// input yields wording that differs only in the interpolated site.
        #[test]
        fn site_name_is_the_only_variant() {
            let mk = |site| {
                extraction_message(
                    site,
                    FilterValidationError::UnknownKey {
                        key: "k".into(),
                        available: vec!["a".into()],
                    },
                )
            };
            assert_eq!(
                mk("PornHub"),
                "Unknown filter 'k' for PornHub. Available: a"
            );
            assert_eq!(
                mk("RedTube"),
                "Unknown filter 'k' for RedTube. Available: a"
            );
            assert_eq!(
                mk("XHamster"),
                "Unknown filter 'k' for XHamster. Available: a"
            );
        }
    }
}
