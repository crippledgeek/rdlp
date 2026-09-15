//! By-name resolution of the 0.5.2 `extract-playlist` export.
//!
//! `wit/COMPATIBILITY.md` §3/§8. The host binds the frozen
//! `extractor-plugin-host` bindings, which omit every post-0.5.0 export, so
//! this export is looked up on the live instance through
//! [`crate::adapter::call_export_by_name`] — the same mechanism
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
use crate::adapter::{CommonPluginErr, ExportCall, call_export_by_name, common_plugin_error};
use crate::instance::PluginStoreData;

// Why every hand-declared item below carries a per-item
// `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`: the caller —
// `PluginExtractor` — lands in a later task of this slice (refs #768), so
// each is unreachable from production code today. Scoped per item, not at
// module level, so each `expect` unfulfills (and fails the build) the
// moment that specific item is wired in, instead of one blanket
// suppression silently covering whatever is still unused.

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PluginExtractor by a later task in this slice (refs #768)"
    )
)]
const EXTRACT_PLAYLIST_EXPORT: &str = "extract-playlist";

/// Hand-lift of `wit/types.wit`'s `playlist-entry` record. See the module
/// doc for why this is hand-declared rather than bindgen-generated.
#[derive(Debug, Clone, PartialEq, Eq, ComponentType, Lift)]
#[component(record)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PluginExtractor by a later task in this slice (refs #768)"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PluginExtractor by a later task in this slice (refs #768)"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PluginExtractor by a later task in this slice (refs #768)"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PluginExtractor by a later task in this slice (refs #768)"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into PluginExtractor by a later task in this slice (refs #768)"
    )
)]
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

#[cfg(test)]
mod tests;
