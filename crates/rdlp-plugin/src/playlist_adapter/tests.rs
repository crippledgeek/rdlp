use super::*;
use crate::test_harness::instantiate;

/// A component with no `extract-playlist` export at all.
const NO_PLAYLIST_WAT: &str = r#"(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
)"#;

/// `extract-playlist` exported with the wrong type (`u32` instead of the
/// result). Needs `memory`/`realloc` exports because the component-level
/// signature still declares a `string` param — the canonical ABI must be
/// able to lower it into guest memory even though the call is expected to
/// fail typecheck before the core function ever runs.
const PLAYLIST_WRONG_TYPE_WAT: &str = r#"(component
  (core module $m
    (memory (export "mem") 1)
    (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 200))
    (func (export "extract-playlist") (param i32 i32 i32) (result i32) (i32.const 7)))
  (core instance $i (instantiate $m))
  (func (export "extract-playlist") (param "url" string) (param "page" u32) (result u32)
    (canon lift (core func $i "extract-playlist") (memory $i "mem") (realloc (func $i "realloc"))
      string-encoding=utf8))
)"#;

#[tokio::test]
async fn absent_export_is_ok_none() {
    let (mut store, inst) = instantiate(NO_PLAYLIST_WAT).await;
    let r = call_extract_playlist(
        &mut store,
        &inst,
        PlaylistPageRequest {
            url: "https://x.example/u/a",
            page: 1,
        },
    )
    .await
    .expect("no trap");
    assert!(r.is_none());
}

#[tokio::test]
async fn mis_typed_export_is_a_trap_and_a_strike() {
    use crate::adapter::counts_as_strike;
    let (mut store, inst) = instantiate(PLAYLIST_WRONG_TYPE_WAT).await;
    let err = call_extract_playlist(
        &mut store,
        &inst,
        PlaylistPageRequest {
            url: "https://x.example/u/a",
            page: 1,
        },
    )
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
fn playlist_entry_lift_mirrors_the_wit_record_field_for_field() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "url: string,",
        "id: option<string>,",
        "title: option<string>,",
    ];
    let (_, after) = TYPES_WIT
        .split_once("record playlist-entry {")
        .expect("types.wit declares playlist-entry");
    let (body, _) = after.split_once('}').expect("record body is closed");
    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        fields, expected,
        "playlist-entry drifted from the Rust lift"
    );
}

#[test]
fn playlist_page_lift_mirrors_the_wit_record_field_for_field() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "entries: list<playlist-entry>,",
        "page: u32,",
        "has-more: bool,",
        "playlist-id: option<string>,",
        "playlist-title: option<string>,",
        "total-estimate: option<u64>,",
    ];
    let (_, after) = TYPES_WIT
        .split_once("record playlist-page {")
        .expect("types.wit declares playlist-page");
    let (body, _) = after.split_once('}').expect("record body is closed");
    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(fields, expected, "playlist-page drifted from the Rust lift");
}

#[test]
fn playlist_error_lift_mirrors_the_wit_variant_case_for_case() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "unsupported-url(string),",
        "not-found(string),",
        "rate-limited(option<u32>),",
        "network(string),",
        "parse(string),",
        "internal(string),",
    ];
    let (_, after) = TYPES_WIT
        .split_once("variant playlist-error {")
        .expect("types.wit declares playlist-error");
    let (body, _) = after.split_once('}').expect("variant body is closed");
    let cases: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(cases, expected, "playlist-error drifted from the Rust lift");
}

#[test]
fn playlist_error_mapping_strikes_only_internal() {
    use crate::adapter::counts_as_strike;
    for (e, strike) in [
        (WitPlaylistError::UnsupportedUrl("u".into()), false),
        (WitPlaylistError::NotFound("u".into()), false),
        (WitPlaylistError::RateLimited(Some(3)), false),
        (WitPlaylistError::Network(String::new()), false),
        (WitPlaylistError::Parse(String::new()), false),
        (WitPlaylistError::Internal(String::new()), true),
    ] {
        let mapped = playlist_error_to_plugin_error("p", e);
        assert_eq!(counts_as_strike(&mapped), strike, "{mapped:?}");
    }
}

/// The end-to-end lift of a real `playlist-page` needs a fixture component
/// (Task 10); this pins the hand-lift's field shape by constructing one
/// plainly and reading it back, so every `WitPlaylistPage`/`WitPlaylistEntry`
/// field — including `total_estimate`, which no other test in this module
/// touches — is exercised without a WASM round trip.
#[test]
fn playlist_page_carries_every_field_through() {
    let entry = WitPlaylistEntry {
        url: "https://x.example/v/1".into(),
        id: Some("1".into()),
        title: Some("Clip One".into()),
    };
    let page = WitPlaylistPage {
        entries: vec![entry],
        page: 2,
        has_more: true,
        playlist_id: Some("pl-1".into()),
        playlist_title: Some("A Playlist".into()),
        total_estimate: Some(42),
    };
    assert_eq!(
        page.entries.first().map(|e| e.url.as_str()),
        Some("https://x.example/v/1")
    );
    assert_eq!(page.page, 2);
    assert!(page.has_more);
    assert_eq!(page.playlist_id.as_deref(), Some("pl-1"));
    assert_eq!(page.playlist_title.as_deref(), Some("A Playlist"));
    assert_eq!(page.total_estimate, Some(42));
}
