use super::*;
use crate::convert::{ExtractionSite, info_dict_from_extraction};
use crate::test_harness::{EMPTY_COMPONENT_WAT, instantiate, wit_body_lines};
use crate::test_support::unit::test_origin;
use serde_json::json;

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

/// This export's "absent" meaning: a component without
/// `extract-with-metadata` is `Ok(None)` for `call_plugin_extract` to
/// fall through on. The lookup/typecheck/trap mechanism itself is
/// `adapter::tests::by_name_*`.
#[tokio::test]
async fn absent_export_is_ok_none() {
    let (mut store, inst) = instantiate(EMPTY_COMPONENT_WAT).await;
    let r = call_extract_with_metadata(&mut store, &inst, "https://x.example/u/a")
        .await
        .expect("no trap");
    assert!(r.is_none());
}

/// Drift guards for the hand-written lifts (`test_harness::wit_body_lines`
/// says why): each record/variant's lines in `wit/types.wit` must be
/// exactly these, in this order.
#[test]
fn thumbnail_lift_mirrors_the_wit_record_field_for_field() {
    assert_eq!(
        wit_body_lines("record", "thumbnail"),
        [
            "url: string,",
            "id: option<string>,",
            "width: option<u32>,",
            "height: option<u32>,",
            "preference: option<s32>,",
        ],
        "thumbnail drifted from the Rust lift"
    );
}

#[test]
fn meta_value_lift_mirrors_the_wit_variant_case_for_case() {
    assert_eq!(
        wit_body_lines("variant", "meta-value"),
        [
            "text(string),",
            "integer(s64),",
            "number(f64),",
            "flag(bool),",
            "text-list(list<string>),",
        ],
        "meta-value drifted from the Rust lift"
    );
}

#[test]
fn info_dict_extra_lift_mirrors_the_wit_record_field_for_field() {
    assert_eq!(
        wit_body_lines("record", "info-dict-extra"),
        [
            "actors: list<string>,",
            "channel: option<string>,",
            "channel-url: option<string>,",
            "age-limit: option<u8>,",
            "thumbnails: list<thumbnail>,",
            "extras: list<tuple<string, meta-value>>,",
        ],
        "info-dict-extra drifted from the Rust lift"
    );
}

#[test]
fn extraction_lift_mirrors_the_wit_record_field_for_field() {
    assert_eq!(
        wit_body_lines("record", "extraction"),
        ["core: info-dict,", "extra: info-dict-extra,"],
        "extraction drifted from the Rust lift"
    );
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

// ---- Task 7: `extras` reach `InfoDict::extra` through `info_dict_from_extraction` ----

/// End to end through `info_dict_from_extraction`: a kept extra is a
/// TOP-LEVEL key of the serialized `InfoDict` (`extra` is
/// `#[serde(flatten)]`), which is what `--dump-json` prints and what
/// `%(studio)s` resolves through the template renderer's JSON lookup.
#[test]
fn extras_reach_info_dict_extra_top_level_json() {
    let extraction = WitExtraction {
        core: minimal_core(),
        extra: WitInfoDictExtra {
            actors: Vec::new(),
            channel: None,
            channel_url: None,
            age_limit: None,
            thumbnails: Vec::new(),
            extras: vec![
                ("studio".into(), WitMetaValue::Text("acme".into())),
                ("title".into(), WitMetaValue::Text("shadow".into())),
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
    let extraction = WitExtraction {
        core: minimal_core(),
        extra: WitInfoDictExtra {
            actors: Vec::new(),
            channel: None,
            channel_url: None,
            age_limit: None,
            thumbnails: Vec::new(),
            extras: vec![
                ("a".into(), WitMetaValue::Text("x".into())),
                ("b".into(), WitMetaValue::Text("y".into())),
            ],
        },
    };
    let caps = MetadataCaps {
        extras: 1,
        ..MetadataCaps::default()
    };
    let site = ExtractionSite {
        url: "https://x.example/v/1",
        origin: test_origin(),
        caps: &caps,
    };
    let info = info_dict_from_extraction(extraction, &site);
    assert_eq!(info.extra.len(), 1);
    assert!(info.extra.contains_key("a"));
}
