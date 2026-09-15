use super::*;
use crate::convert::{ExtractionSite, info_dict_from_extraction};
use crate::test_harness::instantiate;
use crate::test_support::unit::test_origin;

/// A minimal `info-dict` core — only `id`/`title` vary across callers, so
/// [`typed_extra_fields_reach_info_dict`] and [`empty_thumbnails_is_none`]
/// share this rather than repeating every `None`/`Vec::new()` field.
fn minimal_core() -> WitInfoDict {
    WitInfoDict {
        id: "abc123".into(),
        title: "A Title".into(),
        url: None,
        formats: Vec::new(),
        subtitles: Vec::new(),
        thumbnail: None,
        description: None,
        uploader: None,
        uploader_id: None,
        upload_date: None,
        duration: None,
        view_count: None,
        like_count: None,
        tags: Vec::new(),
        categories: Vec::new(),
    }
}

/// A component with no `extract-with-metadata` export at all.
const NO_METADATA_WAT: &str = r#"(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
)"#;

/// `extract-with-metadata` exported with the wrong type (`u32` instead of
/// the result). Needs `memory`/`realloc` exports for the same reason as
/// `playlist_adapter`'s wrong-type fixture — see its comment.
const METADATA_WRONG_TYPE_WAT: &str = r#"(component
  (core module $m
    (memory (export "mem") 1)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 200))
    (func (export "extract-with-metadata") (param i32 i32) (result i32) (i32.const 7)))
  (core instance $i (instantiate $m))
  (func (export "extract-with-metadata") (param "url" string) (result u32)
    (canon lift (core func $i "extract-with-metadata") (memory $i "mem") (realloc (func $i "realloc"))
      string-encoding=utf8))
)"#;

#[tokio::test]
async fn absent_export_is_ok_none() {
    let (mut store, inst) = instantiate(NO_METADATA_WAT).await;
    let r = call_extract_with_metadata(&mut store, &inst, "https://x.example/u/a")
        .await
        .expect("no trap");
    assert!(r.is_none());
}

#[tokio::test]
async fn mis_typed_export_is_a_trap_and_a_strike() {
    use crate::adapter::counts_as_strike;
    let (mut store, inst) = instantiate(METADATA_WRONG_TYPE_WAT).await;
    let err = call_extract_with_metadata(&mut store, &inst, "https://x.example/u/a")
        .await
        .unwrap_err();
    assert!(matches!(err, PluginError::Trapped { .. }), "{err:?}");
    assert!(counts_as_strike(&err));
}

/// Drift guard for the hand-written lifts, mirroring
/// `search_adapter::tests::lift_mirrors_the_wit_record_field_for_field`:
/// each record/variant's field lines in `wit/types.wit` must be exactly
/// these, in this order. wasmtime typechecks the lift by field/case
/// name, type, AND order at `get_typed_func`
/// (`wasmtime::component::func::typed::typecheck_record`/`typecheck_variant`,
/// wasmtime 30.0.2) — a drifted hand-lift would trap, and strike, every
/// plugin at call time rather than fail silently, so this test exists to
/// catch the drift here, at test time, before that happens.
#[test]
fn thumbnail_lift_mirrors_the_wit_record_field_for_field() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "url: string,",
        "id: option<string>,",
        "width: option<u32>,",
        "height: option<u32>,",
        "preference: option<s32>,",
    ];
    let (_, after) = TYPES_WIT
        .split_once("record thumbnail {")
        .expect("types.wit declares thumbnail");
    let (body, _) = after.split_once('}').expect("record body is closed");
    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(fields, expected, "thumbnail drifted from the Rust lift");
}

#[test]
fn meta_value_lift_mirrors_the_wit_variant_case_for_case() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "text(string),",
        "integer(s64),",
        "number(f64),",
        "flag(bool),",
        "text-list(list<string>),",
    ];
    let (_, after) = TYPES_WIT
        .split_once("variant meta-value {")
        .expect("types.wit declares meta-value");
    let (body, _) = after.split_once('}').expect("variant body is closed");
    let cases: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(cases, expected, "meta-value drifted from the Rust lift");
}

#[test]
fn info_dict_extra_lift_mirrors_the_wit_record_field_for_field() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "actors: list<string>,",
        "channel: option<string>,",
        "channel-url: option<string>,",
        "age-limit: option<u8>,",
        "thumbnails: list<thumbnail>,",
        "extras: list<tuple<string, meta-value>>,",
    ];
    let (_, after) = TYPES_WIT
        .split_once("record info-dict-extra {")
        .expect("types.wit declares info-dict-extra");
    let (body, _) = after.split_once('}').expect("record body is closed");
    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        fields, expected,
        "info-dict-extra drifted from the Rust lift"
    );
}

#[test]
fn extraction_lift_mirrors_the_wit_record_field_for_field() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = ["core: info-dict,", "extra: info-dict-extra,"];
    let (_, after) = TYPES_WIT
        .split_once("record extraction {")
        .expect("types.wit declares extraction");
    let (body, _) = after.split_once('}').expect("record body is closed");
    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(fields, expected, "extraction drifted from the Rust lift");
}

/// The end-to-end lift of a real `extraction` record needs a fixture
/// component (Task 10); this pins every hand-lift's field shape by
/// constructing one of each plainly and reading every field back — so
/// `WitThumbnail.preference`, `WitInfoDictExtra.{channel_url,age_limit,
/// thumbnails}`, and every `WitMetaValue` variant (none of which any other
/// test in this module touches) are exercised without a WASM round trip.
#[test]
fn extraction_carries_every_field_through() {
    let thumb = WitThumbnail {
        url: "https://x.example/t/1.jpg".into(),
        id: Some("t1".into()),
        width: Some(320),
        height: Some(180),
        preference: Some(-1),
    };
    let extra_values = [
        WitMetaValue::Text("studio".into()),
        WitMetaValue::Integer(-7),
        WitMetaValue::Number(1.5),
        WitMetaValue::Flag(true),
        WitMetaValue::TextList(vec!["a".into(), "b".into()]),
    ];
    assert!(matches!(&extra_values[0], WitMetaValue::Text(s) if s == "studio"));
    assert!(matches!(&extra_values[1], WitMetaValue::Integer(-7)));
    assert!(matches!(&extra_values[2], WitMetaValue::Number(n) if (*n - 1.5).abs() < f64::EPSILON));
    assert!(matches!(&extra_values[3], WitMetaValue::Flag(true)));
    assert!(matches!(&extra_values[4], WitMetaValue::TextList(v) if v.len() == 2));

    let core = WitInfoDict {
        id: "abc123".into(),
        title: "A Title".into(),
        url: None,
        formats: Vec::new(),
        subtitles: Vec::new(),
        thumbnail: None,
        description: None,
        uploader: None,
        uploader_id: None,
        upload_date: None,
        duration: None,
        view_count: None,
        like_count: None,
        tags: Vec::new(),
        categories: Vec::new(),
    };
    let extra = WitInfoDictExtra {
        actors: vec!["Alice".into()],
        channel: Some("chan".into()),
        channel_url: Some("https://x.example/c/chan".into()),
        age_limit: Some(18),
        thumbnails: vec![thumb],
        extras: vec![("studio".into(), WitMetaValue::Text("Acme".into()))],
    };
    let extraction = WitExtraction { core, extra };

    assert_eq!(extraction.core.id, "abc123");
    assert_eq!(extraction.extra.channel.as_deref(), Some("chan"));
    assert_eq!(
        extraction.extra.channel_url.as_deref(),
        Some("https://x.example/c/chan")
    );
    assert_eq!(extraction.extra.age_limit, Some(18));
    assert_eq!(
        extraction
            .extra
            .thumbnails
            .first()
            .and_then(|t| t.preference),
        Some(-1)
    );
    assert_eq!(extraction.extra.extras.len(), 1);
}

/// Task 6: `info_dict_from_extraction` copies every typed `info-dict-extra`
/// field onto the `InfoDict` it builds around [`info_dict_from_wit`]'s core
/// conversion — `actors`/`channel`/`channel_url`/`age_limit` verbatim, and
/// a non-empty thumbnail list wrapped in `Some`.
#[test]
fn typed_extra_fields_reach_info_dict() {
    let extraction = WitExtraction {
        core: minimal_core(),
        extra: WitInfoDictExtra {
            actors: vec!["a".into(), "b".into()],
            channel: Some("c".into()),
            channel_url: Some("https://x.example/c".into()),
            age_limit: Some(18),
            thumbnails: vec![WitThumbnail {
                url: "https://x.example/t.jpg".into(),
                id: Some("t1".into()),
                width: Some(320),
                height: Some(180),
                preference: Some(2),
            }],
            extras: Vec::new(),
        },
    };
    let caps = MetadataCaps::default();
    let site = ExtractionSite {
        url: "https://x.example/v/1",
        origin: test_origin(),
        caps: &caps,
    };

    let out = info_dict_from_extraction(extraction, &site);

    assert_eq!(out.actors, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(out.channel.as_deref(), Some("c"));
    assert_eq!(out.channel_url.as_deref(), Some("https://x.example/c"));
    assert_eq!(out.age_limit, Some(18));
    let thumbs = out.thumbnails.expect("non-empty thumbnails become Some");
    assert_eq!(thumbs.len(), 1);
    let thumb = thumbs.first().expect("checked len() == 1 above");
    assert_eq!(thumb.url, "https://x.example/t.jpg");
    assert_eq!(thumb.id.as_deref(), Some("t1"));
    assert_eq!(thumb.width, Some(320));
    assert_eq!(thumb.height, Some(180));
    assert_eq!(thumb.preference, Some(2));
}

/// An `info-dict-extra` with no thumbnails at all must leave
/// `InfoDict::thumbnails` as `None`, not `Some(vec![])` — the same
/// empty-collapses-to-None convention `info_dict_from_wit` already applies
/// to `tags`/`categories`.
#[test]
fn empty_thumbnails_is_none() {
    let extraction = WitExtraction {
        core: minimal_core(),
        extra: WitInfoDictExtra {
            actors: Vec::new(),
            channel: None,
            channel_url: None,
            age_limit: None,
            thumbnails: Vec::new(),
            extras: Vec::new(),
        },
    };
    let caps = MetadataCaps::default();
    let site = ExtractionSite {
        url: "https://x.example/v/1",
        origin: test_origin(),
        caps: &caps,
    };

    let out = info_dict_from_extraction(extraction, &site);

    assert!(out.thumbnails.is_none());
}

// ---- Task 7: `extras_from_wit` — validation, caps, JSON mapping ----

mod extras {
    use super::super::*;
    use crate::test_support::unit::{
        LogEntries, TEST_LOG_TARGET, captured_entry_containing, captured_logs, test_origin,
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

    /// How many captured entries on the test target contain `needle` —
    /// the "warn once per refusal class" assertions need a count, not
    /// just presence.
    fn warn_count(logs: &LogEntries, needle: &str) -> usize {
        logs.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(t, m)| t == TEST_LOG_TARGET && m.contains(needle))
            .count()
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
        assert_eq!(warn_count(&logs, needle), 1, "warn once per class");
    }

    #[test]
    fn value_bytes_boundary() {
        let logs = captured_logs();
        let caps = MetadataCaps {
            value_bytes: 8,
            ..LOOSE
        };
        let out = run(
            vec![
                ("t8", text("12345678")),
                ("t9", text("123456789")),
                (
                    "l8",
                    WitMetaValue::TextList(vec!["abcd".into(), "efgh".into()]),
                ),
                (
                    "l9",
                    WitMetaValue::TextList(vec!["abcd".into(), "efghi".into()]),
                ),
                // A multi-byte char counts its UTF-8 bytes, not its chars:
                // 4 × "ä" (2 bytes each) is 8 bytes, 5 × is 10.
                ("u8", text("ääää")),
                ("u10", text("äääää")),
            ],
            &caps,
        );
        assert!(out.contains_key("t8"), "{out:?}");
        assert!(!out.contains_key("t9"));
        assert!(out.contains_key("l8"));
        assert!(!out.contains_key("l9"));
        assert!(out.contains_key("u8"));
        assert!(!out.contains_key("u10"));
        assert_eq!(out.len(), 3);
        let needle = "3 extras whose value exceeds 8 bytes";
        captured_entry_containing(&logs, needle);
        assert_eq!(warn_count(&logs, needle), 1, "warn once per class");
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
        assert_eq!(warn_count(&logs, needle), 1, "warn once per class");
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

    /// End to end through `info_dict_from_extraction`: a kept extra is a
    /// TOP-LEVEL key of the serialized `InfoDict` (`extra` is
    /// `#[serde(flatten)]`), which is what `--dump-json` prints and what
    /// `%(studio)s` resolves through the template renderer's JSON lookup.
    #[test]
    fn extras_reach_info_dict_extra_top_level_json() {
        use crate::convert::{ExtractionSite, info_dict_from_extraction};
        let extraction = WitExtraction {
            core: super::minimal_core(),
            extra: WitInfoDictExtra {
                actors: Vec::new(),
                channel: None,
                channel_url: None,
                age_limit: None,
                thumbnails: Vec::new(),
                extras: vec![
                    ("studio".into(), text("acme")),
                    ("title".into(), text("shadow")),
                ],
            },
        };
        let caps = MetadataCaps::default();
        let site = ExtractionSite {
            url: "https://x.example/v/1",
            origin: test_origin(),
            caps: &caps,
        };
        let info = info_dict_from_extraction(extraction, &site);
        assert_eq!(info.extra.get("studio"), Some(&json!("acme")));
        let v = serde_json::to_value(&info).expect("InfoDict serializes");
        assert_eq!(v.get("studio"), Some(&json!("acme")));
        assert_eq!(
            v.get("title"),
            Some(&json!("A Title")),
            "a shadowing extra never reaches JSON"
        );
        assert!(v.get("extra").is_none(), "flattened, not nested");
    }

    /// The caps the site carries are the ones applied — not `Default`.
    #[test]
    fn info_dict_from_extraction_applies_the_site_caps() {
        use crate::convert::{ExtractionSite, info_dict_from_extraction};
        let extraction = WitExtraction {
            core: super::minimal_core(),
            extra: WitInfoDictExtra {
                actors: Vec::new(),
                channel: None,
                channel_url: None,
                age_limit: None,
                thumbnails: Vec::new(),
                extras: vec![("a".into(), text("x")), ("b".into(), text("y"))],
            },
        };
        let caps = MetadataCaps { extras: 1, ..LOOSE };
        let site = ExtractionSite {
            url: "https://x.example/v/1",
            origin: test_origin(),
            caps: &caps,
        };
        let info = info_dict_from_extraction(extraction, &site);
        assert_eq!(info.extra.len(), 1);
        assert!(info.extra.contains_key("a"));
    }
}
