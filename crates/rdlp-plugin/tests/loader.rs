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

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use rdlp_core::InfoExtractor;
use rdlp_plugin::PluginError;
use rdlp_plugin::adapter::{HostResources, PluginExtractor};
use rdlp_plugin::engine::{Engine, EngineConfig};
use rdlp_plugin::loader::Loader;
use rdlp_plugin::manifest::SearchClaims;
use rdlp_plugin::prompt::{AlwaysApprove, AlwaysDeny, ConfirmRequest, ConfirmResponse};
use rdlp_plugin::test_support::{
    EXAMPLE_0_5_0_WASM, RecordingPrompter, SignedPluginSpec, discover_signed_after, extraction_ctx,
    write_signed_plugin,
};
use rdlp_plugin::trust_store::TrustStore;
use std::sync::{Arc, Mutex, Once};
use tempfile::TempDir;

const MINIMAL_COMPONENT_WAT: &str = r#"(component)"#;

/// The `(component)` WAT stub most tests here sign: it compiles, so the
/// loader reaches the trust-store step, and it exports nothing, which is
/// all these tests need.
fn stub_wasm() -> Vec<u8> {
    wat::parse_str(MINIMAL_COMPONENT_WAT).unwrap()
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
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugins_dir.join("youtube"),
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..SignedPluginSpec::stub("youtube", &wasm)
        },
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
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugins_dir.join("foo"),
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..SignedPluginSpec::stub("foo", &wasm)
        },
    );

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysDeny));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].is_err());
    assert!(trust.lookup("foo").is_none());
}

/// Records this crate's WARN-and-above `log` output for one assertion.
///
/// Not `testing_logger`: that sets the global max level to `Trace`, and
/// wasmtime's compile threads then pretty-print pre-regalloc instructions
/// through a path that is `unreachable!()` for virtual registers
/// (cranelift-codegen 0.117 `x64/inst/external.rs::enc`) — every test in
/// the binary that compiles a component aborts. Filtering to this crate's
/// target at `Warn` keeps cranelift's tracing off.
struct WarnCapture;

static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();

impl log::Log for WarnCapture {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target().starts_with("rdlp_plugin")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            CAPTURED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

fn captured_warnings() -> Vec<String> {
    INSTALL.call_once(|| {
        log::set_logger(&WarnCapture).expect("no other logger in this test binary");
        log::set_max_level(log::LevelFilter::Warn);
    });
    CAPTURED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// `discover` hands a failed load back as `Err((dir, e))` for the caller
/// to report; logging it here as well printed every failure twice in
/// rdlp's bootstrap (`rdlp_plugin::loader` WARN, then the identical
/// `rdlp_api::plugin_bootstrap` WARN).
#[test]
fn a_failed_load_is_returned_to_the_caller_not_logged_as_well() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugins_dir.join("foo"),
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..SignedPluginSpec::stub("foo", &wasm)
        },
    );

    let _ = captured_warnings(); // installs the capture before the load
    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysDeny));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert!(outcomes[0].is_err(), "the denial is returned");
    let echoed: Vec<String> = captured_warnings()
        .into_iter()
        .filter(|body| body.contains("failed to load"))
        .collect();
    assert!(echoed.is_empty(), "returned AND logged: {echoed:?}");
}

#[test]
fn identity_mismatch_refuses_load() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key1 = SigningKey::generate(&mut OsRng);
    let plugin_dir = plugins_dir.join("foo");
    let wasm = stub_wasm();
    let foo = SignedPluginSpec {
        capabilities: &["log"],
        ..SignedPluginSpec::stub("foo", &wasm)
    };
    write_signed_plugin(&plugin_dir, &key1, &foo);

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
    write_signed_plugin(&plugin_dir, &key2, &foo);

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

/// A `plugin.wasm` over the cap is refused from its size alone — before
/// it is read, so before the signature is checked (#784). The file is
/// sparse: `set_len` past the cap costs no disk and no time, and a
/// refusal that had read the bytes first would have found a signature
/// mismatch, not a size error.
#[test]
fn an_oversized_wasm_is_refused_by_size_before_it_is_read() {
    use rdlp_plugin::signature::MAX_PLUGIN_WASM_BYTES;

    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugins_dir.join("huge"),
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..SignedPluginSpec::stub("huge", &wasm)
        },
    );
    let wasm_path = plugins_dir.join("huge").join("plugin.wasm");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&wasm_path)
        .unwrap()
        .set_len(MAX_PLUGIN_WASM_BYTES + 1)
        .unwrap();

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let outcomes = loader.discover(&plugins_dir);

    assert_eq!(outcomes.len(), 1);
    let Err((_, err)) = &outcomes[0] else {
        panic!("an oversized plugin must be refused")
    };
    assert!(
        matches!(
            err,
            PluginError::WasmTooLarge { bytes, max, .. }
                if *bytes == MAX_PLUGIN_WASM_BYTES + 1 && *max == MAX_PLUGIN_WASM_BYTES
        ),
        "size refusal, not a signature one: {err}"
    );
    assert!(trust.lookup("huge").is_none(), "nothing recorded");
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
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugin_dir,
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..SignedPluginSpec::stub("bar", &wasm)
        },
    );
    {
        let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
        let mut loader = Loader::new(&engine, &mut trust, prompter);
        let outcomes = loader.discover(&plugins_dir);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].is_ok());
    }

    // Update requesting "log" + "fetch" (capability creep)
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(
        &plugin_dir,
        &key,
        &SignedPluginSpec {
            capabilities: &["fetch", "log"],
            ..SignedPluginSpec::stub("bar", &wasm)
        },
    );

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
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugin_dir,
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..SignedPluginSpec::stub("baz", &wasm)
        },
    );
    {
        let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
        let mut loader = Loader::new(&engine, &mut trust, prompter);
        loader.discover(&plugins_dir);
    }

    // Update requesting new capability, denied
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(
        &plugin_dir,
        &key,
        &SignedPluginSpec {
            capabilities: &["fetch", "log"],
            ..SignedPluginSpec::stub("baz", &wasm)
        },
    );

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

/// Same construction as `make_loader_args`, but with the engine already
/// D1 positive compat test: a component built against 0.5.0 (Task 1
/// fixture) loads through the real loader on the current host and answers
/// `metadata` + `extract`. `wit_version = "0.5.0"` in its manifest takes
/// the patch-below path of `check_wit_version_against`. The D1 negative
/// (a manifest claiming `0.5.3`) lives in `tests/abi_0_5_2_fixture.rs`,
/// where it runs over both committed fixtures through the same
/// `discover_signed_after` helper this test uses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_0_5_0_component_loads_on_the_current_host() {
    let td = TempDir::new().unwrap();
    let (engine, outcome) = discover_signed_after(td.path(), &SignedPluginSpec::example(), |_| {});
    let loaded = outcome.unwrap_or_else(|(path, err)| {
        panic!("0.5.0 component must load on the current host: {path:?}: {err:?}")
    });
    assert_eq!(loaded.manifest.wit_version, "0.5.0");

    let adapter = PluginExtractor::new(loaded, engine, HostResources::default())
        .expect("adapter construction must succeed");

    let ctx = extraction_ctx();
    let info = adapter
        .extract("https://example.com/video/1", &ctx)
        .await
        .expect("the 0.5.0 fixture must extract on the current host");
    assert_eq!(info.id, "1");
    assert_eq!(adapter.test_trap_count(), 0);
}

// ── discovery order (code review I5) ──────────────────────────────────────

/// `discover` returns plugins in path order regardless of the order the
/// filesystem lists them, so "first registered wins" tie-breaks downstream
/// are deterministic across machines and filesystems.
#[test]
fn discover_returns_plugins_in_path_order() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();
    // Written b-first so a listing that echoes creation order differs from
    // path order.
    for name in ["b-plugin", "a-plugin", "c-plugin"] {
        write_signed_plugin(
            &plugins_dir.join(name),
            &key,
            &SignedPluginSpec::stub(name, &wasm),
        );
    }

    let (engine, mut trust, prompter) = make_loader_args(&td, Arc::new(AlwaysApprove));
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    let names: Vec<String> = loader
        .discover(&plugins_dir)
        .into_iter()
        .map(|o| o.expect("stub plugins load").manifest.name)
        .collect();
    assert_eq!(names, ["a-plugin", "b-plugin", "c-plugin"]);
}

// ── search-site claim binding (security M3) ───────────────────────────────

/// A search plugin claiming the built-in `pornhub`'s site: the shape a
/// first-install prompt must surface and the trust store must remember.
fn pornhub_claimant<'a>(wasm: &'a [u8], claims_override: &'a [&'a str]) -> SignedPluginSpec<'a> {
    SignedPluginSpec {
        supports_search: true,
        search_site: Some("pornhub"),
        search_claims_override: claims_override,
        ..SignedPluginSpec::stub("ph-search", wasm)
    }
}

fn load_with<P: rdlp_plugin::prompt::Prompter + 'static>(
    td: &TempDir,
    plugins_dir: &std::path::Path,
    prompter: Arc<P>,
) -> Vec<rdlp_plugin::loader::DiscoverOutcome> {
    let engine = Engine::new(EngineConfig::default()).unwrap();
    let mut trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let mut loader = Loader::new(&engine, &mut trust, prompter);
    loader.discover(plugins_dir)
}

/// The first-install prompt names the site the plugin will search AND the
/// override it claims, and an `ApprovePersist` records both.
#[test]
fn first_install_prompt_shows_the_search_claim_and_the_store_records_it() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();
    write_signed_plugin(
        &plugins_dir.join("ph-search"),
        &key,
        &pornhub_claimant(&wasm, &["pornhub"]),
    );

    let prompter = Arc::new(RecordingPrompter::answering(
        ConfirmResponse::ApprovePersist,
    ));
    let outcomes = load_with(&td, &plugins_dir, Arc::clone(&prompter));
    assert!(outcomes[0].is_ok(), "{:?}", outcomes[0].as_ref().err());

    let expected = SearchClaims {
        search_site: Some("pornhub".into()),
        search_claims_override: vec!["pornhub".into()],
    };
    match prompter.requests().as_slice() {
        [ConfirmRequest::FirstInstall { search, .. }] => assert_eq!(*search, expected),
        other => panic!("expected one FirstInstall, got {other:?}"),
    }
    let trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    assert_eq!(
        trust.lookup("ph-search").expect("recorded").search,
        expected
    );
}

/// An update that starts claiming the built-in's search is re-confirmed
/// like capability creep: denied, it does not load; approved, the store
/// records the new claim; and an unchanged claim prompts nothing.
#[test]
fn a_changed_search_claim_reprompts_and_an_unchanged_one_does_not() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let plugin_dir = plugins_dir.join("ph-search");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();

    // First install: serves pornhub, claims no override.
    write_signed_plugin(&plugin_dir, &key, &pornhub_claimant(&wasm, &[]));
    let first = Arc::new(RecordingPrompter::answering(
        ConfirmResponse::ApprovePersist,
    ));
    assert!(load_with(&td, &plugins_dir, Arc::clone(&first))[0].is_ok());
    assert_eq!(first.requests().len(), 1, "one FirstInstall");

    // Same manifest again: nothing to confirm.
    let again = Arc::new(RecordingPrompter::answering(ConfirmResponse::Deny));
    assert!(load_with(&td, &plugins_dir, Arc::clone(&again))[0].is_ok());
    assert!(
        again.requests().is_empty(),
        "an unchanged claim must not prompt"
    );

    // Update now claims the override — denied.
    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(&plugin_dir, &key, &pornhub_claimant(&wasm, &["pornhub"]));
    let deny = Arc::new(RecordingPrompter::answering(ConfirmResponse::Deny));
    let outcomes = load_with(&td, &plugins_dir, Arc::clone(&deny));
    match &outcomes[0] {
        Err((_, PluginError::SearchClaimsChange { plugin, .. })) => assert_eq!(plugin, "ph-search"),
        Err((_, other)) => panic!("expected SearchClaimsChange, got {other:?}"),
        Ok(_) => panic!("expected SearchClaimsChange, got Ok"),
    }
    match deny.requests().as_slice() {
        [
            ConfirmRequest::SearchClaimsChange {
                previously_approved,
                requested,
                ..
            },
        ] => {
            assert!(previously_approved.search_claims_override.is_empty());
            assert_eq!(requested.search_claims_override, vec!["pornhub"]);
        }
        other => panic!("expected one SearchClaimsChange, got {other:?}"),
    }

    // Same update, approved and persisted — the store now carries the claim.
    let approve = Arc::new(RecordingPrompter::answering(
        ConfirmResponse::ApprovePersist,
    ));
    assert!(load_with(&td, &plugins_dir, approve)[0].is_ok());
    let trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    assert_eq!(
        trust
            .lookup("ph-search")
            .unwrap()
            .search
            .search_claims_override,
        vec!["pornhub"]
    );
}

/// The committed example fixture is what `SignedPluginSpec::example`
/// signs; pin that so a fixture swap cannot silently change every test
/// built on it.
#[test]
fn example_spec_signs_the_committed_fixture() {
    assert_eq!(SignedPluginSpec::example().wasm, EXAMPLE_0_5_0_WASM);
    assert_eq!(SignedPluginSpec::example().name, "example");
}

/// An update that bundles a new capability WITH a new search claim must
/// fire BOTH prompts. The trap this pins: an `ApprovePersist` on the
/// capability-creep prompt records the whole entry — new claim included —
/// and a claims check that reads the store afterwards finds nothing to
/// confirm, so the plugin shadows a built-in's search behind a prompt that
/// never mentioned search.
#[test]
fn a_bundled_capability_and_search_claim_change_fires_both_prompts() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let plugin_dir = plugins_dir.join("ph-search");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();

    write_signed_plugin(&plugin_dir, &key, &pornhub_claimant(&wasm, &[]));
    let first = Arc::new(RecordingPrompter::answering(
        ConfirmResponse::ApprovePersist,
    ));
    assert!(load_with(&td, &plugins_dir, first)[0].is_ok());

    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(
        &plugin_dir,
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..pornhub_claimant(&wasm, &["pornhub"])
        },
    );
    let both = Arc::new(RecordingPrompter::answering(
        ConfirmResponse::ApprovePersist,
    ));
    assert!(load_with(&td, &plugins_dir, Arc::clone(&both))[0].is_ok());

    let requests = both.requests();
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, ConfirmRequest::CapabilityCreep { .. })),
        "capability creep must be prompted: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, ConfirmRequest::SearchClaimsChange { .. })),
        "the bundled search claim must be prompted too: {requests:?}"
    );

    let trust = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let entry = trust.lookup("ph-search").unwrap();
    assert!(entry.approved_capabilities.contains("log"));
    assert_eq!(entry.search.search_claims_override, vec!["pornhub"]);
}

/// A prompter that answers per request kind: persist the capability creep,
/// deny the search-claim change.
struct PersistCapsDenyClaims(RecordingPrompter);

impl rdlp_plugin::prompt::Prompter for PersistCapsDenyClaims {
    fn confirm(&self, request: ConfirmRequest) -> ConfirmResponse {
        let answer = match &request {
            ConfirmRequest::SearchClaimsChange { .. } => ConfirmResponse::Deny,
            _ => ConfirmResponse::ApprovePersist,
        };
        let _ = self.0.confirm(request);
        answer
    }
}

/// Persisting the first prompt's approval must not write the second
/// prompt's (denied) claim: with capability creep approved-and-persisted
/// and the bundled search claim denied, the load is refused AND the next
/// startup still asks about the search claim — the store may only ever
/// record what every prompt in the load agreed to.
#[test]
fn a_denied_search_claim_is_not_persisted_by_an_earlier_approval() {
    let td = TempDir::new().unwrap();
    let plugins_dir = td.path().join("plugins");
    let plugin_dir = plugins_dir.join("ph-search");
    let key = SigningKey::generate(&mut OsRng);
    let wasm = stub_wasm();

    write_signed_plugin(&plugin_dir, &key, &pornhub_claimant(&wasm, &[]));
    let first = Arc::new(RecordingPrompter::answering(
        ConfirmResponse::ApprovePersist,
    ));
    assert!(load_with(&td, &plugins_dir, first)[0].is_ok());

    std::fs::remove_dir_all(&plugin_dir).unwrap();
    write_signed_plugin(
        &plugin_dir,
        &key,
        &SignedPluginSpec {
            capabilities: &["log"],
            ..pornhub_claimant(&wasm, &["pornhub"])
        },
    );
    let split = Arc::new(PersistCapsDenyClaims(RecordingPrompter::answering(
        ConfirmResponse::Deny,
    )));
    let outcomes = load_with(&td, &plugins_dir, Arc::clone(&split));
    assert!(
        matches!(
            &outcomes[0],
            Err((_, PluginError::SearchClaimsChange { .. }))
        ),
        "the denied claim must refuse this load"
    );
    assert_eq!(split.0.requests().len(), 2, "both prompts fired");

    let entry_after = TrustStore::open(td.path().join("trust.toml")).unwrap();
    let entry = entry_after
        .lookup("ph-search")
        .expect("the first install's entry");
    assert!(
        entry.search.search_claims_override.is_empty(),
        "a denied claim must not reach the store: {entry:?}"
    );
    assert!(
        !entry.approved_capabilities.contains("log"),
        "an approval that was part of a refused load must not be persisted either: {entry:?}"
    );

    // Next startup, same answers: the claim is still unapproved, so it is
    // asked again (a store that had absorbed it would load silently).
    let again = Arc::new(PersistCapsDenyClaims(RecordingPrompter::answering(
        ConfirmResponse::Deny,
    )));
    let outcomes = load_with(&td, &plugins_dir, Arc::clone(&again));
    assert!(
        matches!(
            &outcomes[0],
            Err((_, PluginError::SearchClaimsChange { .. }))
        ),
        "got {:?}",
        outcomes[0].as_ref().err()
    );
    assert!(
        again
            .0
            .requests()
            .iter()
            .any(|r| matches!(r, ConfirmRequest::SearchClaimsChange { .. })),
        "the next startup must still prompt for the search claim: {:?}",
        again.0.requests()
    );
}
