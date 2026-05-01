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

use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rdlp_plugin::PluginError;
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::manifest::canonical_bytes;
use rdlp_plugin::prompt::{AlwaysApprove, AlwaysDeny};
use rdlp_plugin::trust_store::TrustStore;
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
wit_version = "0.1.0"
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
wit_version = "0.1.0"
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
wit_version = "0.1.0"
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
