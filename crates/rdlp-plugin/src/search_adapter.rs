//! Host side of the plugin search contract.
//!
//! `search-filters` is a world export added in 0.5.1. The host binds the
//! `extractor-plugin-host` world (which omits it) because wasmtime's
//! generated bindings require every world export at instantiate time, so
//! a 0.5.0 component would otherwise fail to load at all. This module
//! resolves the export by name on the live instance instead: absent means
//! "no filters", not an error. See `wit/COMPATIBILITY.md` §3.
//!
//! `search` is a frozen 0.5.0 export and goes through the generated
//! bindings like `extract` does. Both calls run inside
//! `PluginExtractor::run_in_fresh_store` under `SEARCH_TIMEOUT`.
//!
//! [`PluginSearchExtractor`] is what the registry holds: it presents a
//! plugin as a `SearchExtractor` and answers the host's shared
//! `PagedSearch` scaffold, so filter validation, paging, `max_results`,
//! pacing and duplicate-page termination happen host-side exactly as they
//! do for a built-in site — the plugin only ever sees one page request.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::OnceCell;
use wasmtime::Store;

use crate::PluginError;
use crate::adapter::{
    CallSpec, CommonPluginErr, FreshInstance, PluginExtractor, SEARCH_TIMEOUT, common_plugin_error,
    plugin_error_to_rdlp,
};
use crate::bindings::rdlp::plugin::types::{
    SearchError as WitSearchError, SearchPage as WitSearchPage, SearchQuery as WitSearchQuery,
    SearchResult as WitSearchResult,
};
use crate::convert::PluginOrigin;
use crate::instance::PluginStoreData;
use rdlp_core::{ExtractionContext, RdlpError, SearchExtractor};
use rdlp_extractor::base::common::{
    PagedSearch, SearchPage, format_std_filter_error, validate_against_descriptors,
};
use rdlp_types::{
    SearchFilter, SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};
use wasmtime::component::{ComponentType, Lift};

/// Name of the optional world export, as declared in `wit/extractor.wit`.
const SEARCH_FILTERS_EXPORT: &str = "search-filters";

/// Upper bound on the descriptors one `search-filters` answer may declare.
/// The largest in-tree table (`RedTube`) has four; 32 leaves room for a far
/// richer site while keeping the `Available:` list the host renders on an
/// unknown-key error bounded by a constant, not by the plugin.
const MAX_SEARCH_FILTER_DESCRIPTORS: usize = 32;

/// Upper bound on one descriptor's `allowed-values`. `RedTube`'s category
/// list, the largest in-tree, is ~90 entries; 256 is headroom over that and
/// bounds the `Allowed:` list an invalid-value error renders.
const MAX_SEARCH_FILTER_VALUES: usize = 256;

/// Upper bound, in bytes, on any single descriptor string (key, display
/// name, default, or one allowed value). These are identifiers and short
/// labels — the longest in-tree value is under 32 bytes — and every one of
/// them is echoed into operator-facing error text, so a plugin must not be
/// able to make that text arbitrarily long.
const MAX_SEARCH_FILTER_STRING_BYTES: usize = 256;

/// Hand-written lift of the `search-filter-descriptor` record from
/// `wit/types.wit`. `bindgen!` emits only the types the bound
/// `extractor-plugin-host` world references, and that world deliberately
/// omits `search-filters` — so the record it returns is not generated and
/// must be declared here, field-for-field, for the typed by-name call.
/// `tests::lift_mirrors_the_wit_record_field_for_field` pins this struct
/// to the record text in `wit/types.wit`; a drifted field order or type
/// fails there before it can mis-lift at runtime.
#[derive(Debug, Clone, PartialEq, Eq, ComponentType, Lift)]
#[component(record)]
pub(crate) struct WitSearchFilterDescriptor {
    /// Machine-readable filter key.
    pub key: String,
    /// Human-readable filter name.
    #[component(name = "display-name")]
    pub display_name: String,
    /// Allowed values; the host renders each label as its value.
    #[component(name = "allowed-values")]
    pub allowed_values: Vec<String>,
    /// Default value, if any.
    pub default: Option<String>,
}

impl PluginExtractor {
    /// The plugin's search filter descriptors, or an empty list when the
    /// component predates the `search-filters` export.
    ///
    /// This is the raw WIT-level call; a `SearchExtractor` impl converts
    /// on top of it. Runs in a fresh store under `SEARCH_TIMEOUT` with
    /// the same strike accounting as `extract`.
    ///
    /// # Errors
    ///
    /// The runner's errors ([`PluginError::Disabled`], `Timeout`,
    /// `Trapped`), or `Trapped` when the export exists with an unexpected
    /// signature or traps while running.
    pub async fn call_search_filters(&self) -> Result<Vec<SearchFilterDescriptor>, PluginError> {
        let spec = CallSpec {
            subject_for_errors: self.manifest.search_site_name(),
            timeout: SEARCH_TIMEOUT,
        };
        self.run_in_fresh_store(spec, |store, inst| {
            Box::pin(call_search_filters(store, inst))
        })
        .await
    }

    /// One page of search results from the plugin's `search` export.
    ///
    /// This is the raw WIT-level call; a `SearchExtractor` impl converts
    /// the query and page on top of it. Runs in a fresh store under
    /// `SEARCH_TIMEOUT` with the same strike accounting as `extract`.
    ///
    /// # Errors
    ///
    /// The runner's errors, or the plugin's own `search-error` mapped to a
    /// `PluginError` — `unsupported` becomes
    /// [`PluginError::SearchUnsupported`], which is not a strike.
    pub async fn call_search(&self, query: WitSearchQuery) -> Result<WitSearchPage, PluginError> {
        let spec = CallSpec {
            subject_for_errors: self.manifest.search_site_name(),
            timeout: SEARCH_TIMEOUT,
        };
        // The query moves into the future: the runner's closure is
        // higher-ranked over the store borrow, so it cannot return a future
        // that also borrows from this frame.
        self.run_in_fresh_store(spec, move |store, inst| {
            Box::pin(async move { call_search(store, inst, &query).await })
        })
        .await
    }
}

/// Look up `search-filters` by name on the live instance and call it.
/// Absent export ⇒ `Ok(vec![])` — the component was built before 0.5.1.
pub(crate) async fn call_search_filters(
    store: &mut Store<PluginStoreData>,
    inst: &FreshInstance,
) -> Result<Vec<SearchFilterDescriptor>, PluginError> {
    let plugin = store.data().plugin_name.clone();
    let Some(idx) = inst
        .raw
        .get_export(&mut *store, None, SEARCH_FILTERS_EXPORT)
    else {
        log::debug!(
            target: &store.data().log_target,
            "plugin exports no `{SEARCH_FILTERS_EXPORT}` (pre-0.5.1); treating as no filters"
        );
        return Ok(Vec::new());
    };
    let trapped = |stage: &str, e: wasmtime::Error| PluginError::Trapped {
        plugin: plugin.clone(),
        reason: format!("{stage} {SEARCH_FILTERS_EXPORT}: {e}"),
    };
    let func = inst
        .raw
        .get_typed_func::<(), (Vec<WitSearchFilterDescriptor>,)>(&mut *store, idx)
        .map_err(|e| trapped("signature of", e))?;
    let (descs,) = func
        .call_async(&mut *store, ())
        .await
        .map_err(|e| trapped("call", e))?;
    func.post_return_async(&mut *store)
        .await
        .map_err(|e| trapped("post-return", e))?;
    Ok(descriptors_from_wit(descs, &store.data().origin()))
}

/// Convert a plugin's descriptor list, enforcing the three bounds above:
/// descriptors past [`MAX_SEARCH_FILTER_DESCRIPTORS`] are dropped, a
/// descriptor with an over-long key/name/default is dropped whole, and an
/// over-long or over-count allowed value is dropped from its descriptor.
/// Each refusal warns once on the plugin's log target, naming the bound.
fn descriptors_from_wit(
    descs: Vec<WitSearchFilterDescriptor>,
    origin: &PluginOrigin<'_>,
) -> Vec<SearchFilterDescriptor> {
    let total = descs.len();
    if total > MAX_SEARCH_FILTER_DESCRIPTORS {
        log::warn!(
            target: origin.log_target,
            "search-filters: plugin {} declared {total} descriptors; keeping the first {MAX_SEARCH_FILTER_DESCRIPTORS}",
            origin.plugin_name
        );
    }
    descs
        .into_iter()
        .take(MAX_SEARCH_FILTER_DESCRIPTORS)
        .filter_map(|d| descriptor_from_wit(d, origin))
        .collect()
}

/// Whether a descriptor string fits [`MAX_SEARCH_FILTER_STRING_BYTES`].
const fn fits(s: &str) -> bool {
    s.len() <= MAX_SEARCH_FILTER_STRING_BYTES
}

/// Call the generated `search` export and map its `search-error`.
pub(crate) async fn call_search(
    store: &mut Store<PluginStoreData>,
    inst: &FreshInstance,
    query: &WitSearchQuery,
) -> Result<WitSearchPage, PluginError> {
    let plugin = store.data().plugin_name.clone();
    let wit_result = inst
        .host
        .call_search(&mut *store, query)
        .await
        .map_err(|e| PluginError::Trapped {
            plugin: plugin.clone(),
            reason: format!("call_search: {e}"),
        })?;
    wit_result.map_err(|e| search_error_to_plugin_error(&plugin, e))
}

/// Map a WIT `search-error` to a `PluginError`. `unsupported` is the one
/// search-only case; the rest share [`common_plugin_error`] with the
/// extract mapper so both paths produce identical variants.
fn search_error_to_plugin_error(plugin: &str, err: WitSearchError) -> PluginError {
    let plugin = plugin.to_string();
    let common = match err {
        WitSearchError::Unsupported => return PluginError::SearchUnsupported { plugin },
        WitSearchError::RateLimited(retry_after) => CommonPluginErr::RateLimited(retry_after),
        WitSearchError::Network(detail) => CommonPluginErr::Network(detail),
        WitSearchError::Parse(detail) => CommonPluginErr::Parse(detail),
        WitSearchError::Cancelled => CommonPluginErr::Cancelled,
        WitSearchError::Internal(detail) => CommonPluginErr::Internal(detail),
    };
    common_plugin_error(plugin, common)
}

/// The WIT record carries values only; the host renders each label as its
/// value (documented on the `search-filter-descriptor` record). `None` when
/// the descriptor's key, display name, or default exceeds
/// [`MAX_SEARCH_FILTER_STRING_BYTES`] — such a descriptor is dropped whole,
/// since every one of those strings is echoed into operator-facing text.
fn descriptor_from_wit(
    d: WitSearchFilterDescriptor,
    origin: &PluginOrigin<'_>,
) -> Option<SearchFilterDescriptor> {
    if !fits(&d.key) || !fits(&d.display_name) || !d.default.as_deref().is_none_or(fits) {
        log::warn!(
            target: origin.log_target,
            "search-filters: plugin {} declared a descriptor with a key, display name, or default over {MAX_SEARCH_FILTER_STRING_BYTES} bytes; dropping it",
            origin.plugin_name
        );
        return None;
    }
    let declared = d.allowed_values.len();
    let mut kept = 0usize;
    let mut oversize = 0usize;
    let allowed = SearchFilterValue::list(
        d.allowed_values
            .iter()
            .filter(|v| {
                let ok = fits(v);
                if !ok {
                    oversize += 1;
                }
                ok
            })
            .take(MAX_SEARCH_FILTER_VALUES)
            .inspect(|_| kept += 1)
            .map(|v| (v.as_str(), v.as_str())),
    );
    if oversize > 0 || declared - oversize > kept {
        log::warn!(
            target: origin.log_target,
            "search-filters: plugin {} filter '{}' declared {declared} allowed values; kept {kept} (dropped {oversize} over {MAX_SEARCH_FILTER_STRING_BYTES} bytes, the rest past {MAX_SEARCH_FILTER_VALUES})",
            origin.plugin_name,
            d.key
        );
    }
    Some(SearchFilterDescriptor::new(
        d.key,
        d.display_name,
        allowed,
        d.default.as_deref(),
    ))
}

/// The host's page request for the plugin: the scaffold has already chosen
/// `page`, so `SearchQuery::page` (which may be `None` for "all pages") is
/// replaced rather than forwarded. Filters keep their CLI order.
pub(crate) fn search_query_to_wit(q: &SearchQuery, page: u32) -> WitSearchQuery {
    WitSearchQuery {
        query: q.query.clone(),
        page: Some(page),
        filters: q
            .filters
            .iter()
            .map(|f| (f.key.clone(), f.value.clone()))
            .collect(),
    }
}

/// The plugin's page as the scaffold consumes it. The WIT page's own
/// `page` number is dropped: the scaffold echoes the page it asked for, so
/// a plugin cannot report a different one.
pub(crate) fn search_page_from_wit(p: WitSearchPage) -> SearchPage {
    SearchPage {
        results: p.results.into_iter().map(result_from_wit).collect(),
        has_more: p.has_more,
        total_estimate: p.total_estimate,
    }
}

/// The WIT `search-result` carries the five preview fields a card can
/// show; the rest of the preview stays unset for `enrich` to fill.
fn result_from_wit(r: WitSearchResult) -> SearchResultPreview {
    SearchResultPreview {
        video_url: r.url,
        title: r.title,
        thumbnail_url: r.thumbnail,
        duration: r.duration.map(f64::from),
        uploader: r.uploader,
        uploader_url: None,
        actors: Vec::new(),
        view_count: None,
        upload_date: None,
    }
}

/// A search call's failure as the scaffold reports it. Every `PluginError`
/// already renders its own operator message — including
/// [`PluginError::SearchUnsupported`], whose `Display` names the plugin —
/// so one mapping covers them all. No URL: a search has no subject URL.
fn search_call_error(e: PluginError) -> RdlpError {
    plugin_error_to_rdlp(e, None)
}

/// A loaded plugin presented as a search site.
///
/// Implements `SearchExtractor` for the registry and [`PagedSearch`] for the
/// shared scaffold, which supplies everything a built-in site gets:
/// filter validation against the plugin's own descriptors, page iteration
/// from page 1, `max_results`, inter-page pacing, duplicate-page
/// termination, and the first-page/later-page error asymmetry. The plugin
/// itself only answers `search` for one page at a time.
///
/// `filters` caches the plugin's `search-filters` descriptors. The cache
/// exists because [`PagedSearch::validate_search_filters`] is synchronous
/// (every built-in's descriptor table is static) while a plugin's
/// descriptors come from an async wasm call. `search` and `search_page`
/// prime it before delegating to the scaffold, so by the time the
/// scaffold validates, the sync method reads a populated cell; a call that
/// bypasses those entry points sees no descriptors and rejects any filter
/// as unknown rather than reaching into wasm from a sync context.
pub struct PluginSearchExtractor {
    inner: Arc<PluginExtractor>,
    filters: OnceCell<Vec<SearchFilterDescriptor>>,
}

impl PluginSearchExtractor {
    /// Wrap a loaded plugin; descriptors are fetched on first use.
    #[must_use]
    pub fn new(inner: Arc<PluginExtractor>) -> Self {
        Self {
            inner,
            filters: OnceCell::new(),
        }
    }

    /// A wrapper whose descriptor cache is already populated, so the
    /// validator can be exercised without a component that exports
    /// `search-filters`.
    #[cfg(test)]
    fn with_filters(inner: Arc<PluginExtractor>, filters: Vec<SearchFilterDescriptor>) -> Self {
        Self {
            inner,
            filters: OnceCell::new_with(Some(filters)),
        }
    }

    /// The cached descriptors, fetching them from the plugin on the first
    /// call. A failed fetch is reported as no filters and NOT cached, so a
    /// transient failure is retried next time; a plugin that traps here
    /// keeps paying strikes through the runner until the 3-strike rule
    /// disables it.
    async fn filters_or_fetch(&self) -> Vec<SearchFilterDescriptor> {
        let fetched = self
            .filters
            .get_or_try_init(|| self.inner.call_search_filters())
            .await;
        match fetched {
            Ok(filters) => filters.clone(),
            Err(e) => {
                log::warn!(
                    "plugin '{}' search-filters failed; treating as no filters, so every \
                     search filter will be rejected as unknown until the plugin answers: {e:#}",
                    self.inner.manifest.name
                );
                Vec::new()
            }
        }
    }
}

#[async_trait]
impl SearchExtractor for PluginSearchExtractor {
    fn name(&self) -> &str {
        self.inner.manifest.search_site_name()
    }

    async fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        self.filters_or_fetch().await
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> rdlp_core::Result<Vec<SearchResultPreview>> {
        self.filters_or_fetch().await;
        self.search_all_pages(query, ctx).await
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> rdlp_core::Result<SearchPageResponse> {
        self.filters_or_fetch().await;
        self.search_page_response(query, ctx).await
    }

    fn is_plugin(&self) -> bool {
        true
    }

    fn search_priority(&self) -> i32 {
        self.inner.plugin_priority()
    }

    fn overrides_builtin(&self) -> bool {
        self.inner.manifest.overrides_builtin_search()
    }
}

impl PagedSearch for PluginSearchExtractor {
    /// Validates against whatever the cache holds; see the type docs for
    /// why the cache, not the plugin, is consulted here. Wording is the
    /// shared Family-1 form with the manifest's site name.
    fn validate_search_filters(&self, filters: &[SearchFilter]) -> rdlp_core::Result<()> {
        let descriptors = self.filters.get().map_or(&[][..], Vec::as_slice);
        validate_against_descriptors(filters, descriptors, &[])
            .map_err(|e| format_std_filter_error(SearchExtractor::name(self), e))
    }

    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        _ctx: &ExtractionContext,
    ) -> rdlp_core::Result<SearchPage> {
        let page = self
            .inner
            .call_search(search_query_to_wit(query, page))
            .await
            .map_err(search_call_error)?;
        Ok(search_page_from_wit(page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_is_the_search_only_variant() {
        assert!(matches!(
            search_error_to_plugin_error("example", WitSearchError::Unsupported),
            PluginError::SearchUnsupported { ref plugin } if plugin == "example"
        ));
    }

    #[test]
    fn shared_cases_map_like_the_extract_mapper() {
        assert!(matches!(
            search_error_to_plugin_error("example", WitSearchError::RateLimited(Some(7))),
            PluginError::RateLimited {
                retry_after: Some(7),
                ..
            }
        ));
        assert!(matches!(
            search_error_to_plugin_error("example", WitSearchError::Network("dns".into())),
            PluginError::ExtractNetwork { ref detail, .. } if detail == "dns"
        ));
        assert!(matches!(
            search_error_to_plugin_error("example", WitSearchError::Parse("html".into())),
            PluginError::ExtractParse { ref detail, .. } if detail == "html"
        ));
        assert!(matches!(
            search_error_to_plugin_error("example", WitSearchError::Cancelled),
            PluginError::Cancelled { .. }
        ));
        match search_error_to_plugin_error("example", WitSearchError::Internal("bad".into())) {
            PluginError::Internal(msg) => assert_eq!(msg, "plugin example: bad"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn only_internal_among_the_search_errors_is_a_strike() {
        use crate::adapter::counts_as_strike;
        let cases = [
            (WitSearchError::Unsupported, false),
            (WitSearchError::RateLimited(None), false),
            (WitSearchError::Network(String::new()), false),
            (WitSearchError::Parse(String::new()), false),
            (WitSearchError::Cancelled, false),
            (WitSearchError::Internal(String::new()), true),
        ];
        for (err, strike) in cases {
            let mapped = search_error_to_plugin_error("example", err);
            assert_eq!(counts_as_strike(&mapped), strike, "{mapped:?}");
        }
    }

    /// Drift guard for the hand-written lift: the record's field lines in
    /// `wit/types.wit` must be exactly these, in this order — the component
    /// canonical ABI lifts records positionally, so a reordered or retyped
    /// field would lift garbage without a type error.
    #[test]
    fn lift_mirrors_the_wit_record_field_for_field() {
        const TYPES_WIT: &str = include_str!("../wit/types.wit");
        let expected = [
            "key: string,",
            "display-name: string,",
            "allowed-values: list<string>,",
            "default: option<string>,",
        ];
        let (_, after) = TYPES_WIT
            .split_once("record search-filter-descriptor {")
            .expect("types.wit declares search-filter-descriptor");
        let (body, _) = after.split_once('}').expect("record body is closed");
        let fields: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(
            fields, expected,
            "search-filter-descriptor drifted from the Rust lift"
        );
    }

    #[test]
    fn descriptor_labels_are_the_values() {
        let d = descriptor_from_wit(
            WitSearchFilterDescriptor {
                key: "ordering".into(),
                display_name: "Ordering".into(),
                allowed_values: vec!["newest".into(), "views".into()],
                default: Some("newest".into()),
            },
            &test_origin(),
        )
        .expect("within every bound");
        assert_eq!(d.key, "ordering");
        assert_eq!(d.display_name, "Ordering");
        assert_eq!(d.default.as_deref(), Some("newest"));
        assert_eq!(
            d.allowed_values,
            SearchFilterValue::list([("newest", "newest"), ("views", "views")])
        );
    }

    // ── PluginSearchExtractor ─────────────────────────────────────────────

    use crate::test_support::unit::{
        FIXTURE_MANIFEST, TEST_LOG_TARGET, captured_entry_containing, captured_logs,
        fixture_extractor, fixture_extractor_with_manifest, test_origin,
    };

    fn host_query(filters: &[(&str, &str)]) -> SearchQuery {
        SearchQuery {
            query: "kittens".into(),
            filters: filters
                .iter()
                .map(|(k, v)| SearchFilter {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            max_results: None,
            page: None,
        }
    }

    #[test]
    fn query_to_wit_sets_the_page_and_keeps_filter_tuples_in_order() {
        let q = host_query(&[("sort", "top"), ("period", "week")]);
        let wit = search_query_to_wit(&q, 3);
        assert_eq!(wit.query, "kittens");
        assert_eq!(wit.page, Some(3));
        assert_eq!(
            wit.filters,
            vec![
                ("sort".to_string(), "top".to_string()),
                ("period".to_string(), "week".to_string()),
            ]
        );
    }

    #[test]
    fn page_from_wit_maps_every_result_field_and_the_page_flags() {
        let page = search_page_from_wit(WitSearchPage {
            results: vec![WitSearchResult {
                url: "https://example.com/video/1".into(),
                title: "One".into(),
                thumbnail: Some("https://example.com/1.jpg".into()),
                duration: Some(90),
                uploader: Some("someone".into()),
            }],
            page: 1,
            has_more: true,
            total_estimate: Some(42),
        });
        assert!(page.has_more);
        assert_eq!(page.total_estimate, Some(42));
        let [r] = page.results.as_slice() else {
            panic!("one result expected: {:?}", page.results)
        };
        assert_eq!(r.video_url, "https://example.com/video/1");
        assert_eq!(r.title, "One");
        assert_eq!(
            r.thumbnail_url.as_deref(),
            Some("https://example.com/1.jpg")
        );
        assert_eq!(r.duration, Some(90.0));
        assert_eq!(r.uploader.as_deref(), Some("someone"));
        // The WIT record carries none of these; they must stay unset.
        assert_eq!(r.uploader_url, None);
        assert!(r.actors.is_empty());
        assert_eq!(r.view_count, None);
        assert_eq!(r.upload_date, None);
    }

    #[test]
    fn page_from_wit_keeps_absent_optionals_absent() {
        let page = search_page_from_wit(WitSearchPage {
            results: vec![WitSearchResult {
                url: "https://example.com/video/2".into(),
                title: "Two".into(),
                thumbnail: None,
                duration: None,
                uploader: None,
            }],
            page: 2,
            has_more: false,
            total_estimate: None,
        });
        assert!(!page.has_more);
        assert_eq!(page.total_estimate, None);
        let [r] = page.results.as_slice() else {
            panic!("one result expected: {:?}", page.results)
        };
        assert_eq!(r.thumbnail_url, None);
        assert_eq!(r.duration, None);
        assert_eq!(r.uploader, None);
    }

    fn sort_descriptor() -> SearchFilterDescriptor {
        SearchFilterDescriptor::new(
            "sort",
            "Sort",
            SearchFilterValue::list([("top", "top")]),
            None,
        )
    }

    /// The fixture is a 0.5.0 component with NO `search-filters` export, so
    /// a validator that reached the wasm would see zero descriptors and
    /// report `sort` as an unknown key. `InvalidValue` proves the cached
    /// descriptors were consulted instead.
    #[test]
    fn validation_reads_the_cached_descriptors_not_the_component() {
        let ext = PluginSearchExtractor::with_filters(
            Arc::new(fixture_extractor()),
            vec![sort_descriptor()],
        );
        let err = ext
            .validate_search_filters(&host_query(&[("sort", "new")]).filters)
            .expect_err("out-of-set value");
        match err {
            RdlpError::Extraction { message, url } => {
                assert_eq!(
                    message,
                    "Invalid value 'new' for filter 'sort'. Allowed: top"
                );
                assert_eq!(url, None);
            }
            other => panic!("got {other:?}"),
        }
        assert_eq!(ext.inner.test_trap_count(), 0);
    }

    #[test]
    fn validation_rejects_an_unknown_key_naming_the_site() {
        let ext = PluginSearchExtractor::with_filters(
            Arc::new(fixture_extractor()),
            vec![sort_descriptor()],
        );
        let err = ext
            .validate_search_filters(&host_query(&[("foo", "x")]).filters)
            .expect_err("unknown key");
        match err {
            RdlpError::Extraction { message, .. } => {
                assert_eq!(message, "Unknown filter 'foo' for example. Available: sort");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn validation_accepts_an_allowed_value_and_no_filters() {
        let ext = PluginSearchExtractor::with_filters(
            Arc::new(fixture_extractor()),
            vec![sort_descriptor()],
        );
        ext.validate_search_filters(&host_query(&[("sort", "top")]).filters)
            .expect("allowed value");
        ext.validate_search_filters(&[]).expect("no filters");
    }

    /// Before `search`/`search_page` prime the cache, the sync validator
    /// has no descriptors: every filter is unknown, none is not.
    #[test]
    fn an_unprimed_cache_treats_every_filter_as_unknown() {
        let ext = PluginSearchExtractor::new(Arc::new(fixture_extractor()));
        ext.validate_search_filters(&[]).expect("no filters");
        let err = ext
            .validate_search_filters(&host_query(&[("sort", "top")]).filters)
            .expect_err("nothing is known yet");
        match err {
            RdlpError::Extraction { message, .. } => {
                assert_eq!(message, "Unknown filter 'sort' for example. Available: ");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn identity_and_arbitration_come_from_the_manifest() {
        let plain = PluginSearchExtractor::new(Arc::new(fixture_extractor()));
        assert_eq!(SearchExtractor::name(&plain), "example");
        assert!(plain.is_plugin());
        assert_eq!(plain.search_priority(), 150);
        assert!(!plain.overrides_builtin());
        assert_eq!(plain.first_page_index(), 1);

        let ext = PluginSearchExtractor::new(Arc::new(fixture_extractor_with_manifest(
            &manifest_claiming("pornhub", "search_claims_override = [\"pornhub\"]"),
        )));
        assert_eq!(SearchExtractor::name(&ext), "pornhub");
        assert!(ext.overrides_builtin());
    }

    // ── search override binding (security M3) ─────────────────────────────

    /// The fixture manifest serving `site` for search, with `claim` lines
    /// appended verbatim.
    fn manifest_claiming(site: &str, claim: &str) -> String {
        FIXTURE_MANIFEST.replace(
            "capabilities = []",
            &format!(
                "capabilities = []\nsupports_search = true\nsearch_site = \"{site}\"\n{claim}"
            ),
        )
    }

    /// The shadowing attempt: a plugin naming a built-in's site with only a
    /// URL-routing `claims_override` (for a host it does match) claims
    /// nothing about search — the override must be the explicit,
    /// site-bound `search_claims_override`.
    #[test]
    fn a_url_claims_override_alone_does_not_override_builtin_search() {
        let ext = PluginSearchExtractor::new(Arc::new(fixture_extractor_with_manifest(
            &manifest_claiming("pornhub", "claims_override = [\"example.com\"]"),
        )));
        assert_eq!(SearchExtractor::name(&ext), "pornhub");
        assert!(!ext.overrides_builtin());
    }

    /// End to end through the real registry: against the built-in
    /// `pornhub`, the URL-only claimant LOSES and the site-bound claimant
    /// WINS — the registry consults exactly `overrides_builtin`.
    #[test]
    fn registry_arbitration_honours_only_the_site_bound_claim() {
        use rdlp_extractor::ExtractorRegistry;

        let mut reg = ExtractorRegistry::new();
        reg.register_search(Arc::new(PluginSearchExtractor::new(Arc::new(
            fixture_extractor_with_manifest(&manifest_claiming(
                "pornhub",
                "claims_override = [\"example.com\"]",
            )),
        ))));
        let found = reg
            .find_search_extractor("pornhub")
            .expect("the built-in exists");
        assert!(
            !found.is_plugin(),
            "a URL-only claimant must lose to the built-in"
        );

        let mut reg = ExtractorRegistry::new();
        reg.register_search(Arc::new(PluginSearchExtractor::new(Arc::new(
            fixture_extractor_with_manifest(&manifest_claiming(
                "pornhub",
                "search_claims_override = [\"pornhub\"]",
            )),
        ))));
        let found = reg.find_search_extractor("pornhub").expect("a match");
        assert!(
            found.is_plugin(),
            "the site-bound claimant must shadow the built-in"
        );
    }

    /// A plugin the runner refuses (here: disabled by the 3-strike rule)
    /// answers no filters — and the failure is NOT cached, so the cell
    /// stays empty for a retry once the plugin can answer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_filters_fetch_reports_no_filters_and_is_not_cached() {
        let inner = Arc::new(fixture_extractor());
        for _ in 0..crate::adapter::TRAP_DISABLE_THRESHOLD {
            inner.test_record_trap();
        }
        assert!(inner.test_is_disabled());
        let ext = PluginSearchExtractor::new(inner);
        assert!(ext.supported_filters().await.is_empty());
        assert!(ext.filters.get().is_none(), "a failure must not be cached");
    }

    /// `SearchUnsupported`'s `Display` IS the operator message, so the
    /// generic mapper needs no special arm; this pins the wording the
    /// operator sees for a plugin that exports `search` only to satisfy
    /// the world.
    #[test]
    fn search_unsupported_reaches_the_operator_by_name() {
        let err = search_call_error(PluginError::SearchUnsupported {
            plugin: "example".into(),
        });
        match err {
            RdlpError::Extraction { message, url } => {
                assert_eq!(message, "plugin 'example' does not support search");
                assert_eq!(url, None);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn descriptor_without_default_or_values_is_preserved() {
        let d = descriptor_from_wit(
            WitSearchFilterDescriptor {
                key: "k".into(),
                display_name: "K".into(),
                allowed_values: vec![],
                default: None,
            },
            &test_origin(),
        )
        .expect("within every bound");
        assert!(d.allowed_values.is_empty());
        assert_eq!(d.default, None);
    }

    // ── descriptor bounds (security L3) ───────────────────────────────────

    fn descriptor(key: &str, values: usize) -> WitSearchFilterDescriptor {
        WitSearchFilterDescriptor {
            key: key.into(),
            display_name: key.to_uppercase(),
            allowed_values: (0..values).map(|i| format!("v{i}")).collect(),
            default: None,
        }
    }

    /// Exactly `MAX_SEARCH_FILTER_DESCRIPTORS` descriptors all survive; one
    /// more is cut back to the bound, with the first ones kept, and the cut
    /// is reported on the plugin's log target.
    #[test]
    fn descriptor_count_is_capped_at_the_bound_inclusive() {
        let logs = captured_logs();
        let at = descriptors_from_wit(
            (0..MAX_SEARCH_FILTER_DESCRIPTORS)
                .map(|i| descriptor(&format!("k{i}"), 1))
                .collect(),
            &test_origin(),
        );
        assert_eq!(at.len(), MAX_SEARCH_FILTER_DESCRIPTORS);

        let over = descriptors_from_wit(
            (0..=MAX_SEARCH_FILTER_DESCRIPTORS)
                .map(|i| descriptor(&format!("k{i}"), 1))
                .collect(),
            &test_origin(),
        );
        assert_eq!(over.len(), MAX_SEARCH_FILTER_DESCRIPTORS);
        assert_eq!(
            over.first().map(|d| d.key.as_str()),
            Some("k0"),
            "the first descriptors are the ones kept"
        );
        let (target, msg) = captured_entry_containing(
            &logs,
            &format!("declared {} descriptors", MAX_SEARCH_FILTER_DESCRIPTORS + 1),
        );
        assert_eq!(target, TEST_LOG_TARGET);
        assert!(
            msg.contains(&MAX_SEARCH_FILTER_DESCRIPTORS.to_string()),
            "{msg}"
        );
    }

    /// Exactly `MAX_SEARCH_FILTER_VALUES` allowed values survive; one more is
    /// cut back to the bound (first ones kept) with a warning naming the key.
    #[test]
    fn allowed_values_are_capped_at_the_bound_inclusive() {
        let logs = captured_logs();
        let at = descriptor_from_wit(descriptor("at", MAX_SEARCH_FILTER_VALUES), &test_origin())
            .expect("a full list is within the bound");
        assert_eq!(at.allowed_values.len(), MAX_SEARCH_FILTER_VALUES);

        let over = descriptor_from_wit(
            descriptor("over", MAX_SEARCH_FILTER_VALUES + 1),
            &test_origin(),
        )
        .expect("an over-long list is cut, not refused");
        assert_eq!(over.allowed_values.len(), MAX_SEARCH_FILTER_VALUES);
        assert_eq!(
            over.allowed_values.first().map(|v| v.value.as_str()),
            Some("v0")
        );
        let (target, msg) = captured_entry_containing(
            &logs,
            &format!(
                "filter 'over' declared {} allowed values",
                MAX_SEARCH_FILTER_VALUES + 1
            ),
        );
        assert_eq!(target, TEST_LOG_TARGET);
        assert!(
            msg.contains(&format!("kept {MAX_SEARCH_FILTER_VALUES}")),
            "{msg}"
        );
    }

    /// A key of exactly `MAX_SEARCH_FILTER_STRING_BYTES` is accepted; one
    /// byte more drops the whole descriptor. Same bound, applied per field:
    /// the display name and the default are checked the same way.
    #[test]
    fn descriptor_strings_are_bounded_inclusive_per_field() {
        let logs = captured_logs();
        let at = "k".repeat(MAX_SEARCH_FILTER_STRING_BYTES);
        let over = "k".repeat(MAX_SEARCH_FILTER_STRING_BYTES + 1);
        assert!(descriptor_from_wit(descriptor(&at, 1), &test_origin()).is_some());
        assert!(descriptor_from_wit(descriptor(&over, 1), &test_origin()).is_none());

        let mut long_name = descriptor("k", 1);
        long_name.display_name = over.clone();
        assert!(descriptor_from_wit(long_name, &test_origin()).is_none());

        let mut long_default = descriptor("k", 1);
        long_default.default = Some(over);
        assert!(descriptor_from_wit(long_default, &test_origin()).is_none());

        let (target, msg) = captured_entry_containing(&logs, "key, display name, or default over");
        assert_eq!(target, TEST_LOG_TARGET);
        assert!(
            msg.contains(&MAX_SEARCH_FILTER_STRING_BYTES.to_string()),
            "{msg}"
        );
    }

    /// An over-long allowed value drops only itself, not the descriptor.
    #[test]
    fn an_over_long_allowed_value_is_dropped_from_its_descriptor() {
        let mut d = descriptor("k", 2);
        d.allowed_values
            .push("x".repeat(MAX_SEARCH_FILTER_STRING_BYTES + 1));
        let converted = descriptor_from_wit(d, &test_origin()).expect("descriptor survives");
        assert_eq!(
            converted
                .allowed_values
                .iter()
                .map(|v| v.value.as_str())
                .collect::<Vec<_>>(),
            ["v0", "v1"]
        );
    }
}
