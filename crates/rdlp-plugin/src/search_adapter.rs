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

use wasmtime::Store;

use crate::PluginError;
use crate::adapter::{
    CallSpec, CommonPluginErr, FreshInstance, PluginExtractor, SEARCH_TIMEOUT, common_plugin_error,
};
use crate::bindings::rdlp::plugin::types::{
    SearchError as WitSearchError, SearchPage as WitSearchPage, SearchQuery as WitSearchQuery,
};
use crate::instance::PluginStoreData;
use rdlp_types::{SearchFilterDescriptor, SearchFilterValue};
use wasmtime::component::{ComponentType, Lift};

/// Name of the optional world export, as declared in `wit/extractor.wit`.
const SEARCH_FILTERS_EXPORT: &str = "search-filters";

/// Hand-written lift of the `search-filter-descriptor` record from
/// `wit/types.wit`. `bindgen!` emits only the types the bound
/// `extractor-plugin-host` world references, and that world deliberately
/// omits `search-filters` — so the record it returns is not generated and
/// must be declared here, field-for-field, for the typed by-name call.
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
            url_for_errors: self.manifest.search_site_name(),
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
            url_for_errors: self.manifest.search_site_name(),
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
    Ok(descs.into_iter().map(descriptor_from_wit).collect())
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
/// value (documented on the `search-filter-descriptor` record).
fn descriptor_from_wit(d: WitSearchFilterDescriptor) -> SearchFilterDescriptor {
    let allowed =
        SearchFilterValue::list(d.allowed_values.iter().map(|v| (v.as_str(), v.as_str())));
    SearchFilterDescriptor::new(d.key, d.display_name, allowed, d.default.as_deref())
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

    #[test]
    fn descriptor_labels_are_the_values() {
        let d = descriptor_from_wit(WitSearchFilterDescriptor {
            key: "ordering".into(),
            display_name: "Ordering".into(),
            allowed_values: vec!["newest".into(), "views".into()],
            default: Some("newest".into()),
        });
        assert_eq!(d.key, "ordering");
        assert_eq!(d.display_name, "Ordering");
        assert_eq!(d.default.as_deref(), Some("newest"));
        assert_eq!(
            d.allowed_values,
            SearchFilterValue::list([("newest", "newest"), ("views", "views")])
        );
    }

    #[test]
    fn descriptor_without_default_or_values_is_preserved() {
        let d = descriptor_from_wit(WitSearchFilterDescriptor {
            key: "k".into(),
            display_name: "K".into(),
            allowed_values: vec![],
            default: None,
        });
        assert!(d.allowed_values.is_empty());
        assert_eq!(d.default, None);
    }
}
