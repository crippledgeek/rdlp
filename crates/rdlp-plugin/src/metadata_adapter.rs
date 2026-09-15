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
//!
//! The open `extras` tail of `info-dict-extra` is validated, capped and
//! mapped to JSON here too (`extras_from_wit`), with the bounds
//! (`MetadataCaps`) coming from `Config` per call.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde_json::Value;
use wasmtime::Store;
use wasmtime::component::{ComponentType, Lift};

use crate::PluginError;
use crate::adapter::{ExportCall, call_export_by_name};
use crate::bindings::rdlp::plugin::types::{
    ExtractError as WitExtractError, InfoDict as WitInfoDict,
};
use crate::convert::PluginOrigin;
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
    /// [`extras_from_wit`] before reaching `InfoDict::extra`.
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

/// Longest admissible `extras` key, in bytes. 63 is the DNS label limit
/// (RFC 1035 §2.3.4) that Kubernetes label names and OpenTelemetry
/// attribute-key conventions both inherit, and the keys are the same
/// `[a-z][a-z0-9-]*` shape — a plugin author copying a key from either
/// ecosystem lands inside the bound.
pub(crate) const MAX_METADATA_KEY_BYTES: usize = 63;

/// What an `integer`, `number`, or `flag` value counts toward the aggregate
/// byte bound: the 8 bytes an `i64`/`f64` occupies. A `bool` is charged the
/// same rather than 1 so the accounting has one scalar size, not three;
/// the per-value bound does not apply to scalars at all (their size is
/// fixed by the type, not chosen by the plugin).
pub(crate) const SCALAR_VALUE_BYTES: usize = 8;

/// Bounds on `WitInfoDictExtra::extras`: entry count, per-value size, and
/// aggregate size. Built from `Config` per call ([`From<&Config>`]) and
/// carried through [`crate::convert::ExtractionSite`] to
/// [`extras_from_wit`]; `Default` is the three `DEFAULT_MAX_METADATA_*`
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
    /// length, or the summed lengths of a `text-list`).
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

/// Whether `key` matches `^[a-z][a-z0-9-]{0,62}$` — the OpenTelemetry /
/// Kubernetes-label key shape `wit/types.wit` documents for `extras` — as
/// a byte loop, since every admissible byte is ASCII and a multi-byte
/// char can only ever fail.
fn key_is_well_formed(key: &str) -> bool {
    let bytes = key.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= MAX_METADATA_KEY_BYTES
        && first.is_ascii_lowercase()
        && rest
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// The JSON object `InfoDict::new` serialises to: exactly the fields that
/// are neither `None` nor empty at construction, i.e. the ones a
/// deserialiser must be handed. The probe in [`key_is_reserved`] starts
/// from this so it never has to name a required field itself.
static REQUIRED_INFO_DICT_JSON: LazyLock<serde_json::Map<String, Value>> =
    LazyLock::new(
        || match serde_json::to_value(rdlp_types::InfoDict::new("", "", "", "")) {
            Ok(Value::Object(map)) => map,
            other => unreachable!("InfoDict serialises to a JSON object, got {other:?}"),
        },
    );

/// Whether `snake` (an `extras` key with `-` already folded to `_`) names
/// a typed `InfoDict` field, so that admitting it would let the
/// `#[serde(flatten)]`ed `extra` map overwrite that field in every JSON
/// rendering of the dict.
///
/// Asks serde rather than a list: a serialised `InfoDict` hides every
/// field that is `None`/empty (`skip_serializing_if`), so the key set of
/// one instance is not the field set, and a hand-kept list of ~40 names
/// drifts. Deserialising `{required fields…, snake: null}` instead routes
/// the key exactly as the flatten does — to a typed field (which either
/// accepts `null` or rejects it: reserved either way) or into `extra`
/// (free). The cost is one small deserialisation per candidate key,
/// bounded by `MetadataCaps::extras` per extraction.
fn key_is_reserved(snake: &str) -> bool {
    let mut probe = REQUIRED_INFO_DICT_JSON.clone();
    probe.insert(snake.to_string(), Value::Null);
    let lands_in_extra = serde_json::from_value::<rdlp_types::InfoDict>(Value::Object(probe))
        .is_ok_and(|dict| dict.extra.contains_key(snake));
    !lands_in_extra
}

/// Why one `extras` entry was refused. One counter per class in
/// [`ExtrasTally`], one warning per non-zero class at the end, so a
/// plugin returning a hundred bad keys costs one log line, not a hundred,
/// and the line names the bound — never the key or value, which may be
/// the very thing being refused (an escape sequence, a megabyte of text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// Key outside `^[a-z][a-z0-9-]{0,62}$`.
    MalformedKey,
    /// Key names a typed `InfoDict` field (after `-` → `_`).
    ReservedKey,
    /// Key already admitted earlier in the same list.
    DuplicateKey,
    /// `number` that is NaN or ±infinity — JSON has no representation.
    NonFiniteNumber,
    /// Text-carrying value over `MetadataCaps::value_bytes`.
    ValueTooLarge,
    /// Entry past `MetadataCaps::extras` admitted entries.
    OverCount,
    /// Entry that would take the aggregate past `MetadataCaps::total_bytes`
    /// — or any entry after one that did.
    OverTotal,
}

/// Per-extraction admission accounting for `extras`: how many entries
/// were kept, how many bytes they add up to, whether the aggregate bound
/// has already been crossed (after which nothing is admitted), and one
/// refusal count per [`Refusal`] class.
#[derive(Debug, Default)]
struct ExtrasTally {
    kept: usize,
    kept_bytes: usize,
    total_crossed: bool,
    malformed_key: usize,
    reserved_key: usize,
    duplicate_key: usize,
    non_finite_number: usize,
    value_too_large: usize,
    over_count: usize,
    over_total: usize,
}

impl ExtrasTally {
    const fn refuse(&mut self, why: Refusal) {
        let counter = match why {
            Refusal::MalformedKey => &mut self.malformed_key,
            Refusal::ReservedKey => &mut self.reserved_key,
            Refusal::DuplicateKey => &mut self.duplicate_key,
            Refusal::NonFiniteNumber => &mut self.non_finite_number,
            Refusal::ValueTooLarge => &mut self.value_too_large,
            Refusal::OverCount => &mut self.over_count,
            Refusal::OverTotal => &mut self.over_total,
        };
        *counter += 1;
    }

    /// Admit an entry of `bytes` (key plus value) under the count and
    /// aggregate bounds, recording the refusal otherwise.
    const fn admit(&mut self, bytes: usize, caps: &MetadataCaps) -> bool {
        if self.total_crossed {
            self.refuse(Refusal::OverTotal);
            return false;
        }
        if self.kept >= caps.extras {
            self.refuse(Refusal::OverCount);
            return false;
        }
        let next = self.kept_bytes.saturating_add(bytes);
        if next > caps.total_bytes {
            self.total_crossed = true;
            self.refuse(Refusal::OverTotal);
            return false;
        }
        self.kept += 1;
        self.kept_bytes = next;
        true
    }

    /// One warning per non-zero refusal class on the plugin's log target,
    /// naming the bound the class broke.
    fn report(&self, caps: &MetadataCaps, origin: &PluginOrigin<'_>) {
        let classes = [
            (
                self.malformed_key,
                format!(
                    "whose key is not `^[a-z][a-z0-9-]{{0,62}}$` (1..={MAX_METADATA_KEY_BYTES} bytes); dropping them"
                ),
            ),
            (
                self.reserved_key,
                "whose key shadows an info-dict field; dropping them".to_string(),
            ),
            (
                self.duplicate_key,
                "repeating an earlier key; keeping the first of each".to_string(),
            ),
            (
                self.non_finite_number,
                "whose number is NaN or infinite, which JSON cannot represent; dropping them"
                    .to_string(),
            ),
            (
                self.value_too_large,
                format!(
                    "whose value exceeds {} bytes; dropping them",
                    caps.value_bytes
                ),
            ),
            (
                self.over_count,
                format!("past the {}-entry bound; dropping them", caps.extras),
            ),
            (
                self.over_total,
                format!(
                    "past the {}-byte aggregate bound; dropping them and every later entry",
                    caps.total_bytes
                ),
            ),
        ];
        for (n, what) in classes {
            if n == 0 {
                continue;
            }
            log::warn!(
                target: origin.log_target,
                "{EXTRACT_WITH_METADATA_EXPORT}: plugin {} supplied {n} extras {what}",
                origin.plugin_name
            );
        }
    }
}

/// A value's byte size for the bounds, and its JSON form. `Err` when the
/// value itself is inadmissible regardless of any cap.
fn value_to_json(v: WitMetaValue, caps: &MetadataCaps) -> Result<(usize, Value), Refusal> {
    let text_bytes = |len: usize| {
        if len > caps.value_bytes {
            Err(Refusal::ValueTooLarge)
        } else {
            Ok(len)
        }
    };
    match v {
        WitMetaValue::Text(s) => {
            let len = text_bytes(s.len())?;
            Ok((len, Value::String(s)))
        }
        WitMetaValue::TextList(items) => {
            let len = text_bytes(items.iter().map(String::len).sum())?;
            Ok((
                len,
                Value::Array(items.into_iter().map(Value::String).collect()),
            ))
        }
        WitMetaValue::Integer(i) => Ok((SCALAR_VALUE_BYTES, Value::from(i))),
        WitMetaValue::Number(n) if n.is_finite() => Ok((SCALAR_VALUE_BYTES, Value::from(n))),
        WitMetaValue::Number(_) => Err(Refusal::NonFiniteNumber),
        WitMetaValue::Flag(b) => Ok((SCALAR_VALUE_BYTES, Value::Bool(b))),
    }
}

/// Validate, bound, and map a plugin's `extras` tail into the JSON map
/// `InfoDict::extra` holds. Entries are visited in the plugin's order and
/// the first `caps.extras` admissible ones are kept; each refusal class
/// warns once on `origin`'s log target (see [`Refusal`]).
///
/// Per entry, in order: the key must be well-formed
/// ([`key_is_well_formed`]), must not shadow a typed `InfoDict` field
/// after `-` → `_` ([`key_is_reserved`] — `extra` is flattened into the
/// top level of every JSON rendering, so a shadowing key would overwrite
/// the field), and must not repeat an admitted key; the value must be
/// finite and, when text-carrying, within `caps.value_bytes`; then the
/// count and aggregate bounds apply, the aggregate charging key bytes plus
/// value bytes with scalars at [`SCALAR_VALUE_BYTES`]. Once the aggregate
/// bound is crossed no later entry is admitted, so a plugin cannot fill
/// the remaining budget with whatever happens to fit after a large entry.
pub(crate) fn extras_from_wit(
    extras: Vec<(String, WitMetaValue)>,
    caps: &MetadataCaps,
    origin: &PluginOrigin<'_>,
) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let mut tally = ExtrasTally::default();
    for (key, value) in extras {
        if !key_is_well_formed(&key) {
            tally.refuse(Refusal::MalformedKey);
            continue;
        }
        if key_is_reserved(&key.replace('-', "_")) {
            tally.refuse(Refusal::ReservedKey);
            continue;
        }
        if out.contains_key(&key) {
            tally.refuse(Refusal::DuplicateKey);
            continue;
        }
        let (value_bytes, json) = match value_to_json(value, caps) {
            Ok(v) => v,
            Err(why) => {
                tally.refuse(why);
                continue;
            }
        };
        if tally.admit(key.len().saturating_add(value_bytes), caps) {
            out.insert(key, json);
        }
    }
    tally.report(caps, origin);
    out
}

#[cfg(test)]
mod tests;
