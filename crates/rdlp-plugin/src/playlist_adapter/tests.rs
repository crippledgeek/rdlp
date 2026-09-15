use super::*;
use crate::convert::MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES;
use crate::test_harness::instantiate;
use crate::test_support::unit::{
    FIXTURE_MANIFEST, TEST_LOG_TARGET, captured_entry_containing, captured_logs, fixture_extractor,
    fixture_extractor_from, test_origin,
};
use crate::test_support::{EXAMPLE_0_5_2_WASM, extraction_ctx};

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

/// `playlist_page_from_wit` copies every field verbatim (entries included)
/// from the lifted `WitPlaylistPage` into the host-owned loop's own
/// `PlaylistPage`/`PlaylistEntry`.
#[test]
fn page_fetch_maps_wit_page_to_playlist_page() {
    let w = WitPlaylistPage {
        entries: vec![WitPlaylistEntry {
            url: "https://x.example/v/1".into(),
            id: Some("1".into()),
            title: Some("Clip One".into()),
        }],
        page: 2,
        has_more: true,
        playlist_id: Some("pl-1".into()),
        playlist_title: Some("A Playlist".into()),
        total_estimate: Some(42),
    };
    let page = playlist_page_from_wit(w, &test_origin());
    assert_eq!(page.entries.len(), 1);
    let entry = page.entries.first().expect("one entry");
    assert_eq!(entry.url, "https://x.example/v/1");
    assert_eq!(entry.id.as_deref(), Some("1"));
    assert_eq!(entry.title.as_deref(), Some("Clip One"));
    assert!(page.has_more);
    assert_eq!(page.playlist_id.as_deref(), Some("pl-1"));
    assert_eq!(page.playlist_title.as_deref(), Some("A Playlist"));
    assert_eq!(page.total_estimate, Some(42));
}

/// `playlist_page_from_wit` caps `entries` the same way
/// `info_dict_from_wit` caps `formats` (`convert::format_cap_tests`):
/// truncate to the bound, warn once on the plugin's log target, naming the
/// call and the count.
#[test]
fn page_fetch_caps_entries_and_warns() {
    let logs = captured_logs();
    let entries: Vec<WitPlaylistEntry> = (0..=MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES)
        .map(|i| WitPlaylistEntry {
            url: format!("https://x.example/v/{i}"),
            id: None,
            title: None,
        })
        .collect();
    let w = WitPlaylistPage {
        entries,
        page: 1,
        has_more: false,
        playlist_id: None,
        playlist_title: None,
        total_estimate: None,
    };
    let page = playlist_page_from_wit(w, &test_origin());
    assert_eq!(page.entries.len(), MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES);
    let (target, msg) = captured_entry_containing(
        &logs,
        &format!(
            "extract-playlist: plugin test supplied {} playlist entries",
            MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES + 1
        ),
    );
    assert_eq!(target, TEST_LOG_TARGET);
    assert!(
        msg.contains(&MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES.to_string()),
        "{msg}"
    );
}

/// The N-side of the boundary `page_fetch_caps_entries_and_warns` pins
/// from N+1: exactly `MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES` entries pass
/// through unchanged and must NOT be reported as capped (fix round 1
/// finding 6 — the `>=`-instead-of-`>` off-by-one this pins would
/// false-positive exactly here). The log buffer is process-global,
/// never cleared, and shared with concurrently-running tests, so this
/// asserts the ABSENCE of the one message text only a wrong comparison at
/// this exact count would produce, rather than a before/after length
/// delta (which a concurrent test's own warn can flip under parallel
/// execution).
#[test]
fn page_fetch_does_not_cap_or_warn_at_exactly_the_bound() {
    let logs = captured_logs();
    let entries: Vec<WitPlaylistEntry> = (0..MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES)
        .map(|i| WitPlaylistEntry {
            url: format!("https://x.example/v/{i}"),
            id: None,
            title: None,
        })
        .collect();
    let w = WitPlaylistPage {
        entries,
        page: 1,
        has_more: false,
        playlist_id: None,
        playlist_title: None,
        total_estimate: None,
    };
    let page = playlist_page_from_wit(w, &test_origin());
    assert_eq!(page.entries.len(), MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES);
    let false_positive_cap_message = format!(
        "extract-playlist: plugin test supplied {MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES} playlist entries"
    );
    let wrongly_capped = logs
        .lock()
        .expect("test mutex is never poisoned")
        .iter()
        .any(|(_, m)| m.contains(&false_positive_cap_message));
    assert!(
        !wrongly_capped,
        "exactly the bound must not warn (a >= instead of > would false-positive here)"
    );
}

// ── `PluginExtractor::extract_playlist` (the probe + fallback) ───────────

/// A plugin whose component never declared `extract-playlist` (the
/// committed 0.5.0 fixture) falls back to the trait default: one
/// `extract` call, wrapped in a one-element `Vec`, with no playlist index
/// stamped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_export_falls_back_to_single_extract() {
    let ext = fixture_extractor();
    let out = ext
        .extract_playlist("https://example.com/video/42", &extraction_ctx())
        .await
        .expect("falls back to the single extract");
    assert_eq!(out.len(), 1);
    let info = out.first().expect("one entry");
    assert_eq!(info.title, "Example Video 42");
    assert_eq!(info.playlist_index, None);
}

// The four tests below load the committed 0.5.2 fixture
// (`test_support::EXAMPLE_0_5_2_WASM`): a WAT component cannot export
// both `extract-playlist` and the full 0.5.0 `extract` surface without
// reproducing `info-dict`/`extract-error`'s whole shape by hand, so a
// real compiled component is the only way to drive the probe-then-fall-
// back path end to end. What its `extract-playlist` answers per URL is
// pinned in `examples/plugins/example-extractor/src/lib.rs`.

/// A plugin whose `extract-playlist` answers `err(unsupported-url)` for
/// this URL falls back the same way `absent_export_falls_back_to_single_extract`
/// does: the fixture's `extract` accepts this URL as a single video, so
/// the fallback is the one entry it produces.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_url_falls_back_to_single_extract() {
    let ext = fixture_extractor_from(FIXTURE_MANIFEST, EXAMPLE_0_5_2_WASM);
    let out = ext
        .extract_playlist("https://example.com/not-a-playlist", &extraction_ctx())
        .await
        .expect("falls back to the single extract");
    assert_eq!(out.len(), 1);
    assert_eq!(out.first().expect("one entry").playlist_index, None);
    assert_eq!(ext.test_trap_count(), 0);
}

/// `internal` from `extract-playlist` on page one strikes exactly like any
/// other `PluginError::Internal` — regression coverage for fix round 1
/// finding 1 (mapping the domain error INSIDE the runner's closure, where
/// `counts_as_strike` can see it, instead of after `run_in_fresh_store`
/// already returned). The 0.5.2 fixture's `extract-playlist` answers
/// `err(internal(...))` for this URL.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn internal_playlist_error_on_page_1_strikes() {
    let ext = fixture_extractor_from(FIXTURE_MANIFEST, EXAMPLE_0_5_2_WASM);
    let err = ext
        .extract_playlist(
            "https://example.com/internal-error-playlist",
            &extraction_ctx(),
        )
        .await
        .expect_err("an internal domain error propagates rather than falling back");
    assert!(err.to_string().contains("example"), "{err}");
    assert_eq!(
        ext.test_trap_count(),
        1,
        "an internal extract-playlist error must strike like any other PluginError::Internal"
    );
}

/// A `not-found` domain error on page one propagates as its own error —
/// the "any other domain error" branch of `extract_playlist_via_plugin`'s
/// match, distinct from `unsupported-url`'s fallback. The 0.5.2
/// fixture's `extract-playlist` answers `err(not-found(...))` for this URL.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_found_on_page_1_propagates() {
    let ext = fixture_extractor_from(FIXTURE_MANIFEST, EXAMPLE_0_5_2_WASM);
    let err = ext
        .extract_playlist("https://example.com/gone-playlist", &extraction_ctx())
        .await
        .expect_err("not-found propagates rather than falling back to extract");
    assert!(matches!(err, RdlpError::Extraction { .. }), "{err:?}");
    assert!(
        err.to_string().contains("reported resource not found"),
        "{err}"
    );
    // Not a strike: `NotFound` is a domain outcome, same as `UnsupportedUrl`.
    assert_eq!(ext.test_trap_count(), 0);
}

/// A real first page (at least one entry) hands off to the shared
/// `PagedPlaylist` scaffold, and page one is fetched EXACTLY ONCE — the
/// probe's own page is reused, never re-fetched — proved via the
/// by-name-call debug line `call_extract_playlist_page` logs on every
/// attempt. The 0.5.2 fixture's `extract-playlist` answers two pages for
/// this URL (page 1 `has-more = true`, page 2 final), so the scaffold
/// really does continue past the probed page — and page 1 still shows up
/// once. The count is filtered on THIS playlist's URL: the capture buffer
/// is process-global, and every other playlist test in this binary
/// probes its own page 1 too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_first_page_hands_off_to_the_scaffold_and_fetches_page_1_once() {
    let ext = fixture_extractor_from(FIXTURE_MANIFEST, EXAMPLE_0_5_2_WASM);
    let logs = captured_logs();
    let out = ext
        .extract_playlist("https://example.com/a-real-playlist", &extraction_ctx())
        .await
        .expect("a real first page hands off to the scaffold");
    let ids: Vec<&str> = out.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        ["1", "2", "3"],
        "both pages' entries resolved, in order"
    );
    let page_fetches = |page: u32| {
        logs.lock()
            .expect("test mutex is never poisoned")
            .iter()
            .filter(|(_, m)| {
                m.contains(&format!(
                    "extract-playlist: fetching page {page} of https://example.com/a-real-playlist"
                ))
            })
            .count()
    };
    assert_eq!(page_fetches(2), 1, "the scaffold fetched page 2 itself");
    let page_1_fetches = page_fetches(1);
    assert_eq!(
        page_1_fetches, 1,
        "page one must be fetched exactly once — the probe's page is reused by the scaffold"
    );
}
