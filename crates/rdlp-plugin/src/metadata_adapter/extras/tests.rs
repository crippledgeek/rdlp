use super::super::{
    DEFAULT_MAX_METADATA_EXTRAS, DEFAULT_MAX_METADATA_EXTRAS_BYTES,
    DEFAULT_MAX_METADATA_VALUE_BYTES,
};
use super::*;
use crate::test_support::unit::{
    TEST_LOG_TARGET, captured_count_containing, captured_entry_containing, captured_logs,
    test_origin,
};
use serde_json::json;

/// Caps loose enough that only the check under test can refuse.
const LOOSE: MetadataCaps = MetadataCaps {
    extras: 1000,
    value_bytes: 1000,
    total_bytes: 100_000,
};

fn text(s: &str) -> WitMetaValue {
    WitMetaValue::Text(s.to_string())
}

fn run(extras: Vec<(&str, WitMetaValue)>, caps: &MetadataCaps) -> HashMap<String, Value> {
    extras_from_wit(
        extras
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        caps,
        &test_origin(),
    )
}

#[test]
fn valid_keys_and_all_value_kinds_map_to_json() {
    let out = run(
        vec![
            ("studio", text("acme")),
            ("views-week", WitMetaValue::Integer(12)),
            ("score", WitMetaValue::Number(4.5)),
        ],
        &MetadataCaps::default(),
    );
    assert_eq!(out.get("studio"), Some(&json!("acme")));
    assert_eq!(out.get("views-week"), Some(&json!(12)));
    assert_eq!(out.get("score"), Some(&json!(4.5)));
    assert_eq!(out.len(), 3);
}

#[test]
fn flag_and_text_list_map_to_bool_and_array() {
    let out = run(
        vec![
            ("hd", WitMetaValue::Flag(true)),
            (
                "aliases",
                WitMetaValue::TextList(vec!["a".into(), "b".into()]),
            ),
        ],
        &MetadataCaps::default(),
    );
    assert_eq!(out.get("hd"), Some(&json!(true)));
    assert_eq!(out.get("aliases"), Some(&json!(["a", "b"])));
}

#[test]
fn negative_integer_keeps_its_sign() {
    let out = run(vec![("delta", WitMetaValue::Integer(-7))], &LOOSE);
    assert_eq!(out.get("delta"), Some(&json!(-7)));
}

#[test]
fn key_charset_rejected() {
    let logs = captured_logs();
    let long = "a".repeat(64);
    for k in [
        "Studio",
        "1st",
        "a_b",
        "-x",
        "",
        "a b",
        "ä",
        "a.b",
        long.as_str(),
    ] {
        let out = run(vec![(k, WitMetaValue::Flag(true))], &LOOSE);
        assert!(!out.contains_key(k), "{k:?} must be refused");
        assert!(out.is_empty(), "{k:?}: nothing else may appear");
    }
    let (_, msg) = captured_entry_containing(&logs, "whose key is not");
    assert!(msg.contains(&MAX_METADATA_KEY_BYTES.to_string()), "{msg}");
}

#[test]
fn key_of_63_chars_accepted() {
    let k = format!("a{}", "b".repeat(62));
    assert_eq!(k.len(), MAX_METADATA_KEY_BYTES);
    let out = run(vec![(k.as_str(), WitMetaValue::Flag(true))], &LOOSE);
    assert!(out.contains_key(&k));
}

#[test]
fn digits_and_hyphens_after_the_first_letter_are_accepted() {
    let out = run(vec![("a1-2b", WitMetaValue::Flag(true))], &LOOSE);
    assert!(out.contains_key("a1-2b"));
}

/// `InfoDict::extra` is `#[serde(flatten)]`, so an extra named after a
/// core field would overwrite that field's JSON — refused at the
/// boundary. The reserved set is compared after `-` → `_`, so the
/// kebab spelling of a snake-case field is refused too; and it must
/// cover fields `InfoDict::new` leaves `None` (which never serialize),
/// hence `view-count`/`age-limit`/`playlist-index`.
#[test]
fn shadowing_a_core_field_is_dropped_with_a_warning() {
    let logs = captured_logs();
    for k in [
        "title",
        "id",
        "extractor",
        "formats",
        "view-count",
        "age-limit",
        "playlist-index",
        "webpage-url",
        "actors",
        "extractor-key",
    ] {
        let out = run(vec![(k, WitMetaValue::Flag(true))], &LOOSE);
        assert!(!out.contains_key(k), "{k:?} shadows a core field");
    }
    let (target, msg) = captured_entry_containing(&logs, "shadows");
    assert_eq!(target, TEST_LOG_TARGET);
    assert!(
        !msg.contains("playlist-index"),
        "must not echo the key: {msg}"
    );
}

/// `extra` is the Rust field holding the map, not a JSON key (it is
/// flattened away), so it is not reserved.
#[test]
fn the_flattened_extra_field_name_itself_is_not_reserved() {
    let out = run(vec![("extra", WitMetaValue::Flag(true))], &LOOSE);
    assert!(out.contains_key("extra"));
}

#[test]
fn non_finite_number_is_dropped_with_a_warning() {
    let logs = captured_logs();
    let out = run(
        vec![
            ("nan", WitMetaValue::Number(f64::NAN)),
            ("inf", WitMetaValue::Number(f64::INFINITY)),
            ("ninf", WitMetaValue::Number(f64::NEG_INFINITY)),
            ("ok", WitMetaValue::Number(0.5)),
        ],
        &LOOSE,
    );
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out.get("ok"), Some(&json!(0.5)));
    let (_, msg) = captured_entry_containing(&logs, "NaN or infinite");
    assert!(msg.contains('3'), "{msg}");
}

#[test]
fn duplicate_key_keeps_the_first_and_warns() {
    let logs = captured_logs();
    let out = run(
        vec![("studio", text("first")), ("studio", text("second"))],
        &LOOSE,
    );
    assert_eq!(out.get("studio"), Some(&json!("first")));
    assert_eq!(out.len(), 1);
    captured_entry_containing(&logs, "repeating an earlier key");
}

#[test]
fn max_extras_boundary() {
    let logs = captured_logs();
    let caps = MetadataCaps { extras: 3, ..LOOSE };
    let at = run(
        vec![
            ("a", WitMetaValue::Flag(true)),
            ("b", WitMetaValue::Flag(true)),
            ("c", WitMetaValue::Flag(true)),
        ],
        &caps,
    );
    assert_eq!(at.len(), 3);

    let over = run(
        vec![
            ("a", WitMetaValue::Flag(true)),
            ("b", WitMetaValue::Flag(true)),
            ("c", WitMetaValue::Flag(true)),
            ("d", WitMetaValue::Flag(true)),
            ("e", WitMetaValue::Flag(true)),
        ],
        &caps,
    );
    assert_eq!(over.len(), 3);
    assert!(over.contains_key("a") && over.contains_key("c"));
    assert!(!over.contains_key("d") && !over.contains_key("e"));
    let needle = "2 extras past the 3-entry bound";
    let (target, _) = captured_entry_containing(&logs, needle);
    assert_eq!(target, TEST_LOG_TARGET);
    assert_eq!(
        captured_count_containing(&logs, needle),
        1,
        "warn once per class"
    );
}

/// Once the count bound is reached, later entries
/// are refused BEFORE the reserved-key probe (or any other per-entry
/// work) runs — observable here because reserved keys past the bound are
/// tallied as over-count, not as reserved. Three reserved keys under a
/// count bound of 1 therefore produce exactly one reserved refusal (one
/// probe) and two over-count refusals.
#[test]
fn entries_past_the_count_bound_skip_the_reserved_probe() {
    let logs = captured_logs();
    let caps = MetadataCaps { extras: 1, ..LOOSE };
    let out = run(
        vec![
            ("keep", WitMetaValue::Flag(true)),
            ("title", WitMetaValue::Flag(true)),
            ("id", WitMetaValue::Flag(true)),
            ("extractor", WitMetaValue::Flag(true)),
        ],
        &caps,
    );
    assert_eq!(out.len(), 1);
    assert!(out.contains_key("keep"));
    let needle = "3 extras past the 1-entry bound";
    captured_entry_containing(&logs, needle);
    assert_eq!(captured_count_containing(&logs, needle), 1);
    assert_eq!(
        captured_count_containing(&logs, "1 extras whose key shadows"),
        0,
        "no probe ran past the bound"
    );
}

/// The same short-circuit after the aggregate bound is crossed: reserved
/// keys arriving after it count as over-total, never as reserved.
#[test]
fn entries_after_the_aggregate_is_crossed_skip_the_reserved_probe() {
    let logs = captured_logs();
    let caps = MetadataCaps {
        total_bytes: 4,
        ..LOOSE
    };
    let out = run(
        vec![
            ("big", text("12345")),
            ("title", WitMetaValue::Flag(true)),
            ("id", WitMetaValue::Flag(true)),
        ],
        &caps,
    );
    assert!(out.is_empty(), "{out:?}");
    captured_entry_containing(&logs, "3 extras past the 4-byte aggregate bound");
    assert_eq!(
        captured_count_containing(&logs, "2 extras whose key shadows"),
        0
    );
}

/// `value_bytes` bounds a `text` by its UTF-8 bytes and a `text-list` by
/// its summed bytes plus [`METADATA_LIST_ITEM_BYTES`] per element, so the
/// pairs below sit exactly at the bound (32) and one past it.
#[test]
fn value_bytes_boundary() {
    let logs = captured_logs();
    let caps = MetadataCaps {
        value_bytes: 32,
        ..LOOSE
    };
    let out = run(
        vec![
            ("t32", text(&"x".repeat(32))),
            ("t33", text(&"x".repeat(33))),
            // 16 string bytes + 2 × 8 = 32; 17 + 16 = 33.
            (
                "l32",
                WitMetaValue::TextList(vec!["abcdefgh".into(), "abcdefgh".into()]),
            ),
            (
                "l33",
                WitMetaValue::TextList(vec!["abcdefgh".into(), "abcdefghi".into()]),
            ),
            // A multi-byte char counts its UTF-8 bytes, not its chars:
            // 16 × "ä" (2 bytes each) is 32 bytes, 17 × is 34.
            ("u32", text(&"ä".repeat(16))),
            ("u34", text(&"ä".repeat(17))),
        ],
        &caps,
    );
    assert!(out.contains_key("t32"), "{out:?}");
    assert!(!out.contains_key("t33"));
    assert!(out.contains_key("l32"));
    assert!(!out.contains_key("l33"));
    assert!(out.contains_key("u32"));
    assert!(!out.contains_key("u34"));
    assert_eq!(out.len(), 3);
    let needle = "3 extras whose value exceeds 32 bytes";
    let (_, msg) = captured_entry_containing(&logs, needle);
    assert!(
        msg.contains(&format!("{METADATA_LIST_ITEM_BYTES} per text-list item")),
        "{msg}"
    );
    assert_eq!(
        captured_count_containing(&logs, needle),
        1,
        "warn once per class"
    );
}

/// A `text-list`'s element COUNT is bounded even
/// when every element is empty — `value_bytes / METADATA_LIST_ITEM_BYTES`
/// empty strings are accepted, one more is refused. Summed string bytes
/// alone would have admitted any number of them.
#[test]
fn text_list_item_count_boundary() {
    let caps = MetadataCaps {
        value_bytes: 4 * METADATA_LIST_ITEM_BYTES,
        ..LOOSE
    };
    let at = run(
        vec![("l", WitMetaValue::TextList(vec![String::new(); 4]))],
        &caps,
    );
    assert_eq!(at.get("l").and_then(Value::as_array).map(Vec::len), Some(4));
    let over = run(
        vec![("l", WitMetaValue::TextList(vec![String::new(); 5]))],
        &caps,
    );
    assert!(over.is_empty(), "{over:?}");
}

/// The element charge reaches the aggregate too: 12 kept bytes, then a
/// two-empty-string list at 1 + 16 = 17 → 29 > 20, refused.
#[test]
fn text_list_item_charge_counts_toward_the_aggregate() {
    let caps = MetadataCaps {
        total_bytes: 20,
        ..LOOSE
    };
    let out = run(
        vec![
            ("a", text("123456789ab")),
            ("l", WitMetaValue::TextList(vec![String::new(); 2])),
        ],
        &caps,
    );
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out.contains_key("a"));
}

/// A scalar's fixed size is not something a plugin chooses, so the
/// per-value cap does not apply to it — even a cap smaller than
/// [`SCALAR_VALUE_BYTES`] keeps scalars.
#[test]
fn per_value_cap_does_not_refuse_scalars() {
    let caps = MetadataCaps {
        value_bytes: 1,
        ..LOOSE
    };
    let out = run(
        vec![
            ("i", WitMetaValue::Integer(1)),
            ("n", WitMetaValue::Number(1.0)),
            ("f", WitMetaValue::Flag(false)),
            ("t", text("xy")),
        ],
        &caps,
    );
    assert_eq!(out.len(), 3, "{out:?}");
    assert!(!out.contains_key("t"));
}

/// Aggregate accounting is key bytes + value bytes per kept entry,
/// scalars at [`SCALAR_VALUE_BYTES`]; the entry that would cross the
/// bound is dropped and nothing after it is admitted, even one that
/// would still fit.
#[test]
fn total_bytes_boundary() {
    let logs = captured_logs();
    let caps = MetadataCaps {
        total_bytes: 20,
        ..LOOSE
    };
    // 1 + 11 = 12, then 1 + 7 = 8 → exactly 20.
    let at = run(
        vec![("a", text("123456789ab")), ("b", text("1234567"))],
        &caps,
    );
    assert_eq!(at.len(), 2, "{at:?}");

    // 12, then an integer at 1 + 8 = 9 → 21: dropped; the empty text
    // after it (1 byte) would fit but the bound is already crossed.
    let over = run(
        vec![
            ("a", text("123456789ab")),
            ("b", WitMetaValue::Integer(1)),
            ("c", text("")),
        ],
        &caps,
    );
    assert_eq!(over.len(), 1, "{over:?}");
    assert!(over.contains_key("a"));
    let needle = "2 extras past the 20-byte aggregate bound";
    captured_entry_containing(&logs, needle);
    assert_eq!(
        captured_count_containing(&logs, needle),
        1,
        "warn once per class"
    );
}

#[test]
fn caps_come_from_config() {
    let c = rdlp_types::Config {
        max_metadata_extras: Some(1),
        max_metadata_value_bytes: Some(2),
        max_metadata_extras_bytes: Some(3),
        ..Default::default()
    };
    let caps = MetadataCaps::from(&c);
    assert_eq!(caps.extras, 1);
    assert_eq!(caps.value_bytes, 2);
    assert_eq!(caps.total_bytes, 3);
}

#[test]
fn unset_config_fields_keep_the_defaults() {
    let caps = MetadataCaps::from(&rdlp_types::Config::default());
    assert_eq!(caps.extras, DEFAULT_MAX_METADATA_EXTRAS);
    assert_eq!(caps.value_bytes, DEFAULT_MAX_METADATA_VALUE_BYTES);
    assert_eq!(caps.total_bytes, DEFAULT_MAX_METADATA_EXTRAS_BYTES);
    let d = MetadataCaps::default();
    assert_eq!(d.extras, DEFAULT_MAX_METADATA_EXTRAS);
    assert_eq!(d.value_bytes, DEFAULT_MAX_METADATA_VALUE_BYTES);
    assert_eq!(d.total_bytes, DEFAULT_MAX_METADATA_EXTRAS_BYTES);
}

// ---- Probe cost: the reserved-key probe is bounded by the caps, not by ----
// ---- the entry count a plugin chooses (#768 pre-push S1)               ----

/// [`admit_extras`] without the report, plus the tally's probe count.
fn run_counting(
    extras: Vec<(&str, WitMetaValue)>,
    caps: &MetadataCaps,
) -> (HashMap<String, Value>, usize) {
    let (out, tally) = admit_extras(
        extras
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        caps,
    );
    (out, tally.probes)
}

/// The same reserved key repeated never re-runs the probe: the verdict is
/// memoised per distinct key for the call, so 10 000 × `title` costs one
/// `InfoDict` deserialisation, not 10 000 on the extraction's worker
/// thread.
#[test]
fn a_repeated_reserved_key_is_probed_once() {
    let (out, probes) = run_counting(vec![("title", WitMetaValue::Flag(true)); 10_000], &LOOSE);
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(probes, 1, "one probe per distinct reserved key");
}

/// A duplicate of an already-admitted key is refused BEFORE the probe —
/// the first `a` is probed once when admitted, the 10 000 repeats never
/// are.
#[test]
fn duplicates_of_an_admitted_key_are_never_probed() {
    let (out, probes) = run_counting(vec![("a", WitMetaValue::Flag(true)); 10_001], &LOOSE);
    assert_eq!(out.len(), 1);
    assert_eq!(probes, 1, "only the admitted first `a` is probed");
}

/// The refusal budget, at its boundary. Every refusal that cost per-entry
/// work counts toward `caps.extras`; once that many have been spent, the
/// rest of the list is refused as over-count with no further work. With
/// `extras: 3`, three distinct reserved keys leave a later legitimate key
/// admissible (four probes in all, the legitimate key's included); four
/// exhaust the budget and the legitimate key after them is refused
/// without a probe — four probes again, but `keep` is not among them.
#[test]
fn distinct_reserved_keys_boundary_of_the_refusal_budget() {
    let caps = MetadataCaps { extras: 3, ..LOOSE };
    let flag = WitMetaValue::Flag(true);
    let (at, probes) = run_counting(
        vec![
            ("title", flag.clone()),
            ("id", flag.clone()),
            ("extractor", flag.clone()),
            ("keep", flag.clone()),
        ],
        &caps,
    );
    assert!(at.contains_key("keep"), "{at:?}");
    assert_eq!(probes, 4);

    let logs = captured_logs();
    let (over, probes) = run_counting(
        vec![
            ("title", flag.clone()),
            ("id", flag.clone()),
            ("extractor", flag.clone()),
            ("formats", flag.clone()),
            ("keep", flag),
        ],
        &caps,
    );
    assert!(over.is_empty(), "{over:?}");
    assert_eq!(probes, 4, "the fifth entry is refused without a probe");
    // Through the reporting entry point, so the class the budget files
    // the refusal under is pinned too.
    run(
        vec![
            ("title", WitMetaValue::Flag(true)),
            ("id", WitMetaValue::Flag(true)),
            ("extractor", WitMetaValue::Flag(true)),
            ("formats", WitMetaValue::Flag(true)),
            ("keep", WitMetaValue::Flag(true)),
        ],
        &caps,
    );
    captured_entry_containing(&logs, "1 extras past the 3-entry bound");
}

/// A value refusal costs work too, so it spends the same budget — and the
/// value is checked before the probe, so a refused value never buys a
/// probe. Three distinct keys with a NaN value under `extras: 2` exhaust
/// the budget without one probe, and the legitimate key after them is
/// refused.
#[test]
fn value_refusals_spend_the_budget_and_skip_the_probe() {
    let caps = MetadataCaps { extras: 2, ..LOOSE };
    let (out, probes) = run_counting(
        vec![
            ("n1", WitMetaValue::Number(f64::NAN)),
            ("n2", WitMetaValue::Number(f64::NAN)),
            ("n3", WitMetaValue::Number(f64::NAN)),
            ("keep", WitMetaValue::Flag(true)),
        ],
        &caps,
    );
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(probes, 0);
}
