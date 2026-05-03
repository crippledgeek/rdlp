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
// Lints suppressed for test code — panicking on unexpected errors is intentional here.

//! MPD golden test — proves the `extract-mpd` host helper round-trips
//! correctly through Python → componentize-py → wasmtime → Rust host.
//!
//! What this exercises end-to-end:
//!
//! - `rdlp plugin build-from-ytdlp` compiling a minimal single-class
//!   InfoExtractor (`MpdGoldenIE`) that calls
//!   `_extract_mpd_formats_and_subtitles`.
//! - The `host:fetch` fixture-replay harness intercepting two URLs:
//!   the page HTML embedding a `data-mpd=` attribute and the MPD body
//!   from `crates/rdlp-downloader/tests/fixtures/dash/segment_template.mpd`.
//! - The WIT `extract-mpd` helper returning a `(formats, subtitles)` tuple
//!   that crosses the WASM boundary and is marshalled into `InfoDict`.
//! - Assertions that at least one `DownloadProtocol::HttpDashSegments`
//!   format is present, along with separate video-only and audio-only
//!   formats (matching the two `AdaptationSet` entries in the fixture MPD).
//!
//! Run with:
//!   cd tools/ytdlp-compat && python3 -m venv .venv && \
//!     .venv/bin/pip install -r requirements-dev.txt
//!   cargo test -p rdlp-plugin --test mpd_golden -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rdlp_core::{ExtractionContext, InfoExtractor};
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::BoaJsEngine;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::manifest::canonical_bytes;
use rdlp_plugin::prompt::AlwaysApprove;
use rdlp_plugin::trust_store::TrustStore;
use rdlp_types::Config;
use tempfile::TempDir;

const TEST_URL: &str = "https://mpd-test.example.com/abc123";
const EXPECTED_VIDEO_ID: &str = "abc123";

/// Inline page HTML — contains the `data-mpd=` attribute the plugin
/// extracts via `_search_regex`.
const PAGE_HTML: &[u8] =
    b"<html><body data-mpd=\"https://mpd.example.com/v.mpd\"></body></html>";

/// Real SegmentTemplate MPD fixture from the downloader test suite.
/// Contains one video `AdaptationSet` (avc1) and one audio `AdaptationSet`
/// (mp4a) — the assertions verify both sides are returned.
const MPD_BODY: &[u8] = include_bytes!(
    "../../rdlp-downloader/tests/fixtures/dash/segment_template.mpd"
);

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Build the mpd-golden plugin via `cargo run -p rdlp-cli -- plugin
/// build-from-ytdlp ...`. Returns the path to the produced `plugin.wasm`.
///
/// CI may set `RDLP_TEST_USE_CACHED_WASM=1` after restoring a `plugin.wasm`
/// from the actions/cache step keyed on plugin source + componentize-py
/// version + WIT contract. When the env var is present AND the wasm exists,
/// the rebuild is skipped — saving ~30s of componentize-py per plugin.
/// Local runs never set this; the rebuild path remains the default for
/// correctness.
fn build_mpd_golden_plugin() -> PathBuf {
    let root = workspace_root();
    let py = root.join("examples/plugins/mpd-golden/mpd_golden.py");
    assert!(py.exists(), "source missing: {py:?}");

    // build-from-ytdlp normalises filenames to kebab-case plugin names
    // and emits into <output_dir>/<name>/. For mpd_golden.py the output
    // dir ends up at examples/plugins/mpd-golden/mpd-golden/plugin.wasm.
    let plugin_dir = py.parent().unwrap().join("mpd-golden");
    let wasm = plugin_dir.join("plugin.wasm");

    if std::env::var("RDLP_TEST_USE_CACHED_WASM").is_ok() && wasm.exists() {
        eprintln!(
            "[cache] mpd-golden: reusing cached plugin.wasm ({} bytes) — skipping build",
            std::fs::metadata(&wasm).unwrap().len()
        );
        return wasm;
    }

    // Clean any prior build so a stale wasm doesn't mask a build failure.
    let _ = std::fs::remove_dir_all(&plugin_dir);

    let status = std::process::Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "rdlp-cli",
            "--",
            "plugin",
            "build-from-ytdlp",
        ])
        .arg(&py)
        .current_dir(&root)
        .status()
        .expect("invoke rdlp build-from-ytdlp");
    assert!(status.success(), "build-from-ytdlp failed");

    assert!(wasm.exists(), "wasm missing at {wasm:?}");
    wasm
}

/// Inline copy of the test signing helper (matches
/// `python_plugin_smoke.rs::write_signed_plugin`). Writes a signed
/// `plugin.toml` next to a `plugin.wasm` so `Loader::discover` accepts it.
fn write_signed_plugin(
    dir: &Path,
    name: &str,
    key: &SigningKey,
    wasm: &[u8],
    matches: &[&str],
    capabilities: &[&str],
) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("plugin.wasm"), wasm).unwrap();

    let pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    let cap_str = capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let match_str = matches
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml_placeholder = format!(
        r#"
name = "{name}"
version = "0.1.0"
wit_version = "0.1.0"
matches = [{match_str}]
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

/// Build the fixture map: page HTML at the test URL and the MPD body at
/// the URL embedded in `data-mpd=`.
fn build_mpd_fixtures() -> FetchFixtures {
    FetchFixtures::new()
        .with(TEST_URL, FixtureResponse::ok(PAGE_HTML.to_vec()))
        .with(
            "https://mpd.example.com/v.mpd",
            FixtureResponse::ok(MPD_BODY.to_vec()),
        )
}

fn make_extraction_ctx() -> ExtractionContext {
    let http = Arc::new(HttpClientFactory::default().build());
    let js = Arc::new(BoaJsEngine::new());
    let cookies = Arc::new(rdlp_cookies::SimpleCookieJar::new());
    let cfg = Arc::new(Config::default());
    ExtractionContext::new(http, js, cookies, cfg)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "slow: builds ~35MB mpd-golden wasm via componentize-py (~30s) and \
            requires tools/ytdlp-compat/.venv populated"]
async fn mpd_golden_extract_returns_formats_via_fixture() {
    // ── build the plugin ───────────────────────────────────────────────────
    let wasm_path = build_mpd_golden_plugin();
    let wasm = std::fs::read(&wasm_path).unwrap();
    eprintln!("[measure] mpd-golden.wasm size: {} bytes", wasm.len());

    // ── sign + discover ────────────────────────────────────────────────────
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    write_signed_plugin(
        &plugins_dir.join("mpd-golden"),
        "mpd-golden",
        &key,
        &wasm,
        &["https://mpd-test.example.com/*"],
        // componentize-py emits IMPORTS for every interface in the WIT
        // world (Phase-1 limitation). The manifest MUST declare all six
        // caps so the linker wires every import the wasm references — the
        // host still gates *use* at runtime via populate_capability_contexts.
        &[
            "fetch",
            "cookie-jar",
            "js-eval",
            "html-select",
            "log",
            "store-kv",
        ],
    );

    let engine = Arc::new(Engine::new(EngineConfig::default()).unwrap());
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let prompter = Arc::new(AlwaysApprove);
    let mut loader = Loader::new(engine.as_ref(), &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);
    assert_eq!(outcomes.len(), 1);
    let loaded = match outcomes.into_iter().next().unwrap() {
        Ok(p) => p,
        Err((path, err)) => panic!("discover failed for {}: {:?}", path.display(), err),
    };
    assert_eq!(loaded.manifest.name, "mpd-golden");

    // ── build adapter with fixtures injected ───────────────────────────────
    let fixtures = Arc::new(build_mpd_fixtures());
    eprintln!("[measure] {} fetch fixtures registered", fixtures.len());

    let host_resources = HostResources {
        fetch_client: Some(HttpClientFactory::default().build()),
        cookie_jar: None,
        kv_db: None,
        fetch_fixtures: Some(fixtures),
    };
    let adapter = PluginExtractor::new(loaded, engine.clone(), host_resources)
        .expect("adapter construction must succeed");

    // ── dispatch + assert ──────────────────────────────────────────────────
    let ctx = make_extraction_ctx();
    let info = match adapter.extract(TEST_URL, &ctx).await {
        Ok(info) => info,
        Err(err) => panic!("extract returned Err: {err}"),
    };

    use rdlp_types::DownloadProtocol;

    assert_eq!(info.id, EXPECTED_VIDEO_ID, "id mismatch");
    assert!(
        !info.formats.is_empty(),
        "expected non-empty formats list; got {info:?}",
    );
    // The fixture MPD carries one video and one audio AdaptationSet;
    // every Representation becomes a Format with HttpDashSegments protocol.
    assert!(
        info.formats
            .iter()
            .any(|f| matches!(f.protocol, DownloadProtocol::HttpDashSegments)),
        "expected at least one DASH format; got {:?}",
        info.formats,
    );
    // Segment-template MPD has one video-only and one audio-only AdaptationSet.
    let has_video_only = info
        .formats
        .iter()
        .any(|f| f.vcodec.is_present() && !f.acodec.is_present());
    let has_audio_only = info
        .formats
        .iter()
        .any(|f| f.acodec.is_present() && !f.vcodec.is_present());
    assert!(
        has_video_only,
        "expected at least one video-only format; got {:?}",
        info.formats,
    );
    assert!(
        has_audio_only,
        "expected at least one audio-only format; got {:?}",
        info.formats,
    );
}
