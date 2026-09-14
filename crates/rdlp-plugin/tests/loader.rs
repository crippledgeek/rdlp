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

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rdlp_core::{ExtractionContext, InfoExtractor};
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::BoaJsEngine;
use rdlp_plugin::PluginError;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::manifest::canonical_bytes;
use rdlp_plugin::prompt::{AlwaysApprove, AlwaysDeny};
use rdlp_plugin::trust_store::TrustStore;
use rdlp_types::Config;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

const MINIMAL_COMPONENT_WAT: &str = r#"(component)"#;

fn write_signed_plugin(
    dir: &Path,
    name: &str,
    key: &SigningKey,
    capabilities: &[&str],
    priority: u32,
    claims_override: &[&str],
) {
    std::fs::create_dir_all(dir).unwrap();
    let wasm = wat::parse_str(MINIMAL_COMPONENT_WAT).unwrap();
    std::fs::write(dir.join("plugin.wasm"), &wasm).unwrap();

    let pubkey_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        key.verifying_key().as_bytes(),
    );
    let cap_str = capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let claims_str = claims_override
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml_placeholder = format!(
        r#"
name = "{name}"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://example.com/*"]
priority = {priority}
claims_override = [{claims_str}]
capabilities = [{cap_str}]

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#,
    );

    let mut m = rdlp_plugin::manifest::parse_manifest_str(&toml_placeholder).unwrap();
    let mut buf = canonical_bytes(&m);
    buf.extend_from_slice(&wasm);
    let sig = key.sign(&buf);
    let sig_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, sig.to_bytes());

    if let rdlp_plugin::manifest::Signature::Ed25519 { signature, .. } = &mut m.signature {
        *signature = sig_b64.clone();
    }

    let final_toml = toml_placeholder.replace("PLACEHOLDER", &sig_b64);
    std::fs::write(dir.join("plugin.toml"), final_toml).unwrap();
}

fn make_loader_args(
    td: &TempDir,
    prompter: Arc<dyn rdlp_plugin::prompt::Prompter>,
) -> (Engine, TrustStore, Arc<dyn rdlp_plugin::prompt::Prompter>) {
    let engine = Engine::new(EngineConfig::default()).unwrap();
    let trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    (engine, trust, prompter)
}

#[test]
fn empty_dir_loads_nothing() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);
    assert!(outcomes.is_empty());
}

#[test]
fn missing_dir_returns_empty_with_warn() {
    let td = TempDir::new().unwrap();
    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&td.path().join("nonexistent"));
    assert!(outcomes.is_empty());
}

#[test]
fn first_install_with_approval_loads_plugin() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    write_signed_plugin(
        &plugins_dir.join("youtube"),
        "youtube",
        &key,
        &["log"],
        150,
        &[],
    );

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    let loaded = outcomes[0].as_ref().expect("should load");
    assert_eq!(loaded.manifest.name, "youtube");
    assert!(trust.lookup("youtube").is_some());
}

#[test]
fn first_install_denied_does_not_load() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    write_signed_plugin(&plugins_dir.join("foo"), "foo", &key, &["log"], 150, &[]);

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysDeny));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].is_err());
    assert!(trust.lookup("foo").is_none());
}

#[test]
fn identity_mismatch_refuses_load() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key1 = SigningKey::generate(&mut OsRng);
    let plugin_dir = plugins_dir.join("foo");
    write_signed_plugin(&plugin_dir, "foo", &key1, &["log"], 150, &[]);

    // First install — approved
    {
        let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
        let mut loader = Loader::new(&engine, &mut trust, prompter);
        let outcomes = loader.discover(&plugins_dir);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].is_ok());
    }

    // Re-sign with a different key — same name, different identity
    let key2 = SigningKey::generate(&mut OsRng);
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(&plugin_dir, "foo", &key2, &["log"], 150, &[]);

    let engine = Engine::new(EngineConfig::default()).unwrap();
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let prompter: Arc<dyn rdlp_plugin::prompt::Prompter> = Arc::new(AlwaysApprove);
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        Ok(_) => panic!("expected IdentityMismatch error, got Ok"),
        Err((_, err)) => assert!(
            matches!(err, PluginError::IdentityMismatch { .. }),
            "expected IdentityMismatch, got {:?}",
            err
        ),
    }
}

#[test]
fn bad_signature_logged_and_skipped() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    std::fs::create_dir_all(plugins_dir.join("bad")).unwrap();
    std::fs::write(
        plugins_dir.join("bad").join("plugin.wasm"),
        wat::parse_str("(component)").unwrap(),
    )
    .unwrap();
    std::fs::write(
        plugins_dir.join("bad").join("plugin.toml"),
        r#"
name = "bad"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
# Real ed25519 pubkey (derived from a fixed test seed), paired with a
# syntactically-valid-but-cryptographically-bogus 64-byte signature
# (all 0x01). An all-zeros pubkey hits ed25519-dalek's lax-verify
# low-order edge case where ~24% of messages spuriously verify, and the
# specific canonical_bytes content can tip miss-or-hit across WIT version
# bumps (see PR D-3, issue #274).
pubkey = "mLGicHAE7d8IhiYHhTFCUkGBtcLuwooeU+q8VENgGNM="
signature = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQ=="
"#,
    )
    .unwrap();

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        Ok(_) => panic!("expected SignatureInvalid error, got Ok"),
        Err((_, err)) => assert!(
            matches!(err, PluginError::SignatureInvalid { .. }),
            "expected SignatureInvalid, got {:?}",
            err
        ),
    }
}

#[test]
fn capability_creep_approved_updates_trust_store() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    let plugin_dir = plugins_dir.join("bar");

    // First install with only "log"
    write_signed_plugin(&plugin_dir, "bar", &key, &["log"], 150, &[]);
    {
        let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
        let mut loader = Loader::new(&engine, &mut trust, prompter);
        let outcomes = loader.discover(&plugins_dir);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].is_ok());
    }

    // Update requesting "log" + "fetch" (capability creep)
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(&plugin_dir, "bar", &key, &["fetch", "log"], 150, &[]);

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    if let Err((_, ref e)) = outcomes[0] {
        panic!("expected ok, got {:?}", e);
    }
    // Trust store should now reflect the expanded capability set.
    let entry = trust.lookup("bar").expect("entry should exist");
    assert!(entry.approved_capabilities.contains("fetch"));
    assert!(entry.approved_capabilities.contains("log"));
}

#[test]
fn capability_creep_denied_blocks_load() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    let plugin_dir = plugins_dir.join("baz");

    // First install with only "log"
    write_signed_plugin(&plugin_dir, "baz", &key, &["log"], 150, &[]);
    {
        let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
        let mut loader = Loader::new(&engine, &mut trust, prompter);
        loader.discover(&plugins_dir);
    }

    // Update requesting new capability, denied
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(&plugin_dir, "baz", &key, &["fetch", "log"], 150, &[]);

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysDeny));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        Ok(_) => panic!("expected CapabilityCreep error, got Ok"),
        Err((_, err)) => assert!(
            matches!(err, PluginError::CapabilityCreep { .. }),
            "expected CapabilityCreep, got {:?}",
            err
        ),
    }
}

#[test]
fn dir_without_wasm_is_skipped() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let plugin_dir = plugins_dir.join("incomplete");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    // Write only the manifest, no plugin.wasm
    std::fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "incomplete"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
signature = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
"#,
    )
    .unwrap();

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);
    // Incomplete directories are silently skipped.
    assert!(outcomes.is_empty());
}

/// A plugin to sign with a REAL compiled component (as opposed to
/// `write_signed_plugin`'s `MINIMAL_COMPONENT_WAT` stub) under a
/// caller-chosen `wit_version`. Grouped into one struct (not appended as a
/// 7th positional parameter to `write_signed_plugin`) per
/// `limit-function-arguments`: `wit_version` is the one axis this D1 compat
/// test varies and none of the other tests in this file need it.
struct RealComponentPluginSpec<'a> {
    name: &'a str,
    wasm: &'a [u8],
    wit_version: &'a str,
    matches: &'a [&'a str],
    capabilities: &'a [&'a str],
}

/// Sign real component bytes (not the `(component)` WAT stub) into `dir`,
/// declaring `spec.wit_version` verbatim — the D1 compat tests below sign
/// the SAME `.wasm` under both an accepted and a rejected version to
/// exercise `check_wit_version_against`'s patch boundary through the real
/// loader, not just the unit-level helper.
fn write_signed_real_component_plugin(dir: &Path, key: &SigningKey, spec: RealComponentPluginSpec) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("plugin.wasm"), spec.wasm).unwrap();

    let pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    let cap_str = spec
        .capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let match_str = spec
        .matches
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml_placeholder = format!(
        r#"
name = "{name}"
version = "1.0.0"
wit_version = "{wit_version}"
matches = [{match_str}]
priority = 150
claims_override = []
capabilities = [{cap_str}]

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#,
        name = spec.name,
        wit_version = spec.wit_version,
    );

    let m = rdlp_plugin::manifest::parse_manifest_str(&toml_placeholder).unwrap();
    let mut buf = canonical_bytes(&m);
    buf.extend_from_slice(spec.wasm);
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

/// D1 positive compat test: a component built against 0.5.0 (Task 1
/// fixture) loads through the real loader on this 0.5.1 host and answers
/// `metadata` + `extract`. `wit_version = "0.5.0"` in its manifest takes
/// the patch-below path of `check_wit_version_against`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_5_0_component_loads_on_the_0_5_1_host() {
    let wasm = std::fs::read("tests/fixtures/example-extractor-0.5.0/plugin.wasm").unwrap();
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);

    write_signed_real_component_plugin(
        &plugins_dir.join("example"),
        &key,
        RealComponentPluginSpec {
            name: "example",
            wasm: &wasm,
            wit_version: "0.5.0",
            matches: &["https://example.com/*"],
            // example-extractor's plugin.toml.template declares no
            // capabilities — it is a pure, deterministic plugin.
            capabilities: &[],
        },
    );

    let engine = Arc::new(Engine::new(EngineConfig::default()).unwrap());
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let prompter: Arc<dyn rdlp_plugin::prompt::Prompter> = Arc::new(AlwaysApprove);
    let mut loader = Loader::new(engine.as_ref(), &mut trust, prompter);
    let mut outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1, "expected exactly one discover outcome");
    let loaded = outcomes.remove(0).unwrap_or_else(|(path, err)| {
        panic!("0.5.0 component must load on the 0.5.1 host: {path:?}: {err:?}")
    });
    assert_eq!(loaded.manifest.wit_version, "0.5.0");

    let host_resources = HostResources {
        fetch_client: None,
        cookie_jar: None,
        kv_db: None,
        fetch_fixtures: None,
    };
    let adapter = PluginExtractor::new(loaded, engine.clone(), host_resources)
        .expect("adapter construction must succeed");

    let ctx = make_extraction_ctx();
    let result = adapter.extract("https://example.com/video/1", &ctx).await;
    match result {
        Ok(info) => assert_eq!(info.id, "1"),
        Err(err) => {
            // Any failure here must be a domain error the plugin itself
            // returned (e.g. a future ExtractError variant), never an
            // instantiate/trap fault — the 3-strike trap counter stays at
            // zero for domain errors (see adapter.rs's `extract`).
            assert_eq!(
                adapter.test_trap_count(),
                0,
                "expected a domain error, got a trap/instantiate fault: {err}"
            );
        }
    }
}

/// D1 negative compat test: the SAME 0.5.0 component, but the manifest
/// claims a NEWER patch (`0.5.2`) than this 0.5.1 host accepts. Rejected at
/// `discover` — before the component is even compiled for instantiation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_component_declaring_a_newer_patch_is_rejected() {
    let wasm = std::fs::read("tests/fixtures/example-extractor-0.5.0/plugin.wasm").unwrap();
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);

    write_signed_real_component_plugin(
        &plugins_dir.join("example"),
        &key,
        RealComponentPluginSpec {
            name: "example",
            wasm: &wasm,
            wit_version: "0.5.2",
            matches: &["https://example.com/*"],
            capabilities: &[],
        },
    );

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        Ok(_) => panic!("expected WitVersionMismatch, got Ok"),
        Err((_, err)) => assert!(
            matches!(err, PluginError::WitVersionMismatch { .. }),
            "expected WitVersionMismatch, got {err:?}"
        ),
    }
}
