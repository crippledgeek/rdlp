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
    CallSpec, CommonPluginErr, ExportCall, FreshInstance, PluginExtractor, SEARCH_TIMEOUT,
    TimeoutStrikes, call_export_by_name, common_plugin_error, plugin_error_to_rdlp,
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
/// The largest in-tree table (xHamster) has six; 32 leaves room for a far
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
/// Two tests pin it: `tests::lift_mirrors_the_wit_record_field_for_field`
/// checks the record's source text in `wit/types.wit` against the field
/// list this struct must mirror, and
/// `tests::a_correctly_typed_export_lifts_and_post_returns` lifts a
/// hand-laid-out record through the canonical ABI via the typed by-name
/// call — a drifted field order or type fails one of them before it can
/// mis-lift at runtime.
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
            timeout_strikes: TimeoutStrikes::Always,
        };
        self.run_in_fresh_store(spec, |store, inst| {
            Box::pin(call_search_filters(store, &inst.raw))
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
            timeout_strikes: TimeoutStrikes::Always,
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
/// `call_export_by_name` already logs the absent-export case once, at
/// debug; this function does not log it again.
///
/// Takes the raw instance rather than a [`FreshInstance`] because the
/// by-name lookup is all it needs — which also lets a unit test drive it
/// with a minimal component that exports only `search-filters`.
pub(crate) async fn call_search_filters(
    store: &mut Store<PluginStoreData>,
    inst: &wasmtime::component::Instance,
) -> Result<Vec<SearchFilterDescriptor>, PluginError> {
    let out = call_export_by_name::<(), (Vec<WitSearchFilterDescriptor>,)>(
        store,
        inst,
        ExportCall {
            name: SEARCH_FILTERS_EXPORT,
            params: (),
        },
    )
    .await?;
    let Some((descs,)) = out else {
        return Ok(Vec::new());
    };
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

/// Why a descriptor string was refused: over
/// [`MAX_SEARCH_FILTER_STRING_BYTES`], or carrying a control character.
/// Every descriptor string is echoed into operator-facing error text and
/// logs, so a terminal escape (CWE-150) or a line break in one must never
/// get that far — `char::is_control` is the same stdlib predicate rdlp-api
/// applies to its own operator-visible names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringRefusal {
    TooLong,
    ControlCharacter,
}

/// Whether a descriptor string is admissible, else why not.
fn check_string(s: &str) -> Result<(), StringRefusal> {
    if s.len() > MAX_SEARCH_FILTER_STRING_BYTES {
        Err(StringRefusal::TooLong)
    } else if s.chars().any(char::is_control) {
        Err(StringRefusal::ControlCharacter)
    } else {
        Ok(())
    }
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
/// the descriptor's key, display name, or default is refused by
/// [`check_string`] — such a descriptor is dropped whole, since every one
/// of those strings is echoed into operator-facing text. A refused allowed
/// value drops only itself. The warning never echoes the offending text
/// (it may be the very escape being refused); it names the bound instead.
fn descriptor_from_wit(
    d: WitSearchFilterDescriptor,
    origin: &PluginOrigin<'_>,
) -> Option<SearchFilterDescriptor> {
    let identity_refusal = check_string(&d.key)
        .and_then(|()| check_string(&d.display_name))
        .and_then(|()| d.default.as_deref().map_or(Ok(()), check_string));
    if let Err(why) = identity_refusal {
        log::warn!(
            target: origin.log_target,
            "search-filters: plugin {} declared a descriptor whose key, display name, or default is {}; dropping it",
            origin.plugin_name,
            refusal_text(why)
        );
        return None;
    }
    let mut tally = ValueTally::default();
    let allowed = SearchFilterValue::list(
        d.allowed_values
            .iter()
            .filter(|v| tally.admit(v))
            .map(|v| (v.as_str(), v.as_str())),
    );
    if tally.dropped() > 0 {
        log::warn!(
            target: origin.log_target,
            "search-filters: plugin {} filter '{}' declared {} allowed values; kept {} (dropped {} over {MAX_SEARCH_FILTER_STRING_BYTES} bytes, {} with a control character, {} past the {MAX_SEARCH_FILTER_VALUES}-value bound)",
            origin.plugin_name,
            d.key,
            d.allowed_values.len(),
            tally.kept,
            tally.too_long,
            tally.control,
            tally.over_count
        );
    }
    Some(SearchFilterDescriptor::new(
        d.key,
        d.display_name,
        allowed,
        d.default.as_deref(),
    ))
}

/// The bound a refused string broke, for the warning.
const fn refusal_text(why: StringRefusal) -> &'static str {
    match why {
        StringRefusal::TooLong => "over the byte-length bound",
        StringRefusal::ControlCharacter => "carrying a control character",
    }
}

/// Per-descriptor accounting of `allowed-values` admissions, so the warning
/// reports each dropped value under its actual reason: a value refused on
/// its own account is never counted as "past the count bound", and the
/// count bound applies to admissible values only.
#[derive(Debug, Default)]
struct ValueTally {
    kept: usize,
    too_long: usize,
    control: usize,
    over_count: usize,
}

impl ValueTally {
    /// Whether `value` is admitted, recording why not otherwise.
    fn admit(&mut self, value: &str) -> bool {
        match check_string(value) {
            Err(StringRefusal::TooLong) => self.too_long += 1,
            Err(StringRefusal::ControlCharacter) => self.control += 1,
            Ok(()) if self.kept >= MAX_SEARCH_FILTER_VALUES => self.over_count += 1,
            Ok(()) => {
                self.kept += 1;
                return true;
            }
        }
        false
    }

    const fn dropped(&self) -> usize {
        self.too_long + self.control + self.over_count
    }
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
mod tests;
