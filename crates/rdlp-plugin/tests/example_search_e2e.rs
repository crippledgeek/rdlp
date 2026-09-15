// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs
)]

//! End-to-end proof of the host side of the 0.5.1 search contract
//! (rdlp#762 slice B, decision D3): `search-filters` is resolved by a
//! manual export lookup rather than by the generated bindings, so a
//! 0.5.1 component answers with its descriptors while a 0.5.0 component —
//! which never declared the export — instantiates and answers `[]`.
//!
//! Builds `examples/plugins/example-extractor` with the production-loadable
//! recipe (plain `cargo build` for `wasm32-unknown-unknown`, then
//! `wasm-tools component new` with no adapter — the host wires no WASI),
//! so the test needs `rustup target add wasm32-unknown-unknown`,
//! `cargo-component` (for `bindings.rs`) and `wasm-tools` on `PATH`.
//!
//! Run with:
//!   cargo test -p rdlp-plugin --test example_search_e2e -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rdlp_core::{RdlpError, SearchExtractor};
use rdlp_extractor::base::common::PAGE_RATE_LIMIT_MS;
use rdlp_plugin::PluginError;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::bindings::rdlp::plugin::types::SearchQuery as WitSearchQuery;
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::prompt::AlwaysApprove;
use rdlp_plugin::search_adapter::PluginSearchExtractor;
use rdlp_plugin::test_support::{
    EXAMPLE_0_5_0_WASM, SignedPluginSpec, extraction_ctx, write_signed_plugin,
};
use rdlp_plugin::trust_store::TrustStore;
use rdlp_types::{SearchFilter, SearchQuery};
use tempfile::TempDir;

fn example_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/plugins/example-extractor")
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
    assert!(status.success(), "{program} {args:?} failed: {status}");
}

/// Build the example with the recipe its crate doc and the 0.5.0 fixture
/// README document; returns the composed component bytes.
///
/// Built once per test binary: the `#[ignore]`d tests run in parallel, and
/// two concurrent `cargo build` + `wasm-tools component new` runs in the
/// same `target/` race on the output file (observed as a spurious failure
/// in one of the two tests).
fn build_example_component() -> &'static [u8] {
    static COMPONENT: OnceLock<Vec<u8>> = OnceLock::new();
    COMPONENT.get_or_init(build_example_component_uncached)
}

fn build_example_component_uncached() -> Vec<u8> {
    let dir = example_dir();
    // `src/bindings.rs` is gitignored generated code; regenerate it so a
    // fresh checkout builds.
    run(&dir, "cargo", &["component", "bindings"]);
    run(
        &dir,
        "cargo",
        &["build", "--release", "--target", "wasm32-unknown-unknown"],
    );
    let core = dir.join("target/wasm32-unknown-unknown/release/example_extractor.wasm");
    let out = dir.join("target/example-extractor-0.5.1.component.wasm");
    run(
        &dir,
        "wasm-tools",
        &[
            "component",
            "new",
            core.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
    );
    std::fs::read(&out).unwrap()
}

/// Sign `wasm` under `wit_version` as a search-capable `example` (the
/// template's `supports_search = true`), discover it through the real
/// loader, and wrap it in a `PluginExtractor` with no host resources (the
/// example declares no capabilities).
fn load_adapter(td: &TempDir, wasm: &[u8], wit_version: &str) -> PluginExtractor {
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    write_signed_plugin(
        &plugins_dir.join("example"),
        &key,
        &SignedPluginSpec {
            wit_version,
            supports_search: true,
            wasm,
            ..SignedPluginSpec::example()
        },
    );
    let engine = Arc::new(Engine::new(EngineConfig::default()).unwrap());
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let prompter = Arc::new(AlwaysApprove);
    let mut loader = Loader::new(engine.as_ref(), &mut trust, prompter);
    let mut outcomes = loader.discover(&plugins_dir);
    assert_eq!(outcomes.len(), 1, "expected exactly one discover outcome");
    let loaded = outcomes
        .remove(0)
        .unwrap_or_else(|(path, err)| panic!("discover failed for {path:?}: {err:?}"));
    PluginExtractor::new(loaded, engine, HostResources::default()).expect("adapter")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "builds examples/plugins/example-extractor via cargo + wasm-tools"]
async fn search_filters_and_search_round_trip_through_the_runner() {
    let wasm = build_example_component();

    // (a) present-export path: the 0.5.1 example declares exactly one
    // descriptor; label == value per the WIT record contract.
    let td = TempDir::new().unwrap();
    let adapter = load_adapter(&td, wasm, "0.5.1");
    let filters = adapter.call_search_filters().await.expect("search-filters");
    assert_eq!(
        filters.len(),
        1,
        "example declares one descriptor: {filters:?}"
    );
    let f = &filters[0];
    assert_eq!(f.key, "ordering");
    assert_eq!(f.display_name, "Ordering");
    assert_eq!(f.default.as_deref(), Some("newest"));
    let values: Vec<(&str, &str)> = f
        .allowed_values
        .iter()
        .map(|v| (v.value.as_str(), v.label.as_str()))
        .collect();
    assert_eq!(values, vec![("newest", "newest"), ("views", "views")]);

    // (c) `search` on the example returns its canned page; the page
    // number echoes the query. Page 1 of the example is two results with
    // more to come (page 2 repeats them, for the host's duplicate-page rule).
    let page = adapter
        .call_search(WitSearchQuery {
            query: "kittens".into(),
            page: Some(1),
            filters: vec![],
        })
        .await
        .expect("search");
    assert_eq!(page.page, 1);
    let urls: Vec<&str> = page.results.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls,
        ["https://example.com/video/1", "https://example.com/video/2"],
        "canned page 1: {page:?}"
    );
    assert!(page.has_more);

    // Neither call is a strike.
    assert_eq!(adapter.test_trap_count(), 0);
}

fn host_query(
    filters: &[(&str, &str)],
    max_results: Option<usize>,
    page: Option<u32>,
) -> SearchQuery {
    SearchQuery {
        query: "kittens".into(),
        filters: filters
            .iter()
            .map(|(k, v)| SearchFilter {
                key: (*k).into(),
                value: (*v).into(),
            })
            .collect(),
        max_results,
        page,
    }
}

fn extraction_message(err: RdlpError) -> String {
    match err {
        RdlpError::Extraction { message, url } => {
            assert_eq!(url, None, "a search error carries no URL");
            message
        }
        other => panic!("expected Extraction, got {other:?}"),
    }
}

/// The example's canned `search`: page 1 is videos 1–2 with `has_more`,
/// page 2 is the SAME two videos. Everything asserted here is the host
/// scaffold's doing — the plugin never sees more than one page request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "builds examples/plugins/example-extractor via cargo + wasm-tools"]
async fn plugin_search_extractor_drives_the_example_through_the_host_scaffold() {
    let wasm = build_example_component();
    let td = TempDir::new().unwrap();
    let adapter = Arc::new(load_adapter(&td, wasm, "0.5.1"));
    let site = PluginSearchExtractor::new(Arc::clone(&adapter));
    let ctx = extraction_ctx();
    let pacing = Duration::from_millis(PAGE_RATE_LIMIT_MS);

    // Descriptors reach the SearchExtractor surface through the cache.
    let filters = site.supported_filters().await;
    assert_eq!(filters.len(), 1, "{filters:?}");
    assert_eq!(filters[0].key, "ordering");

    // All pages: page 2 repeats page 1, so duplicate-page termination stops
    // the loop at two results — and the one pacing sleep between the two
    // page fetches is observable as elapsed time.
    let started = Instant::now();
    let all = site
        .search(&host_query(&[], None, None), &ctx)
        .await
        .expect("search");
    let elapsed = started.elapsed();
    let urls: Vec<&str> = all.iter().map(|r| r.video_url.as_str()).collect();
    assert_eq!(
        urls,
        ["https://example.com/video/1", "https://example.com/video/2"]
    );
    assert!(
        elapsed >= pacing,
        "two page fetches must be paced by PAGE_RATE_LIMIT_MS; took {elapsed:?}"
    );

    // `max_results: Some(1)` truncates within page 1 and never fetches
    // page 2. Under `ordering=views` the example's page 2 is an `internal`
    // error — a counted fault — so an over-fetch would show up in the
    // trap count (the store is fresh per call, so the plugin itself cannot
    // count). `later_page_error_returns_partial_results_and_counts_a_strike`
    // below proves that page really does count, so `0` here is not vacuous.
    let one = site
        .search(&host_query(&[("ordering", "views")], Some(1), None), &ctx)
        .await
        .expect("search");
    assert_eq!(adapter.test_trap_count(), 0, "page 2 must not be fetched");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].video_url, "https://example.com/video/1");

    // Single page: the scaffold echoes the page it asked for, and page 2
    // of the example reports no further page.
    let page2 = site
        .search_page(&host_query(&[], None, Some(2)), &ctx)
        .await
        .expect("search_page");
    assert_eq!(page2.page, 2);
    assert_eq!(page2.results.len(), 2);
    assert!(!page2.has_more);
    assert_eq!(page2.total_estimate, Some(2));

    // Filters are validated against the plugin's descriptors BEFORE a page
    // is fetched, in the shared Family-1 wording with the manifest's name.
    // Each case uses a wrapper nothing has primed, so it is `search` /
    // `search_page` ITSELF that must fetch the descriptors first: without
    // that, `ordering` would be an unknown key rather than a bad value.
    let err = PluginSearchExtractor::new(Arc::clone(&adapter))
        .search(&host_query(&[("ordering", "bogus")], None, None), &ctx)
        .await
        .expect_err("out-of-set value");
    assert_eq!(
        extraction_message(err),
        "Invalid value 'bogus' for filter 'ordering'. Allowed: newest, views"
    );
    let err = PluginSearchExtractor::new(Arc::clone(&adapter))
        .search_page(&host_query(&[("ordering", "bogus")], None, None), &ctx)
        .await
        .expect_err("out-of-set value");
    assert_eq!(
        extraction_message(err),
        "Invalid value 'bogus' for filter 'ordering'. Allowed: newest, views"
    );
    let err = site
        .search_page(&host_query(&[("foo", "x")], None, None), &ctx)
        .await
        .expect_err("unknown key");
    assert_eq!(
        extraction_message(err),
        "Unknown filter 'foo' for example. Available: ordering"
    );

    assert_eq!(adapter.test_trap_count(), 0);
}

/// The scaffold's later-page asymmetry through a plugin: under
/// `ordering=views` page 1 succeeds and page 2 fails with `internal`, so
/// the operator gets page 1's results and the plugin is charged one
/// strike. Its own adapter, so the count is attributable to this run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "builds examples/plugins/example-extractor via cargo + wasm-tools"]
async fn later_page_error_returns_partial_results_and_counts_a_strike() {
    let wasm = build_example_component();
    let td = TempDir::new().unwrap();
    let adapter = Arc::new(load_adapter(&td, wasm, "0.5.1"));
    let site = PluginSearchExtractor::new(Arc::clone(&adapter));
    let ctx = extraction_ctx();

    let partial = site
        .search(&host_query(&[("ordering", "views")], None, None), &ctx)
        .await
        .expect("a later-page failure yields the pages gathered so far");
    let urls: Vec<&str> = partial.iter().map(|r| r.video_url.as_str()).collect();
    assert_eq!(
        urls,
        ["https://example.com/video/1", "https://example.com/video/2"]
    );
    assert_eq!(
        adapter.test_trap_count(),
        1,
        "page 2's internal error is a strike"
    );
}

/// Not `#[ignore]`d: needs only the committed fixture, so the default gate
/// proves the absent-export path every run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_5_0_component_without_the_export_answers_no_filters() {
    // (b) absent-export path: the manual lookup — not the bindings —
    // resolves `search-filters`, so a component that never declared it
    // still instantiates and answers `[]` instead of failing at
    // instantiate with `no function export ... found`.
    let td = TempDir::new().unwrap();
    let adapter = load_adapter(&td, EXAMPLE_0_5_0_WASM, "0.5.0");
    let filters = adapter
        .call_search_filters()
        .await
        .expect("absent export is Ok");
    assert!(filters.is_empty(), "got {filters:?}");

    // The 0.5.0 example's `search` returns `unsupported` — a domain
    // outcome, not a strike.
    let err = adapter
        .call_search(WitSearchQuery {
            query: "kittens".into(),
            page: None,
            filters: vec![],
        })
        .await
        .expect_err("0.5.0 example does not implement search");
    assert!(
        matches!(err, PluginError::SearchUnsupported { ref plugin } if plugin == "example"),
        "got {err:?}"
    );
    assert_eq!(adapter.test_trap_count(), 0);
}

/// Not `#[ignore]`d: the committed 0.5.0 fixture through the full
/// `SearchExtractor` surface. Its `search` answers `unsupported`, which
/// the scaffold's first-page rule propagates verbatim — the operator sees
/// WHY there are no results, and it is not a strike.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_5_0_component_reports_unsupported_search_to_the_operator() {
    let td = TempDir::new().unwrap();
    let adapter = Arc::new(load_adapter(&td, EXAMPLE_0_5_0_WASM, "0.5.0"));
    let site = PluginSearchExtractor::new(Arc::clone(&adapter));
    let ctx = extraction_ctx();

    assert_eq!(SearchExtractor::name(&site), "example");
    assert!(site.is_plugin());
    assert_eq!(site.search_priority(), 150);
    assert!(!site.overrides_builtin());
    assert!(site.supported_filters().await.is_empty());

    let err = site
        .search(&host_query(&[], None, None), &ctx)
        .await
        .expect_err("0.5.0 example does not implement search");
    assert_eq!(
        extraction_message(err),
        "plugin 'example' does not support search"
    );

    // With no descriptors, any filter is rejected before the wasm call.
    let err = site
        .search_page(&host_query(&[("ordering", "newest")], None, None), &ctx)
        .await
        .expect_err("no descriptors, so no filter is known");
    assert_eq!(
        extraction_message(err),
        "Unknown filter 'ordering' for example. Available: "
    );

    assert_eq!(adapter.test_trap_count(), 0);
}
