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

#![allow(clippy::disallowed_methods)] // test fixture I/O is allowed

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rdlp_core::{ExtractionContext, InfoExtractor};
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::BoaJsEngine;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::manifest::canonical_bytes;
use rdlp_plugin::prompt::AlwaysApprove;
use rdlp_plugin::trust_store::TrustStore;
use rdlp_types::Config;
use tempfile::TempDir;

const WASM_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/plugins/ytdlp-hello-world/out/plugin.wasm"
);

/// Inline copy of `tests/loader.rs::write_signed_plugin`, adapted to take a
/// pre-built wasm payload (instead of a WAT stub) and a richer capability set.
fn write_signed_plugin(dir: &Path, name: &str, key: &SigningKey, wasm: &[u8], capabilities: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("plugin.wasm"), wasm).unwrap();

    let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    let cap_str = capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml_placeholder = format!(
        r#"
name = "{name}"
version = "0.1.0"
wit_version = "0.1.0"
matches = ["https://example.com/*"]
priority = 150
claims_override = []
capabilities = [{cap_str}]

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#,
    );

    let m = rdlp_plugin::manifest::parse_manifest_str(&toml_placeholder).unwrap();
    let mut buf = canonical_bytes(&m);
    buf.extend_from_slice(wasm);
    let sig = key.sign(&buf);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let final_toml = toml_placeholder.replace("PLACEHOLDER", &sig_b64);
    std::fs::write(dir.join("plugin.toml"), final_toml).unwrap();
}

fn make_extraction_ctx() -> ExtractionContext {
    let http = Arc::new(HttpClientFactory::default().build());
    let js = Arc::new(BoaJsEngine::new());
    let cookies = Arc::new(rdlp_cookies::SimpleCookieJar::new());
    let cfg = Arc::new(Config::default());
    ExtractionContext::new(http, js, cookies, cfg)
}

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
        "hello-world",
        &key,
        &wasm,
        &["fetch", "cookie-jar", "js-eval", "html-select", "log", "store-kv"],
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

/// End-to-end extract dispatch through the bumped `StoreLimits`.
///
/// Originally this test was a negative assertion ("trap on instance count too
/// high at 2") because `PluginStoreData::new` pinned
/// `StoreLimitsBuilder::instances(1)` and a componentize-py CPython component
/// instantiates as ~5–6 core sub-components. Task 4 bumped the host limits
/// (see `crates/rdlp-plugin/src/instance.rs`) so the plugin now runs to
/// completion. The test asserts on real `InfoDict` fields produced by the
/// `examples/plugins/ytdlp-hello-world` plugin. The remaining `wasi:cli`
/// import gap is still worked around in `build.sh` via
/// `componentize-py --stub-wasi`.
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
        "hello-world",
        &key,
        &wasm,
        // componentize-py emits IMPORTS for every interface in the WIT world,
        // so the host must link all six. The Manifest still gates *use*: if the
        // plugin calls a capability whose context isn't populated (see
        // populate_capability_contexts), the host returns "denied" at runtime.
        // Phase 1 of the plugin system documents this trade-off in
        // crates/rdlp-plugin/src/lib.rs § "Known limitations".
        &["fetch", "cookie-jar", "js-eval", "html-select", "log", "store-kv"],
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
    };
    let adapter = PluginExtractor::new(loaded, engine.clone(), host_resources)
        .expect("adapter construction must succeed");

    let ctx = make_extraction_ctx();

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
    assert_eq!(info.formats.len(), 1, "expected 1 format; got {:?}", info.formats);
    assert_eq!(
        info.formats[0].url, "https://example.com/foo",
        "format url mismatch: {:?}",
        info.formats[0]
    );
}
