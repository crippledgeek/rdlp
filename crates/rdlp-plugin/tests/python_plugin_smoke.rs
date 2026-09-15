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

//! Slice-1 spike: verify a componentize-py-built Python plugin loads,
//! instantiates, and dispatches `extract` through the existing host. Measures
//! cold-start (load+sign+discover) and per-call extract latency.
//!
//! Reuses the Ed25519-test-signing pattern from `tests/loader.rs` (inlined
//! here — those helpers are private to that test crate).
//!
//! Run with:
//!   examples/plugins/ytdlp-hello-world/build.sh   # produces out/plugin.wasm
//!   cargo test -p rdlp-plugin --test python_plugin_smoke -- --ignored --nocapture

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use common::{SignedPluginSpec, extraction_ctx, write_signed_plugin};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rdlp_core::InfoExtractor;
use rdlp_http::HttpClientFactory;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::prompt::AlwaysApprove;
use rdlp_plugin::trust_store::TrustStore;
use tempfile::TempDir;

const WASM_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/plugins/ytdlp-hello-world/out/plugin.wasm"
);

/// Measures cold-start (load+sign+discover) — the load-bearing deliverable for
/// Task 2. Does not call `extract`; that path hits Phase 1 host limits
/// documented inline in the second test below.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires examples/plugins/ytdlp-hello-world/build.sh to have run"]
async fn python_hello_world_loads_and_signs() {
    let wasm = read_artefact();
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);

    let load_start = Instant::now();
    write_signed_plugin(
        &plugins_dir.join("hello-world"),
        &key,
        &SignedPluginSpec {
            name: "hello-world",
            version: "0.1.0",
            wit_version: "0.5.0",
            matches: &["https://example.com/*"],
            priority: 150,
            claims_override: &[],
            supports_extract: true,
            supports_search: false,
            capabilities: &[
                "fetch",
                "cookie-jar",
                "js-eval",
                "html-select",
                "log",
                "store-kv",
            ],
            wasm: &wasm,
        },
    );
    let engine = Arc::new(Engine::new(EngineConfig::default()).unwrap());
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let prompter = Arc::new(AlwaysApprove);
    let mut loader = Loader::new(engine.as_ref(), &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);
    let load_ms = load_start.elapsed().as_millis();
    eprintln!("[measure] load+sign+discover: {load_ms} ms");

    assert_eq!(outcomes.len(), 1, "expected one plugin outcome");
    let loaded = match outcomes.into_iter().next().unwrap() {
        Ok(p) => p,
        Err((path, err)) => panic!("discover failed for {}: {:?}", path.display(), err),
    };
    assert_eq!(loaded.manifest.name, "hello-world");
    assert_eq!(loaded.manifest.priority, 150);

    // Adapter construction is part of cold-start too — wires the linker.
    let adapter_start = Instant::now();
    let host_resources = HostResources {
        fetch_client: Some(HttpClientFactory::default().build()),
        cookie_jar: None,
        kv_db: None,
        fetch_fixtures: None,
    };
    let _adapter = PluginExtractor::new(loaded, engine.clone(), host_resources)
        .expect("adapter construction must succeed");
    let adapter_ms = adapter_start.elapsed().as_millis();
    eprintln!("[measure] adapter linker wire: {adapter_ms} ms");
}

fn read_artefact() -> Vec<u8> {
    let wasm_path = PathBuf::from(WASM_PATH);
    if !wasm_path.exists() {
        panic!(
            "Plugin artefact not found at {}. \
             Run examples/plugins/ytdlp-hello-world/build.sh first.",
            wasm_path.display()
        );
    }
    let wasm = std::fs::read(&wasm_path).unwrap();
    eprintln!("[measure] wasm size: {} bytes", wasm.len());
    wasm
}

/// End-to-end extract dispatch. Asserts on real `InfoDict` fields produced
/// by the `examples/plugins/ytdlp-hello-world` plugin. The remaining
/// `wasi:cli` import gap is worked around in `build.sh` via
/// `componentize-py --stub-wasi` (Phase-1 limitation; revisit when the host
/// gains WASI 0.2 surface).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires examples/plugins/ytdlp-hello-world/build.sh to have run"]
async fn python_hello_world_extract_succeeds() {
    let wasm = read_artefact();

    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);

    // ── cold start: write+sign manifest, compile component, run discover ────
    let load_start = Instant::now();
    write_signed_plugin(
        &plugins_dir.join("hello-world"),
        &key,
        &SignedPluginSpec {
            name: "hello-world",
            version: "0.1.0",
            wit_version: "0.5.0",
            matches: &["https://example.com/*"],
            priority: 150,
            claims_override: &[],
            supports_extract: true,
            supports_search: false,
            // componentize-py emits IMPORTS for every interface in the WIT world,
            // so the host must link all six. The Manifest still gates *use*: if the
            // plugin calls a capability whose context isn't populated (see
            // populate_capability_contexts), the host returns "denied" at runtime.
            // Phase 1 of the plugin system documents this trade-off in
            // crates/rdlp-plugin/src/lib.rs § "Known limitations".
            capabilities: &[
                "fetch",
                "cookie-jar",
                "js-eval",
                "html-select",
                "log",
                "store-kv",
            ],
            wasm: &wasm,
        },
    );

    let engine = Arc::new(Engine::new(EngineConfig::default()).unwrap());
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let prompter = Arc::new(AlwaysApprove);
    let mut loader = Loader::new(engine.as_ref(), &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);
    let load_ms = load_start.elapsed().as_millis();
    eprintln!("[measure] load+sign+discover: {load_ms} ms");

    assert_eq!(outcomes.len(), 1, "expected one plugin outcome");
    let loaded = match outcomes.into_iter().next().unwrap() {
        Ok(p) => p,
        Err((path, err)) => panic!("discover failed for {}: {:?}", path.display(), err),
    };
    assert_eq!(loaded.manifest.name, "hello-world");
    assert_eq!(loaded.manifest.priority, 150);

    // ── per-call: build adapter + dispatch extract ─────────────────────────
    let host_resources = HostResources {
        fetch_client: Some(HttpClientFactory::default().build()),
        cookie_jar: None,
        kv_db: None,
        fetch_fixtures: None,
    };
    let adapter = PluginExtractor::new(loaded, engine.clone(), host_resources)
        .expect("adapter construction must succeed");

    let ctx = extraction_ctx();

    let extract_start = Instant::now();
    let result = adapter.extract("https://example.com/foo", &ctx).await;
    let extract_ms = extract_start.elapsed().as_millis();
    eprintln!("[measure] extract dispatch: {extract_ms} ms");

    let info = match result {
        Ok(info) => info,
        Err(err) => panic!("extract returned Err: {err}"),
    };

    assert_eq!(info.id, "hello-1", "id mismatch: {info:?}");
    assert!(
        info.title.contains("Hello"),
        "title should contain \"Hello\"; got {:?}",
        info.title
    );
    assert_eq!(
        info.formats.len(),
        1,
        "expected 1 format; got {:?}",
        info.formats
    );
    assert_eq!(
        info.formats[0].url, "https://example.com/foo",
        "format url mismatch: {:?}",
        info.formats[0]
    );
}
