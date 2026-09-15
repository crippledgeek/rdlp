//! By-name resolution of the 0.5.2 `extract-playlist` export.
//!
//! `wit/COMPATIBILITY.md` §3/§8. The host binds the frozen
//! `extractor-plugin-host` bindings, which omit every post-0.5.0 export, so
//! this export is looked up on the live instance through
//! `crate::adapter::call_export_by_name` — the same mechanism
//! `search_adapter::call_search_filters` uses.
//!
//! `bindgen!` only emits Rust types for records/variants reachable from a
//! function signature in the *bound* world (`extractor-plugin-host`).
//! Adding `use types.{playlist-page, playlist-error, extraction};` to that
//! world (types only, no export) was tried first, per the task brief's
//! verify-don't-assume step; `cargo check -p rdlp-plugin` still reported
//! `no PlaylistPage/PlaylistError/… in bindings::rdlp::plugin::types` for
//! the fields used below, so the `use` added nothing and was reverted. The
//! three types this module needs — `WitPlaylistPage`, `WitPlaylistEntry`,
//! `WitPlaylistError` — are hand-declared instead, exactly as
//! `search_adapter::WitSearchFilterDescriptor` already is for
//! `search-filter-descriptor`. Each carries a source-text pin test in
//! `tests.rs` against `wit/types.wit`.

use wasmtime::Store;
use wasmtime::component::{ComponentType, Lift};

use crate::PluginError;
use crate::adapter::{
    CallSpec, CommonPluginErr, ExportCall, PluginExtractor, SEARCH_TIMEOUT, call_export_by_name,
    common_plugin_error, plugin_error_to_rdlp,
};
use crate::convert::{PluginOrigin, cap_plugin_playlist_entries};
use crate::instance::PluginStoreData;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result as RdlpResult};
use rdlp_extractor::base::common::{PagedPlaylist, PlaylistEntry, PlaylistPage, PlaylistStart};
use rdlp_types::InfoDict;

const EXTRACT_PLAYLIST_EXPORT: &str = "extract-playlist";

/// Hand-lift of `wit/types.wit`'s `playlist-entry` record. See the module
/// doc for why this is hand-declared rather than bindgen-generated.
#[derive(Debug, Clone, PartialEq, Eq, ComponentType, Lift)]
#[component(record)]
pub(crate) struct WitPlaylistEntry {
    /// URL the host resolves via this plugin's `extract`.
    pub url: String,
    /// Plugin-native item id, if the plugin has one.
    pub id: Option<String>,
    /// Item title, if known without resolving.
    pub title: Option<String>,
}

/// Hand-lift of `wit/types.wit`'s `playlist-page` record.
#[derive(Debug, Clone, PartialEq, Eq, ComponentType, Lift)]
#[component(record)]
pub(crate) struct WitPlaylistPage {
    /// The page's items.
    pub entries: Vec<WitPlaylistEntry>,
    /// 1-indexed page number, matching `search-page`.
    pub page: u32,
    /// Whether a further page exists.
    #[component(name = "has-more")]
    pub has_more: bool,
    /// Playlist id, if the plugin has one.
    #[component(name = "playlist-id")]
    pub playlist_id: Option<String>,
    /// Playlist title, if known.
    #[component(name = "playlist-title")]
    pub playlist_title: Option<String>,
    /// Total item count estimate, if known.
    #[component(name = "total-estimate")]
    pub total_estimate: Option<u64>,
}

/// Hand-lift of `wit/types.wit`'s `playlist-error` variant.
#[derive(Debug, Clone, PartialEq, Eq, ComponentType, Lift)]
#[component(variant)]
pub(crate) enum WitPlaylistError {
    /// The URL is not one this plugin's playlist resolver handles.
    #[component(name = "unsupported-url")]
    UnsupportedUrl(String),
    /// The playlist does not exist upstream.
    #[component(name = "not-found")]
    NotFound(String),
    /// Upstream rate limit, with the plugin-suggested retry delay in seconds.
    #[component(name = "rate-limited")]
    RateLimited(Option<u32>),
    /// Upstream network failure.
    #[component(name = "network")]
    Network(String),
    /// Upstream content did not parse.
    #[component(name = "parse")]
    Parse(String),
    /// A genuine plugin-internal failure.
    #[component(name = "internal")]
    Internal(String),
}

/// The two arguments `call_extract_playlist` needs beyond `store`/`inst`,
/// grouped to keep the call to three positional parameters.
pub(crate) struct PlaylistPageRequest<'a> {
    /// URL to list.
    pub url: &'a str,
    /// 1-indexed page number, matching `search-page`.
    pub page: u32,
}

/// `Ok(None)`: the component never declared the export (pre-0.5.2, or a
/// plugin without playlists). `Ok(Some(Err(_)))`: the plugin answered with a
/// domain error. `Err(Trapped)`: wrong signature, trap, or post-return
/// failure.
pub(crate) async fn call_extract_playlist(
    store: &mut Store<PluginStoreData>,
    inst: &wasmtime::component::Instance,
    request: PlaylistPageRequest<'_>,
) -> Result<Option<Result<WitPlaylistPage, WitPlaylistError>>, PluginError> {
    let out = call_export_by_name::<(String, u32), (Result<WitPlaylistPage, WitPlaylistError>,)>(
        store,
        inst,
        ExportCall {
            name: EXTRACT_PLAYLIST_EXPORT,
            params: (request.url.to_string(), request.page),
        },
    )
    .await?;
    Ok(out.map(|(r,)| r))
}

/// `unsupported-url`/`not-found` are domain outcomes (no strike); the shared
/// cases map through `common_plugin_error` so `internal` alone strikes.
pub(crate) fn playlist_error_to_plugin_error(plugin: &str, e: WitPlaylistError) -> PluginError {
    let common = match e {
        WitPlaylistError::UnsupportedUrl(detail) => {
            return PluginError::UnsupportedUrl {
                plugin: plugin.to_string(),
                detail,
            };
        }
        WitPlaylistError::NotFound(detail) => {
            return PluginError::NotFound {
                plugin: plugin.to_string(),
                detail,
            };
        }
        WitPlaylistError::RateLimited(retry_after) => CommonPluginErr::RateLimited(retry_after),
        WitPlaylistError::Network(detail) => CommonPluginErr::Network(detail),
        WitPlaylistError::Parse(detail) => CommonPluginErr::Parse(detail),
        WitPlaylistError::Internal(detail) => CommonPluginErr::Internal(detail),
    };
    common_plugin_error(plugin.to_string(), common)
}

/// Convert a lifted `playlist-entry` to the host-owned loop's own type.
fn playlist_entry_from_wit(w: WitPlaylistEntry) -> PlaylistEntry {
    PlaylistEntry {
        url: w.url,
        id: w.id,
        title: w.title,
    }
}

/// Convert a lifted `playlist-page` to the host-owned loop's own type,
/// capping `entries` at [`crate::convert::MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES`]
/// (same truncate-and-warn-once shape `info_dict_from_wit` uses for
/// `formats`) so one plugin-controlled page can never hand the loop more
/// rows than it would ever keep.
fn playlist_page_from_wit(w: WitPlaylistPage, origin: &PluginOrigin<'_>) -> PlaylistPage {
    let entries = cap_plugin_playlist_entries(w.entries, EXTRACT_PLAYLIST_EXPORT, origin)
        .into_iter()
        .map(playlist_entry_from_wit)
        .collect();
    PlaylistPage {
        entries,
        has_more: w.has_more,
        playlist_id: w.playlist_id,
        playlist_title: w.playlist_title,
        total_estimate: w.total_estimate,
    }
}

impl PluginExtractor {
    /// One page of the plugin's playlist listing, already converted to the
    /// host-owned loop's own [`PlaylistPage`].
    ///
    /// This is the one place `extract-playlist` is called: both
    /// [`PluginExtractor::extract_playlist`]'s absent/unsupported-url probe
    /// (page one) and [`PluginPlaylistSource::fetch_playlist_page`] (every
    /// later page) go through it, so the fresh-store/timeout/strike
    /// plumbing and the WIT→`PlaylistPage` conversion each exist once.
    /// `Ok(None)`: the component never declared the export.
    ///
    /// # Errors
    ///
    /// The runner's errors ([`PluginError::Disabled`], `Timeout`,
    /// `Trapped`), or `Trapped` when the export exists with an unexpected
    /// signature or traps while running.
    async fn call_extract_playlist_page(
        &self,
        url: &str,
        page: u32,
    ) -> Result<Option<Result<PlaylistPage, WitPlaylistError>>, PluginError> {
        let spec = CallSpec {
            subject_for_errors: url,
            timeout: SEARCH_TIMEOUT,
        };
        // The URL moves into the future: the runner's closure is
        // higher-ranked over the store borrow, so it cannot return a
        // future that also borrows from this frame.
        let owned = url.to_string();
        self.run_in_fresh_store(spec, move |store, inst| {
            Box::pin(async move {
                let raw = call_extract_playlist(
                    store,
                    &inst.raw,
                    PlaylistPageRequest { url: &owned, page },
                )
                .await?;
                let origin = store.data().origin();
                Ok(raw.map(|r| r.map(|p| playlist_page_from_wit(p, &origin))))
            })
        })
        .await
    }

    /// Drive this plugin's `extract-playlist` export through the
    /// host-owned [`PagedPlaylist`] loop, falling back to a single
    /// `extract` call — the [`InfoExtractor::extract_playlist`] trait
    /// default — when the export is absent or the plugin declines the URL.
    ///
    /// Page one is fetched here as a probe: an absent export or
    /// `unsupported-url` means "not a playlist for this plugin". A
    /// successful page one is handed to
    /// [`PagedPlaylist::extract_all_entries_from`] rather than re-fetched,
    /// so a real playlist costs exactly the pages it has, not one extra.
    ///
    /// # Errors
    ///
    /// Any other `extract-playlist` domain error, or an error from
    /// `extract`/the resolution loop.
    pub(crate) async fn extract_playlist_via_plugin(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> RdlpResult<Vec<InfoDict>> {
        let source = PluginPlaylistSource { plugin: self };
        let first = self
            .call_extract_playlist_page(url, source.first_page_index())
            .await
            .map_err(|e| plugin_error_to_rdlp(e, Some(url)))?;
        match first {
            None | Some(Err(WitPlaylistError::UnsupportedUrl(_))) => {
                Ok(vec![self.extract(url, ctx).await?])
            }
            Some(Err(e)) => Err(plugin_error_to_rdlp(
                playlist_error_to_plugin_error(&self.manifest.name, e),
                Some(url),
            )),
            Some(Ok(first_page)) => {
                source
                    .extract_all_entries_from(PlaylistStart { url, first_page }, ctx)
                    .await
            }
        }
    }
}

/// `PagedPlaylist` over one plugin: each page is one `extract-playlist`
/// call in a fresh store under `SEARCH_TIMEOUT` ([`PluginExtractor::call_extract_playlist_page`]);
/// each entry resolves via the plugin's own `extract`
/// (`PluginExtractor::extract`).
pub(crate) struct PluginPlaylistSource<'a> {
    pub plugin: &'a PluginExtractor,
}

impl PagedPlaylist for PluginPlaylistSource<'_> {
    /// `manifest.name` for now; Task 8 adds `Manifest::display_name()` and
    /// switches this to it (refs #768).
    fn name(&self) -> &str {
        &self.plugin.manifest.name
    }

    async fn fetch_playlist_page(
        &self,
        url: &str,
        page: u32,
        _ctx: &ExtractionContext,
    ) -> RdlpResult<PlaylistPage> {
        match self
            .plugin
            .call_extract_playlist_page(url, page)
            .await
            .map_err(|e| plugin_error_to_rdlp(e, Some(url)))?
        {
            None => Err(RdlpError::extraction(
                "plugin has no extract-playlist export",
                url,
            )),
            Some(Ok(page)) => Ok(page),
            Some(Err(e)) => Err(plugin_error_to_rdlp(
                playlist_error_to_plugin_error(&self.plugin.manifest.name, e),
                Some(url),
            )),
        }
    }

    async fn resolve_entry(
        &self,
        entry: &PlaylistEntry,
        ctx: &ExtractionContext,
    ) -> RdlpResult<InfoDict> {
        InfoExtractor::extract(self.plugin, &entry.url, ctx).await
    }
}

#[cfg(test)]
mod tests;
