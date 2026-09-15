//! By-name resolution of the 0.5.2 `extract-with-metadata` export.
//!
//! `wit/COMPATIBILITY.md` §3/§8. Same mechanism as
//! [`crate::playlist_adapter`] — see its module doc for why
//! `WitExtraction`/`WitInfoDictExtra`/`WitMetaValue`/`WitThumbnail` are
//! hand-declared: `bindgen!` only emits types reachable from a bound-world
//! function signature, and adding a types-only `use` to
//! `extractor-plugin-host` (tried first, then reverted) did not change that.
//! Each hand-declared type carries a source-text pin test in `tests.rs`
//! against `wit/types.wit`.
//!
//! `extract-with-metadata`'s error case (`metadata-extract-error` in
//! `wit/extractor.wit`) IS a local alias of `types.extract-error`, the same
//! wire type `extract`'s own error uses — that one *is* bindgen-generated,
//! via `extractor-plugin-host`'s own `use`, so it lifts as the existing
//! `crate::bindings::rdlp::plugin::types::ExtractError`, not a second type.
//! Extras validation (`InfoDictExtra`/`MetaValue` bounds) is a later task;
//! this module only calls the export and hands back the raw lift.

// Why `WitMetaValue` and the `extras` field of `WitInfoDictExtra` still
// carry `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`: validating
// and mapping the open key/value tail is Task 7 of this slice (refs #768).
// This task (6) wires `extract-with-metadata` itself plus every typed
// field, so their own markers are removed here — each `expect` would now
// fail the build as "unfulfilled" on a live item.

use wasmtime::Store;
use wasmtime::component::{ComponentType, Lift};

use crate::PluginError;
use crate::adapter::{ExportCall, call_export_by_name};
use crate::bindings::rdlp::plugin::types::{
    ExtractError as WitExtractError, InfoDict as WitInfoDict,
};
use crate::instance::PluginStoreData;

const EXTRACT_WITH_METADATA_EXPORT: &str = "extract-with-metadata";

/// Hand-lift of `wit/types.wit`'s `thumbnail` record.
#[derive(Debug, Clone, PartialEq, ComponentType, Lift)]
#[component(record)]
pub(crate) struct WitThumbnail {
    /// Thumbnail URL.
    pub url: String,
    /// Plugin-native thumbnail id, if any.
    pub id: Option<String>,
    /// Pixel width, if known.
    pub width: Option<u32>,
    /// Pixel height, if known.
    pub height: Option<u32>,
    /// Selection preference, higher is more preferred.
    pub preference: Option<i32>,
}

/// Hand-lift of `wit/types.wit`'s `meta-value` variant. An OpenTelemetry
/// stable-attribute-subset value: scalar or a homogeneous list, never a
/// nested map.
#[derive(Debug, Clone, PartialEq, ComponentType, Lift)]
#[component(variant)]
pub(crate) enum WitMetaValue {
    /// UTF-8 text.
    #[component(name = "text")]
    Text(String),
    /// Signed 64-bit integer.
    #[component(name = "integer")]
    Integer(i64),
    /// 64-bit float.
    #[component(name = "number")]
    Number(f64),
    /// Boolean.
    #[component(name = "flag")]
    Flag(bool),
    /// A homogeneous list of strings.
    #[component(name = "text-list")]
    TextList(Vec<String>),
}

/// Hand-lift of `wit/types.wit`'s `info-dict-extra` record.
#[derive(Debug, Clone, PartialEq, ComponentType, Lift)]
#[component(record)]
pub(crate) struct WitInfoDictExtra {
    /// Cast/performer names.
    pub actors: Vec<String>,
    /// Channel / uploader-account name, distinct from `uploader`.
    pub channel: Option<String>,
    /// Channel URL.
    #[component(name = "channel-url")]
    pub channel_url: Option<String>,
    /// Age-rating minimum, if the site declares one.
    #[component(name = "age-limit")]
    pub age_limit: Option<u8>,
    /// Additional thumbnails beyond `info-dict`'s single `thumbnail`.
    pub thumbnails: Vec<WitThumbnail>,
    /// Open key/value tail, validated and capped host-side before reaching
    /// `InfoDict::extra`. Not yet read outside this struct's own derives
    /// (`Debug`/`Clone`/`PartialEq`) — `extras_from_wit` reads it in Task 7
    /// of this slice (refs #768) — but those derives already count as a
    /// use, so this carries no `expect(dead_code)`.
    pub extras: Vec<(String, WitMetaValue)>,
}

/// Hand-lift of `wit/types.wit`'s `extraction` record: the frozen
/// `info-dict` core plus the 0.5.2 extra.
///
/// No `PartialEq`: bindgen's `InfoDict` (the `core` field) doesn't derive
/// it, so this can't either.
#[derive(Debug, Clone, ComponentType, Lift)]
#[component(record)]
pub(crate) struct WitExtraction {
    /// The frozen 0.5.0 `info-dict`, bindgen-generated (reachable from
    /// `extract`'s own signature in the bound host world).
    pub core: WitInfoDict,
    /// The 0.5.2 extra.
    pub extra: WitInfoDictExtra,
}

/// `Ok(None)`: the component never declared the export (pre-0.5.2). `Ok(Some(Err(_)))`:
/// the plugin answered with a domain error, mapped by the caller through the
/// same `extract_error_to_plugin_error` `extract` already uses. `Err(Trapped)`:
/// wrong signature, trap, or post-return failure.
pub(crate) async fn call_extract_with_metadata(
    store: &mut Store<PluginStoreData>,
    inst: &wasmtime::component::Instance,
    url: &str,
) -> Result<Option<Result<WitExtraction, WitExtractError>>, PluginError> {
    let out = call_export_by_name::<(String,), (Result<WitExtraction, WitExtractError>,)>(
        store,
        inst,
        ExportCall {
            name: EXTRACT_WITH_METADATA_EXPORT,
            params: (url.to_string(),),
        },
    )
    .await?;
    Ok(out.map(|(r,)| r))
}

/// Bounds on `WitInfoDictExtra::extras` (`Task 7`, refs #768): entry count,
/// per-value size, and total size. Threaded through
/// [`crate::convert::ExtractionSite`] now so `info_dict_from_extraction`'s
/// signature is stable across that task; `Default` (all-zero) is a
/// placeholder — the config-derived values and their named consts land in
/// Task 7, which also writes the `extras_from_wit` that reads these
/// fields for its own bounds checks. No test reads a field back either
/// yet, so — unlike the hand-lifted WIT types above, whose derived
/// `PartialEq` already counts as a read — these carry a real
/// `expect(dead_code)` each.
///
/// Unprefixed field names on purpose: the type name `MetadataCaps` already
/// says every field is a ceiling, so `extras`/`value_bytes`/`total_bytes`
/// read as `caps.extras`, `caps.value_bytes`, `caps.total_bytes` without
/// the repeated `max_` clippy's `struct_field_names` (pedantic) correctly
/// flagged.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MetadataCaps {
    /// Maximum number of `extras` entries kept per extraction.
    #[expect(
        dead_code,
        reason = "read by extras_from_wit, added in Task 7 of this slice (refs #768)"
    )]
    pub extras: usize,
    /// Maximum serialized size, in bytes, of one entry's value.
    #[expect(
        dead_code,
        reason = "read by extras_from_wit, added in Task 7 of this slice (refs #768)"
    )]
    pub value_bytes: usize,
    /// Maximum combined size, in bytes, of all kept entries.
    #[expect(
        dead_code,
        reason = "read by extras_from_wit, added in Task 7 of this slice (refs #768)"
    )]
    pub total_bytes: usize,
}

#[cfg(test)]
mod tests;
