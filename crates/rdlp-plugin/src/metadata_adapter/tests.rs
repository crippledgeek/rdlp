use super::*;
use crate::adapter::SEARCH_TIMEOUT;
use crate::instance::PluginStoreData;

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

async fn instantiate(
    wat: &str,
) -> (
    wasmtime::Store<PluginStoreData>,
    wasmtime::component::Instance,
) {
    use crate::engine::{Engine, EngineConfig};
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let component =
        wasmtime::component::Component::new(engine.raw(), wat::parse_str(wat).expect("wat"))
            .expect("component");
    let mut store = crate::instance::build_store(
        &engine,
        "test",
        tokio_util::sync::CancellationToken::new(),
        crate::instance::deadline_ticks(SEARCH_TIMEOUT, engine.tick_period()),
    );
    let instance = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw())
        .instantiate_async(&mut store, &component)
        .await
        .expect("instantiate");
    (store, instance)
}

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
/// these, in this order — the component canonical ABI lifts positionally,
/// so a reordered or retyped field would lift garbage without a type error.
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
/// component (Task 10); this pins the hand-lift's field shape by
/// constructing one plainly and reading it back, so `WitExtraction`'s
/// `core`/`extra` fields are exercised without a WASM round trip.
#[test]
fn extraction_carries_core_and_extra_through() {
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
        channel_url: None,
        age_limit: None,
        thumbnails: Vec::new(),
        extras: Vec::new(),
    };
    let extraction = WitExtraction { core, extra };
    assert_eq!(extraction.core.id, "abc123");
    assert_eq!(extraction.extra.channel.as_deref(), Some("chan"));
}
