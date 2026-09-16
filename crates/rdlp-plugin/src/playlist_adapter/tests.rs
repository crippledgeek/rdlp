use super::*;
use crate::convert::MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES;
use crate::test_harness::{EMPTY_COMPONENT_WAT, instantiate, wit_body_lines};
use crate::test_support::unit::{
    FIXTURE_MANIFEST, TEST_LOG_TARGET, captured_count_containing, captured_count_on_target,
    captured_entry_containing, captured_logs, fixture_extractor, fixture_extractor_from,
    fixture_manifest_named, test_origin,
};
use crate::test_support::{EXAMPLE_0_5_2_WASM, extraction_ctx};

/// This export's "absent" meaning: a component without `extract-playlist`
/// is `Ok(None)` for the caller to fall back on. The lookup/typecheck/trap
/// mechanism itself is `adapter::tests::by_name_*`.
#[tokio::test]
async fn absent_export_is_ok_none() {
    let (mut store, inst) = instantiate(EMPTY_COMPONENT_WAT).await;
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

/// Drift guards for the hand-written lifts (`test_harness::wit_body_lines`
/// says why): each record/variant's lines in `wit/types.wit` must be
/// exactly these, in this order.
#[test]
fn playlist_entry_lift_mirrors_the_wit_record_field_for_field() {
    assert_eq!(
        wit_body_lines("record", "playlist-entry"),
        [
            "url: string,",
            "id: option<string>,",
            "title: option<string>,",
        ],
        "playlist-entry drifted from the Rust lift"
    );
}

#[test]
fn playlist_page_lift_mirrors_the_wit_record_field_for_field() {
    assert_eq!(
        wit_body_lines("record", "playlist-page"),
        [
            "entries: list<playlist-entry>,",
            "page: u32,",
            "has-more: bool,",
            "playlist-id: option<string>,",
            "playlist-title: option<string>,",
            "total-estimate: option<u64>,",
        ],
        "playlist-page drifted from the Rust lift"
    );
}

#[test]
fn playlist_error_lift_mirrors_the_wit_variant_case_for_case() {
    assert_eq!(
        wit_body_lines("variant", "playlist-error"),
        [
            "unsupported-url(string),",
            "not-found(string),",
            "rate-limited(option<u32>),",
            "network(string),",
            "parse(string),",
            "internal(string),",
        ],
        "playlist-error drifted from the Rust lift"
    );
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
    let page = playlist_page_from_wit(w, 2, &test_origin());
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

/// A page echoing a number other than the one requested is a plugin bug
/// worth seeing in the plugin's log; the host keeps its own count and
/// the page is converted unchanged.
#[test]
fn page_echo_mismatch_is_logged_on_the_plugin_target_and_not_acted_on() {
    let logs = captured_logs();
    let w = WitPlaylistPage {
        entries: Vec::new(),
        page: 7,
        has_more: false,
        playlist_id: None,
        playlist_title: None,
        total_estimate: None,
    };
    let page = playlist_page_from_wit(w, 3, &test_origin());
    assert!(page.entries.is_empty());
    let (target, _) = captured_entry_containing(
        &logs,
        "extract-playlist: plugin test answered page 7 for a request for page 3",
    );
    assert_eq!(target, TEST_LOG_TARGET);
    // A matching echo says nothing.
    let quiet = WitPlaylistPage {
        entries: Vec::new(),
        page: 4,
        has_more: false,
        playlist_id: None,
        playlist_title: None,
        total_estimate: None,
    };
    playlist_page_from_wit(quiet, 4, &test_origin());
    assert_eq!(
        captured_count_containing(&logs, "answered page 4 for a request for page 4"),
        0
    );
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
    let page = playlist_page_from_wit(w, 1, &test_origin());
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
/// through unchanged and must NOT be reported as capped (a
/// `>=`-instead-of-`>` off-by-one would false-positive exactly here).
/// The log buffer is process-global,
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
    let page = playlist_page_from_wit(w, 1, &test_origin());
    assert_eq!(page.entries.len(), MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES);
    let false_positive_cap_message = format!(
        "extract-playlist: plugin test supplied {MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES} playlist entries"
    );
    assert_eq!(
        captured_count_containing(&logs, &false_positive_cap_message),
        0,
        "exactly the bound must not warn (a >= instead of > would false-positive here)"
    );
}

/// Every entry of one batch resolves under the SAME timeout gate — the
/// source's own — with the loop's budget and the entry URL as subject, so
/// `adapter::tests::timeouts_under_one_gate_strike_once_per_gate` is what
/// a batch of timed-out entries does: one strike, not one per entry.
#[test]
fn entry_spec_shares_the_batch_gate() {
    use crate::adapter::TimeoutStrikes;
    use std::time::Duration;

    let ext = fixture_extractor();
    let source = PluginPlaylistSource::new(&ext);
    let entries = [
        PlaylistEntry {
            url: "https://example.com/video/1".into(),
            id: None,
            title: None,
        },
        PlaylistEntry {
            url: "https://example.com/video/2".into(),
            id: None,
            title: None,
        },
    ];
    for entry in &entries {
        let request = ResolveRequest {
            entry,
            budget: Duration::from_secs(31),
        };
        let spec = source.entry_spec(&request);
        assert_eq!(spec.subject_for_errors, entry.url);
        assert_eq!(spec.timeout, Duration::from_secs(31));
        let TimeoutStrikes::OncePer(gate) = spec.timeout_strikes else {
            panic!("an entry's timeout must be gated per batch");
        };
        assert!(
            std::ptr::eq(gate, &raw const source.timeout_struck),
            "every entry must share the source's one gate"
        );
    }
}

// ── `PluginExtractor::extract_playlist` (the probe + fallback) ───────────

/// A plugin whose component never declared `extract-playlist` (the
/// committed 0.5.0 fixture) falls back to the trait default: one
/// `extract` call, wrapped in a one-element `Vec`, with no playlist index
/// stamped — and WITHOUT probing: the export's absence is read off the
/// component type at load, so no `extract-playlist` page is ever
/// requested (the by-name-call line `call_extract_playlist_page` logs
/// on every attempt is absent for this URL).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_export_falls_back_to_single_extract_without_probing() {
    let ext = fixture_extractor();
    let logs = captured_logs();
    let out = ext
        .extract_playlist("https://example.com/video/42", &extraction_ctx())
        .await
        .expect("falls back to the single extract");
    assert_eq!(out.len(), 1);
    let info = out.first().expect("one entry");
    assert_eq!(info.title, "Example Video 42");
    assert_eq!(info.playlist_index, None);
    assert_eq!(
        captured_count_containing(
            &logs,
            "extract-playlist: fetching page 1 of https://example.com/video/42"
        ),
        0,
        "a component without the export is never probed"
    );
}

/// `Config::extract_playlist = false` is the operator saying "treat every
/// URL as a single video": the plugin's `extract-playlist` is never
/// called even though the component exports it, and the URL goes to
/// `extract` — which for the fixture's playlist URL is a domain error,
/// never the three listed entries. The plugin is loaded under its own
/// name so its log target is its own: the page-1 line for this URL is
/// also produced by `real_first_page_hands_off_…` (playlists on), and
/// only the target tells that test's line from a probe this one must
/// never make — so the count on THIS target is asserted to be zero.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_playlist_off_skips_the_probe_and_extracts_the_url_itself() {
    use rdlp_core::ExtractionContext;
    use std::sync::Arc;

    let ext = fixture_extractor_from(&fixture_manifest_named("example-off"), EXAMPLE_0_5_2_WASM);
    let logs = captured_logs();
    let ctx = ExtractionContext {
        config: Arc::new(rdlp_types::Config {
            extract_playlist: false,
            ..Default::default()
        }),
        ..extraction_ctx()
    };
    let out = ext
        .extract_playlist("https://example.com/a-real-playlist", &ctx)
        .await;
    assert!(
        !matches!(&out, Ok(v) if v.len() == 3),
        "the listing must not be driven with playlists off: {out:?}"
    );
    assert_eq!(
        captured_count_on_target(
            &logs,
            "plugin::example-off",
            "extract-playlist: fetching page 1 of https://example.com/a-real-playlist"
        ),
        0,
        "playlists off must not probe extract-playlist"
    );
    // The control for the target filter: the same plugin DID log its
    // fallback `extract` of the URL on that target, so an empty target
    // (a wrong name, a wrong derivation) cannot pass the zero above.
    assert_eq!(
        captured_count_on_target(
            &logs,
            "plugin::example-off",
            "for https://example.com/a-real-playlist under a"
        ),
        1,
        "the fallback extract ran on this plugin's own target"
    );
}

// The four tests below load the committed 0.5.2 fixture
// (`test_support::EXAMPLE_0_5_2_WASM`): a WAT component cannot export
// both `extract-playlist` and the full 0.5.0 `extract` surface without
// reproducing `info-dict`/`extract-error`'s whole shape by hand, so a
// real compiled component is the only way to drive the probe-then-fall-
// back path end to end. What its `extract-playlist` answers per URL is
// pinned in `examples/plugins/example-extractor/src/lib.rs`.

/// A plugin whose `extract-playlist` answers `err(unsupported-url)` for
/// this URL falls back the same way `absent_export_falls_back_to_single_extract_without_probing`
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
/// other `PluginError::Internal`: the domain error is mapped INSIDE the
/// runner's closure, where `counts_as_strike` can see it — mapped after
/// `run_in_fresh_store` returned it would not be (#768). The 0.5.2
/// fixture's `extract-playlist` answers `err(internal(...))` for this URL.
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
///
/// The same run also pins that `Config::playlist_item_timeout` is the
/// budget the plugin runner runs each entry's `extract` under — ONE timer
/// per entry, the runner's — via the runner's per-call budget line. A
/// value above `EXTRACT_TIMEOUT` (30 s) is the discriminating case: a
/// loop that wrapped `InfoExtractor::extract` would leave the runner at
/// 30 s and make anything above it dead. One test rather than two because
/// the fixture has exactly one real listing URL and the page-fetch count
/// above is keyed on it — a second test listing it would double the
/// count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_first_page_hands_off_to_the_scaffold_and_fetches_page_1_once() {
    use rdlp_core::ExtractionContext;
    use std::sync::Arc;

    let ext = fixture_extractor_from(FIXTURE_MANIFEST, EXAMPLE_0_5_2_WASM);
    let logs = captured_logs();
    let ctx = ExtractionContext {
        config: Arc::new(rdlp_types::Config {
            playlist_item_timeout: Some(31),
            ..Default::default()
        }),
        ..extraction_ctx()
    };
    let out = ext
        .extract_playlist("https://example.com/a-real-playlist", &ctx)
        .await
        .expect("a real first page hands off to the scaffold");
    let ids: Vec<&str> = out.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        ["1", "2", "3"],
        "both pages' entries resolved, in order"
    );
    let page_fetches = |page: u32| {
        captured_count_containing(
            &logs,
            &format!(
                "extract-playlist: fetching page {page} of https://example.com/a-real-playlist"
            ),
        )
    };
    assert_eq!(page_fetches(2), 1, "the scaffold fetched page 2 itself");
    let page_1_fetches = page_fetches(1);
    assert_eq!(
        page_1_fetches, 1,
        "page one must be fetched exactly once — the probe's page is reused by the scaffold"
    );
    for id in ["1", "2", "3"] {
        assert_eq!(
            captured_count_containing(
                &logs,
                &format!("for https://example.com/video/{id} under a 31s budget")
            ),
            1,
            "entry {id} resolved under the configured budget, exactly once"
        );
    }
}
