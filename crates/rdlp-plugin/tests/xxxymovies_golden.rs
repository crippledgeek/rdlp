//! Slice-2 second plugin: xxxymovies — proves end-to-end plugin
//! extraction with a SECOND real upstream yt-dlp source independent of
//! SVT. Distinct from `svt_golden.rs` in that:
//!
//! - Single-class plugin (XXXYMoviesIE only) — no multi-class dispatch.
//! - HTML-scraping path (no Next.js / urqlState) — exercises
//!   `_search_regex`, `_html_search_regex`, `_html_search_meta`,
//!   `_rta_search`, and the new `clean_html` / `parse_duration`
//!   helpers ported in the same sprint.
//! - Direct `url` field instead of `formats[]` — covers the alternate
//!   InfoDict shape supported by `_validate_id`.
//!
//! Together with `svt_golden`, this gives Slice 2 two independent
//! end-to-end-green plugin ports — proves the build/sign/dispatch/
//! extract pipeline works for distinct extractor archetypes.

#![allow(clippy::disallowed_methods)] // test fixture I/O is allowed

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

// Upstream `_TEST` URL from /tmp/ytdlp-slice2/yt_dlp/extractor/xxxymovies.py:11.
// Live site issues a 301 to a slug-renamed path; the extractor still
// matches the original via `_VALID_URL` regex which captures the numeric
// ID portion only.
const TEST_URL: &str = "http://xxxymovies.com/videos/138669/ecstatic-orgasm-sofcore/";
const FOLLOWED_URL: &str =
    "https://xxxymovies.com/videos/138669/ecstatic-orgasm-sofcore-with-sunny-leone/";
const EXPECTED_VIDEO_ID: &str = "138669";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xxxymovies")
        .join(name)
}

fn build_xxxymovies_plugin() -> PathBuf {
    let root = workspace_root();
    let py = root.join("examples/plugins/xxxymovies/xxxymovies.py");
    assert!(py.exists(), "source missing: {py:?}");

    let plugin_dir = py.parent().unwrap().join("xxxymovies");
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

    let wasm = plugin_dir.join("plugin.wasm");
    assert!(wasm.exists(), "wasm missing at {wasm:?}");
    wasm
}

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

/// Inject the captured page HTML for both the original-shape URL the
/// extractor passes to `_download_webpage` and any redirected variant.
/// xxxymovies's HTML is self-contained — the extractor does NOT recurse
/// into separate API/manifest fetches, so a single fixture suffices.
fn build_xxxymovies_fixtures() -> FetchFixtures {
    let page = std::fs::read(fixture_path("page.html")).unwrap();
    FetchFixtures::new()
        .with(TEST_URL, FixtureResponse::ok(page.clone()))
        .with(FOLLOWED_URL, FixtureResponse::ok(page))
}

fn make_extraction_ctx() -> ExtractionContext {
    let http = Arc::new(HttpClientFactory::default().build());
    let js = Arc::new(BoaJsEngine::new());
    let cookies = Arc::new(rdlp_cookies::SimpleCookieJar::new());
    let cfg = Arc::new(Config::default());
    ExtractionContext::new(http, js, cookies, cfg)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "slow: builds ~35MB xxxymovies wasm via componentize-py (~30s) and \
            requires tools/ytdlp-compat/.venv populated"]
async fn xxxymovies_extract_returns_complete_info_dict() {
    let wasm_path = build_xxxymovies_plugin();
    let wasm = std::fs::read(&wasm_path).unwrap();
    eprintln!("[measure] xxxymovies.wasm size: {} bytes", wasm.len());

    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    write_signed_plugin(
        &plugins_dir.join("xxxymovies"),
        "xxxymovies",
        &key,
        &wasm,
        &["https://*.xxxymovies.com/*", "https://xxxymovies.com/*"],
        // componentize-py emits IMPORTS for every interface in the WIT
        // world; the manifest must declare all six caps so the linker
        // wires every import the wasm references.
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
    assert_eq!(loaded.manifest.name, "xxxymovies");

    let host_resources = HostResources {
        fetch_client: Some(HttpClientFactory::default().build()),
        cookie_jar: None,
        kv_db: None,
        fetch_fixtures: Some(Arc::new(build_xxxymovies_fixtures())),
    };
    let adapter = PluginExtractor::new(loaded, engine.clone(), host_resources)
        .expect("adapter construction must succeed");

    let ctx = make_extraction_ctx();
    let info = match adapter.extract(TEST_URL, &ctx).await {
        Ok(info) => info,
        Err(err) => panic!("extract returned Err: {err}"),
    };

    // Pin the load-bearing fields the upstream _TEST asserts
    // (xxxymovies.py:13-24). The extractor returns `url` instead of
    // `formats[]`, so we unwrap the single-format shape on the rdlp
    // side (info.formats[0]).
    assert_eq!(info.id, EXPECTED_VIDEO_ID, "id mismatch");
    assert_eq!(info.title, "Ecstatic Orgasm Sofcore with Sunny Leone");
    assert!(
        !info.formats.is_empty(),
        "expected at least one format with the direct video_url",
    );
    assert!(
        info.formats[0].url.starts_with("http"),
        "format url should be a real http(s) url, got {:?}",
        info.formats[0].url,
    );
    // Duration should match the "15:31" markup → 931 seconds (per
    // upstream _TEST `'duration': 931`).
    assert_eq!(
        info.duration,
        Some(931.0),
        "duration mismatch — expected 931s from '15:31' markup",
    );
    // NOTE: `age_limit` is not yet in the WIT info-dict shape
    // (`crates/rdlp-plugin/wit/types.wit:40-56`) — the field exists on
    // the Rust-side `InfoDict` but doesn't cross the plugin boundary.
    // The xxxymovies extractor populates it from `_rta_search`, but
    // that value is dropped at WIT marshalling. Lifting age_limit (and
    // dislike_count, display_id, etc.) into the WIT contract is a
    // Slice-2.5 v0.2 add-only schema bump.
    assert_eq!(info.age_limit, None, "age_limit not yet in WIT contract");
}
