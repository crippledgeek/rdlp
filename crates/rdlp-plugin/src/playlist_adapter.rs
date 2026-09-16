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

use std::sync::atomic::AtomicBool;

use wasmtime::Store;
use wasmtime::component::{ComponentType, Lift};

use crate::PluginError;
use crate::adapter::{
    CallSpec, CommonPluginErr, ExportCall, LISTING_TIMEOUT, PluginExtractor, TimeoutStrikes,
    call_export_by_name, common_plugin_error, plugin_detail, plugin_error_to_rdlp,
};
use crate::convert::{PluginOrigin, cap_plugin_playlist_entries};
use crate::instance::PluginStoreData;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result as RdlpResult};
use rdlp_extractor::base::common::{
    PagedPlaylist, PlaylistEntry, PlaylistPage, PlaylistStart, ResolveRequest,
};
use rdlp_redact::RedactedUrl;
use rdlp_types::InfoDict;

pub(crate) const EXTRACT_PLAYLIST_EXPORT: &str = "extract-playlist";

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
    /// 1-indexed page number, matching `search-page`. The host asked for
    /// a page and keeps its own count; this echo is checked against the
    /// request in [`playlist_page_from_wit`] and a mismatch is logged, not
    /// acted on — a plugin echoing the wrong page is a plugin bug worth
    /// seeing in its log, not a reason to re-number the listing.
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
/// Every plugin-authored `detail` goes through `plugin_detail`, here for
/// the two early-return arms and inside `common_plugin_error` for the rest.
pub(crate) fn playlist_error_to_plugin_error(plugin: &str, e: WitPlaylistError) -> PluginError {
    let common = match e {
        WitPlaylistError::UnsupportedUrl(detail) => {
            return PluginError::UnsupportedUrl {
                plugin: plugin.to_string(),
                detail: plugin_detail(&detail),
            };
        }
        WitPlaylistError::NotFound(detail) => {
            return PluginError::NotFound {
                plugin: plugin.to_string(),
                detail: plugin_detail(&detail),
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
/// rows than it would ever keep. `requested` is the page the host asked
/// for; a plugin echoing a different `page` is logged on its target.
fn playlist_page_from_wit(
    w: WitPlaylistPage,
    requested: u32,
    origin: &PluginOrigin<'_>,
) -> PlaylistPage {
    if w.page != requested {
        log::debug!(
            target: origin.log_target,
            "{EXTRACT_PLAYLIST_EXPORT}: plugin {} answered page {} for a request for page {requested}",
            origin.plugin_name,
            w.page
        );
    }
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
    /// The plugin's domain error (`WitPlaylistError`) is mapped to
    /// [`PluginError`] *inside* the runner's closure — the same place
    /// `call_search`/`call_plugin_extract` map theirs — so
    /// [`counts_as_strike`] sees it and `internal` records a strike here
    /// exactly like every other call kind. Mapping it after
    /// `run_in_fresh_store` returned would hand the runner an `Ok` and its
    /// strike accounting would never see the domain error (#768).
    ///
    /// [`counts_as_strike`]: crate::adapter::counts_as_strike
    ///
    /// # Errors
    ///
    /// The runner's errors ([`PluginError::Disabled`], `Timeout`,
    /// `Trapped`), `Trapped` when the export exists with an unexpected
    /// signature or traps while running, or the plugin's own domain error
    /// mapped via [`playlist_error_to_plugin_error`].
    async fn call_extract_playlist_page(
        &self,
        url: &str,
        page: u32,
    ) -> Result<Option<PlaylistPage>, PluginError> {
        let spec = CallSpec {
            subject_for_errors: url,
            timeout: LISTING_TIMEOUT,
            // Pages are fetched one at a time and rate-limited, so a
            // page timeout is never amplified the way entries under
            // concurrency are: every one counts.
            timeout_strikes: TimeoutStrikes::Always,
        };
        // The URL moves into the future: the runner's closure is
        // higher-ranked over the store borrow, so it cannot return a
        // future that also borrows from this frame.
        let owned = url.to_string();
        self.run_in_fresh_store(spec, move |store, inst| {
            Box::pin(async move {
                // Which page of which playlist each call listed, on the
                // plugin's own target: paired with the runner's per-call
                // budget line it is how an operator sees a listing walk
                // page by page, and how a re-fetched page (the probe's
                // page one listed twice) would show. The URL is in the
                // line because one plugin lists many playlists.
                log::debug!(
                    target: &store.data().log_target,
                    "{EXTRACT_PLAYLIST_EXPORT}: fetching page {page} of {}",
                    RedactedUrl::new(&owned)
                );
                let raw = call_extract_playlist(
                    store,
                    &inst.raw,
                    PlaylistPageRequest { url: &owned, page },
                )
                .await?;
                match raw {
                    None => Ok(None),
                    Some(Ok(p)) => {
                        let origin = store.data().origin();
                        Ok(Some(playlist_page_from_wit(p, page, &origin)))
                    }
                    Some(Err(e)) => {
                        Err(playlist_error_to_plugin_error(&store.data().plugin_name, e))
                    }
                }
            })
        })
        .await
    }

    /// Drive this plugin's `extract-playlist` export through the
    /// host-owned [`PagedPlaylist`] loop, falling back to a single
    /// `extract` call — the [`InfoExtractor::extract_playlist`] trait
    /// default — when the export is absent, `Config::extract_playlist`
    /// is off, or the plugin declines the URL.
    ///
    /// The first two are known before any call: the export's presence is
    /// read off the component type at load (`has_extract_playlist`) and
    /// the config is in hand, so neither costs an instantiation. Only
    /// then is page one fetched as a probe: `unsupported-url` means "not
    /// a playlist for this plugin" and is a per-call cost by nature — the
    /// plugin decides per URL. A successful page one is handed to
    /// [`PagedPlaylist::extract_all_entries_from`] rather than re-fetched,
    /// so a real playlist costs exactly the pages it has, not one extra.
    /// `source` is built before the probe (rather than only once a real
    /// playlist is confirmed) because both `validate_selection` — fail
    /// fast on a malformed range before any network call, same as
    /// `extract_all_entries` — and `first_page_index` are read from it
    /// before that probe runs.
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
        if !self.has_extract_playlist || !ctx.config.extract_playlist {
            return Ok(vec![self.extract(url, ctx).await?]);
        }
        let source = PluginPlaylistSource::new(self);
        source.validate_selection(url, ctx)?;
        let first = self
            .call_extract_playlist_page(url, source.first_page_index())
            .await;
        match first {
            Ok(None) | Err(PluginError::UnsupportedUrl { .. }) => {
                Ok(vec![self.extract(url, ctx).await?])
            }
            Err(e) => Err(plugin_error_to_rdlp(e, Some(url))),
            Ok(Some(first_page)) => {
                source
                    .extract_all_entries_from(PlaylistStart { url, first_page }, ctx)
                    .await
            }
        }
    }
}

/// `PagedPlaylist` over one plugin: each page is one `extract-playlist`
/// call in a fresh store under `LISTING_TIMEOUT` ([`PluginExtractor::call_extract_playlist_page`]);
/// each entry resolves via the plugin's own `extract` under the loop's
/// per-item budget ([`PluginExtractor::extract_within`]) and this batch's
/// one timeout gate.
pub(crate) struct PluginPlaylistSource<'a> {
    plugin: &'a PluginExtractor,
    /// The batch's [`TimeoutStrikes::OncePer`] gate: set by the first
    /// entry whose `extract` times out, so under
    /// `Config::playlist_concurrency` a dead upstream costs the plugin one
    /// strike per batch rather than one per in-flight entry. One source is
    /// built per `extract_playlist` call, so each batch starts unclaimed.
    timeout_struck: AtomicBool,
}

impl<'a> PluginPlaylistSource<'a> {
    /// A source for one batch over `plugin`, its timeout gate unclaimed.
    pub(crate) const fn new(plugin: &'a PluginExtractor) -> Self {
        Self {
            plugin,
            timeout_struck: AtomicBool::new(false),
        }
    }

    /// The spec one entry's `extract` runs under: the loop's per-item
    /// budget, the entry URL as the subject, and this batch's timeout gate.
    pub(crate) const fn entry_spec<'r>(&'r self, request: &'r ResolveRequest<'r>) -> CallSpec<'r> {
        CallSpec {
            subject_for_errors: request.entry.url.as_str(),
            timeout: request.budget,
            timeout_strikes: TimeoutStrikes::OncePer(&self.timeout_struck),
        }
    }
}

impl PagedPlaylist for PluginPlaylistSource<'_> {
    /// Display-only — mirrors [`InfoExtractor::name`]'s use of
    /// `Manifest::display_name()`. Identity/routing stay on `manifest.name`.
    fn name(&self) -> &str {
        self.plugin.manifest.display_name()
    }

    async fn fetch_playlist_page(
        &self,
        url: &str,
        page: u32,
        _ctx: &ExtractionContext,
    ) -> RdlpResult<PlaylistPage> {
        match self.plugin.call_extract_playlist_page(url, page).await {
            // Unreachable in practice: `has_extract_playlist` (read off
            // the component type at load) and the probe both proved this
            // component exports `extract-playlist` before this method is
            // ever called. Kept for exhaustiveness and because
            // `call_extract_playlist_page` has no other caller to prove
            // that of.
            Ok(None) => Err(RdlpError::extraction(
                "plugin has no extract-playlist export",
                url,
            )),
            Ok(Some(page)) => Ok(page),
            Err(e) => Err(plugin_error_to_rdlp(e, Some(url))),
        }
    }

    /// The budget is the plugin call's own tokio timeout and epoch
    /// deadline, so a slow entry is a `PluginError::Timeout` and the
    /// loop's guard never competes with it; the timeout is a strike at
    /// most once per batch ([`PluginPlaylistSource::entry_spec`]).
    async fn resolve_entry(
        &self,
        request: ResolveRequest<'_>,
        ctx: &ExtractionContext,
    ) -> RdlpResult<InfoDict> {
        self.plugin
            .extract_within(ctx, self.entry_spec(&request))
            .await
    }
}

#[cfg(test)]
mod tests;
