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

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{SignedPluginSpec, write_signed_plugin};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rdlp_plugin::PluginError;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::bindings::rdlp::plugin::types::SearchQuery as WitSearchQuery;
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::prompt::AlwaysApprove;
use rdlp_plugin::trust_store::TrustStore;
use tempfile::TempDir;

/// The Task 1 fixture: built against `rdlp:plugin@0.5.0`, before
/// `search-filters` existed — the absent-export path.
const FIXTURE_0_5_0: &str = "tests/fixtures/example-extractor-0.5.0/plugin.wasm";

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
fn build_example_component() -> Vec<u8> {
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

/// Sign `wasm` under `wit_version`, discover it through the real loader,
/// and wrap it in a `PluginExtractor` with no host resources (the example
/// declares no capabilities).
fn load_adapter(td: &TempDir, wasm: &[u8], wit_version: &str) -> PluginExtractor {
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    write_signed_plugin(
        &plugins_dir.join("example"),
        &key,
        &SignedPluginSpec {
            name: "example",
            version: "0.1.0",
            wit_version,
            matches: &["https://example.com/*"],
            priority: 150,
            claims_override: &[],
            capabilities: &[],
            wasm,
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
    let adapter = load_adapter(&td, &wasm, "0.5.1");
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
    // number echoes the query.
    let page = adapter
        .call_search(WitSearchQuery {
            query: "kittens".into(),
            page: Some(1),
            filters: vec![],
        })
        .await
        .expect("search");
    assert_eq!(page.page, 1);
    assert_eq!(
        page.results.len(),
        1,
        "canned page has one result: {page:?}"
    );
    assert_eq!(page.results[0].url, "https://example.com/video/1");
    assert!(!page.has_more);

    // Neither call is a strike.
    assert_eq!(adapter.test_trap_count(), 0);
}

/// Not `#[ignore]`d: needs only the committed fixture, so the default gate
/// proves the absent-export path every run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_5_0_component_without_the_export_answers_no_filters() {
    // (b) absent-export path: the manual lookup — not the bindings —
    // resolves `search-filters`, so a component that never declared it
    // still instantiates and answers `[]` instead of failing at
    // instantiate with `no function export ... found`.
    let wasm = std::fs::read(FIXTURE_0_5_0).unwrap();
    let td = TempDir::new().unwrap();
    let adapter = load_adapter(&td, &wasm, "0.5.0");
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
