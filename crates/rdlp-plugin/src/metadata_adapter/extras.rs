//! Validation, bounds, and JSON mapping of the open `extras` tail of a
//! 0.5.2 `info-dict-extra` ([`extras_from_wit`]).
//!
//! Split out of `metadata_adapter.rs` so that file keeps the hand-lifted
//! WIT types, the export call, and `MetadataCaps`, while everything that
//! decides which entries survive the boundary lives here.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde_json::Value;

use super::{MetadataCaps, WitMetaValue};
use crate::convert::PluginOrigin;

/// Longest admissible `extras` key, in bytes. 63 is the DNS label limit
/// (RFC 1035 §2.3.4) that Kubernetes label names inherit, and the keys
/// are the same `[a-z][a-z0-9-]*` shape — a plugin author copying a key
/// from a label lands inside the bound.
pub const MAX_METADATA_KEY_BYTES: usize = 63;

/// The bytes an `extras` key may carry after its first: the `{0,N}` of
/// the key shape's tail, derived so the regex quoted in diagnostics and
/// the byte bound above cannot drift apart.
const KEY_TAIL_MAX: usize = MAX_METADATA_KEY_BYTES - 1;

/// What an `integer`, `number`, or `flag` value counts toward the aggregate
/// byte bound: the 8 bytes an `i64`/`f64` occupies. A `bool` is charged the
/// same rather than 1 so the accounting has one scalar size, not three;
/// the per-value bound does not apply to scalars at all (their size is
/// fixed by the type, not chosen by the plugin).
pub const SCALAR_VALUE_BYTES: usize = 8;

/// What each `text-list` element costs on top of its string bytes, for
/// both the per-value and the aggregate bound. An array element costs the
/// host a JSON node in every `--dump-json` line and archive record
/// regardless of its string length — a list of a hundred thousand empty
/// strings is not free — so it is charged the same figure scalars are.
pub const METADATA_LIST_ITEM_BYTES: usize = SCALAR_VALUE_BYTES;

/// Whether `key` matches `^[a-z][a-z0-9-]{0,KEY_TAIL_MAX}$` (a lowercase
/// letter, then up to [`KEY_TAIL_MAX`] of `[a-z0-9-]`; [`MAX_METADATA_KEY_BYTES`]
/// in all) — the Kubernetes-label key shape `wit/COMPATIBILITY.md` §9 fixes
/// for `extras` — as a byte loop, since every admissible byte is ASCII and
/// a multi-byte char can only ever fail.
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
///
/// The `unreachable!` arm cannot fire: `serde_json::to_value` fails only
/// on a map with non-string keys or a non-finite float, and
/// `InfoDict::new` builds four `String`s, an empty `formats` `Vec`, an
/// empty `extra` `HashMap<String, _>`, and `None` everywhere else — no
/// float is set, and every map key is a `String`. A struct always
/// serialises as a JSON object, so the `Ok` arm is the only one reachable.
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
/// (free). The cost is one small deserialisation per candidate key —
/// bounded per extraction by [`admit_extras`], not by the plugin: see
/// [`extras_from_wit`] for the `2 × caps.extras + 1` derivation.
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
    /// Key outside `^[a-z][a-z0-9-]{0,KEY_TAIL_MAX}$` ([`key_is_well_formed`]).
    MalformedKey,
    /// Key names a typed `InfoDict` field (after `-` → `_`).
    ReservedKey,
    /// Key already admitted earlier in the same list.
    DuplicateKey,
    /// `number` that is NaN or ±infinity — JSON has no representation.
    NonFiniteNumber,
    /// Text-carrying value over `MetadataCaps::value_bytes`.
    ValueTooLarge,
    /// Entry past `MetadataCaps::extras` admitted entries — or past
    /// `MetadataCaps::extras` refusals that each cost per-entry work (the
    /// refusal budget, see [`ExtrasTally::saturated_by`]).
    OverCount,
    /// Entry that would take the aggregate past `MetadataCaps::total_bytes`
    /// — or any entry after one that did.
    OverTotal,
}

impl Refusal {
    /// The warning's description of this class, naming the bound it broke.
    /// Built only for a class with a non-zero count.
    fn describe(self, caps: &MetadataCaps) -> String {
        match self {
            Self::MalformedKey => format!(
                "whose key is not `^[a-z][a-z0-9-]{{0,{KEY_TAIL_MAX}}}$` \
                 (1..={MAX_METADATA_KEY_BYTES} bytes); dropping them"
            ),
            Self::ReservedKey => "whose key shadows an info-dict field; dropping them".to_string(),
            Self::DuplicateKey => "repeating an earlier key; keeping the first of each".to_string(),
            Self::NonFiniteNumber => {
                "whose number is NaN or infinite, which JSON cannot represent; dropping them"
                    .to_string()
            }
            Self::ValueTooLarge => format!(
                "whose value exceeds {} bytes (text bytes, plus {METADATA_LIST_ITEM_BYTES} \
                 per text-list item); dropping them",
                caps.value_bytes
            ),
            Self::OverCount => format!("past the {}-entry bound; dropping them", caps.extras),
            Self::OverTotal => format!(
                "past the {}-byte aggregate bound; dropping them and every later entry",
                caps.total_bytes
            ),
        }
    }
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
    /// Refusals that were reached only after per-entry work (a key scan,
    /// a value conversion, a probe): every class except the two
    /// saturation classes, plus the one entry that crosses the aggregate
    /// bound (probed, then refused in [`admit`](Self::admit)). Spends
    /// the refusal budget.
    worked_refusals: usize,
    /// Snake-cased keys the probe has already found reserved, so a key
    /// repeated a million times is probed once. Bounded by the refusal
    /// budget: at most `caps.extras + 1` distinct keys ever land here.
    reserved_seen: HashSet<String>,
    /// How many times [`key_is_reserved`] ran — the cost the tests bound.
    #[cfg(test)]
    probes: usize,
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
        if !matches!(why, Refusal::OverCount | Refusal::OverTotal) {
            self.worked_refusals += 1;
        }
    }

    /// The bound that already refuses every further entry, if any: the
    /// aggregate once crossed, else the count once reached by admissions
    /// — or exceeded by refusals that each cost per-entry work. Checked
    /// before any per-entry work, so a plugin supplying a million entries
    /// costs the host a million counter increments, not a million probes,
    /// whether those entries are admissible, malformed, reserved,
    /// duplicated, or carry a refused value: the per-entry work a plugin
    /// can buy is `caps.extras` admissions plus `caps.extras + 1`
    /// refusals, however long its list.
    const fn saturated_by(&self, caps: &MetadataCaps) -> Option<Refusal> {
        if self.total_crossed {
            Some(Refusal::OverTotal)
        } else if self.kept >= caps.extras || self.worked_refusals > caps.extras {
            Some(Refusal::OverCount)
        } else {
            None
        }
    }

    /// Whether `snake` names a typed `InfoDict` field, asking
    /// [`key_is_reserved`] once per distinct key and answering repeats
    /// from `reserved_seen`. A key found NOT reserved is admitted by the
    /// caller, so its repeats are caught as duplicates before ever
    /// reaching here — the memo only needs the refused side.
    fn reserved(&mut self, snake: &str) -> bool {
        if self.reserved_seen.contains(snake) {
            return true;
        }
        #[cfg(test)]
        {
            self.probes += 1;
        }
        if key_is_reserved(snake) {
            self.reserved_seen.insert(snake.to_string());
            return true;
        }
        false
    }

    /// Admit an entry of `bytes` (key plus value) under the aggregate
    /// bound, recording the refusal otherwise. The count bound and an
    /// already-crossed aggregate are [`saturated_by`](Self::saturated_by)'s,
    /// checked by the caller before any per-entry work — so this is the
    /// one site the aggregate is crossed, and the one that records it.
    const fn admit(&mut self, bytes: usize, caps: &MetadataCaps) -> bool {
        let next = self.kept_bytes.saturating_add(bytes);
        if next > caps.total_bytes {
            self.total_crossed = true;
            self.refuse(Refusal::OverTotal);
            // The crossing entry paid for its checks and its probe; the
            // entries after it are refused by `saturated_by` before any.
            self.worked_refusals += 1;
            return false;
        }
        self.kept += 1;
        self.kept_bytes = next;
        true
    }

    /// One warning per non-zero refusal class on the plugin's log target,
    /// naming the bound the class broke. The description is built only
    /// for the classes that fire, so the all-admitted path formats nothing.
    fn report(&self, caps: &MetadataCaps, origin: &PluginOrigin<'_>) {
        let classes = [
            (self.malformed_key, Refusal::MalformedKey),
            (self.reserved_key, Refusal::ReservedKey),
            (self.duplicate_key, Refusal::DuplicateKey),
            (self.non_finite_number, Refusal::NonFiniteNumber),
            (self.value_too_large, Refusal::ValueTooLarge),
            (self.over_count, Refusal::OverCount),
            (self.over_total, Refusal::OverTotal),
        ];
        for (n, why) in classes {
            if n == 0 {
                continue;
            }
            log::warn!(
                target: origin.log_target,
                "{}: plugin {} supplied {n} extras {}",
                super::EXTRACT_WITH_METADATA_EXPORT,
                origin.plugin_name,
                why.describe(caps)
            );
        }
    }
}

/// A value's byte size for the bounds, and its JSON form. `Err` when the
/// value itself is inadmissible regardless of any cap.
///
/// A `text-list` is charged its summed string bytes plus
/// [`METADATA_LIST_ITEM_BYTES`] per element, so its element count is
/// bounded by `caps.value_bytes` too — summed bytes alone would let a
/// list of empty strings grow without limit.
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
            let strings: usize = items.iter().map(String::len).sum();
            let nodes = items.len().saturating_mul(METADATA_LIST_ITEM_BYTES);
            let len = text_bytes(strings.saturating_add(nodes))?;
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
/// Per entry, in order: once the count bound is reached, the aggregate
/// bound has been crossed, or more than `caps.extras` refusals have each
/// cost per-entry work, the entry is refused outright — before any key or
/// value work, so the per-entry cost past the bounds is a counter
/// increment. Otherwise the key must be well-formed
/// ([`key_is_well_formed`]) and must not repeat an admitted key; the value
/// must be finite and, when text-carrying, within `caps.value_bytes` (a
/// `text-list` charged [`METADATA_LIST_ITEM_BYTES`] per element on top of
/// its bytes); only then is the key probed for shadowing a typed
/// `InfoDict` field after `-` → `_` ([`key_is_reserved`] — `extra` is
/// flattened into the top level of every JSON rendering, so a shadowing
/// key would overwrite the field), with the verdict memoised per distinct
/// key; then the aggregate bound applies, charging key bytes plus value
/// bytes with scalars at [`SCALAR_VALUE_BYTES`]. Once the aggregate bound
/// is crossed no later entry is admitted, so a plugin cannot fill the
/// remaining budget with whatever happens to fit after a large entry.
///
/// The serde probe therefore runs at most `2 × caps.extras + 1` times per
/// extraction however many entries a plugin supplies: once per admitted
/// key (at most `caps.extras`) plus once per probed-then-refused entry — a
/// distinct reserved key, or the one entry that crosses the aggregate
/// bound — each a worked refusal, of which at most `caps.extras + 1`
/// happen before the budget saturates; every cheaper refusal is decided
/// before the probe and a repeated reserved key is answered from the
/// memo.
pub fn extras_from_wit(
    extras: Vec<(String, WitMetaValue)>,
    caps: &MetadataCaps,
    origin: &PluginOrigin<'_>,
) -> HashMap<String, Value> {
    let (out, tally) = admit_extras(extras, caps);
    tally.report(caps, origin);
    out
}

/// The admission loop behind [`extras_from_wit`], returning the tally so a
/// test can read what the loop cost (`probes`) rather than only what it
/// kept.
fn admit_extras(
    extras: Vec<(String, WitMetaValue)>,
    caps: &MetadataCaps,
) -> (HashMap<String, Value>, ExtrasTally) {
    let mut out = HashMap::new();
    let mut tally = ExtrasTally::default();
    for (key, value) in extras {
        if let Some(why) = tally.saturated_by(caps) {
            tally.refuse(why);
            continue;
        }
        if !key_is_well_formed(&key) {
            tally.refuse(Refusal::MalformedKey);
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
        if tally.reserved(&key.replace('-', "_")) {
            tally.refuse(Refusal::ReservedKey);
            continue;
        }
        if tally.admit(key.len().saturating_add(value_bytes), caps) {
            out.insert(key, json);
        }
    }
    (out, tally)
}

#[cfg(test)]
mod tests;
