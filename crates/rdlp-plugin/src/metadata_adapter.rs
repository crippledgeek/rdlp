//! By-name resolution of the 0.5.2 `extract-with-metadata` export.
//!
//! `wit/COMPATIBILITY.md` §3/§9. Same mechanism as
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
//!
//! The open `extras` tail of `info-dict-extra` is validated, capped and
//! mapped to JSON in `extras` (`extras::extras_from_wit`), with the
//! bounds (`MetadataCaps`) coming from `Config` per call.

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
    /// Open key/value tail, validated and capped host-side by
    /// [`extras::extras_from_wit`] before reaching `InfoDict::extra`.
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

/// Default ceiling on `extras` entries kept per extraction, when
/// `Config::max_metadata_extras` is unset. 64 keys is headroom over the
/// richest single video page among the 15 in-tree site extractors (the
/// fullest populate on the order of twenty typed `InfoDict` fields plus a
/// handful of site-specific ones), so a legitimate plugin never meets it
/// while a runaway one is bounded by a constant, not by its own output.
pub(crate) const DEFAULT_MAX_METADATA_EXTRAS: usize = 64;

/// Default ceiling on one `extras` value's byte length, when
/// `Config::max_metadata_value_bytes` is unset. 4 KiB holds any label,
/// name, or short description a site exposes as metadata; a value beyond
/// it is page content masquerading as a key, not a fact about the video.
pub(crate) const DEFAULT_MAX_METADATA_VALUE_BYTES: usize = 4096;

/// Default ceiling on the aggregate byte length of every kept `extras`
/// entry (key plus value), when `Config::max_metadata_extras_bytes` is
/// unset. 64 KiB is Kubernetes' 256 KiB per-object annotation ceiling
/// scaled down to what one video's metadata can reasonably carry — the
/// map is serialised into every `--dump-json` line and archive record, so
/// its size is paid per download, not once.
pub(crate) const DEFAULT_MAX_METADATA_EXTRAS_BYTES: usize = 65_536;

/// Bounds on `WitInfoDictExtra::extras`: entry count, per-value size, and
/// aggregate size. Built from `Config` per call ([`From<&Config>`]) and
/// carried through [`crate::convert::ExtractionSite`] to
/// [`extras::extras_from_wit`]; `Default` is the three `DEFAULT_MAX_METADATA_*`
/// consts, which is also what `From` falls back to per unset field.
///
/// Unprefixed field names on purpose: the type name `MetadataCaps` already
/// says every field is a ceiling, so `extras`/`value_bytes`/`total_bytes`
/// read as `caps.extras`, `caps.value_bytes`, `caps.total_bytes` without
/// the repeated `max_` clippy's `struct_field_names` (pedantic) correctly
/// flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MetadataCaps {
    /// Maximum number of `extras` entries kept per extraction.
    pub extras: usize,
    /// Maximum size, in bytes, of one text-carrying entry's value (`text`
    /// length; for a `text-list` the summed lengths plus
    /// [`extras::METADATA_LIST_ITEM_BYTES`] per element).
    pub value_bytes: usize,
    /// Maximum combined size, in bytes, of all kept entries' keys and
    /// values.
    pub total_bytes: usize,
}

impl Default for MetadataCaps {
    fn default() -> Self {
        Self {
            extras: DEFAULT_MAX_METADATA_EXTRAS,
            value_bytes: DEFAULT_MAX_METADATA_VALUE_BYTES,
            total_bytes: DEFAULT_MAX_METADATA_EXTRAS_BYTES,
        }
    }
}

impl From<&rdlp_types::Config> for MetadataCaps {
    /// Each field independently: a set `Config` value overrides its
    /// default; an unset one keeps it (`Config::validate` has already
    /// bounded any set value).
    fn from(c: &rdlp_types::Config) -> Self {
        let d = Self::default();
        Self {
            extras: c.max_metadata_extras.unwrap_or(d.extras),
            value_bytes: c.max_metadata_value_bytes.unwrap_or(d.value_bytes),
            total_bytes: c.max_metadata_extras_bytes.unwrap_or(d.total_bytes),
        }
    }
}

pub(crate) mod extras;

#[cfg(test)]
mod tests;
