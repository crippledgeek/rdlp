//! Test-support seams shared by this crate's unit tests, its `tests/`
//! integration binaries, and — for the signer, identity helper, and
//! config-dir isolation wrapper — other crates' integration tests
//! (`rdlp-api`'s bootstrap tests need to fabricate a loadable signed
//! plugin the same way this crate's own tests do, and a second
//! hand-rolled copy of any of these mechanisms is exactly what
//! `extract-before-you-duplicate` forbids).
//!
//! Compiled into the library (not `cfg(test)`) because an integration
//! test binary links the library as an external crate and cannot see its
//! `cfg(test)` items. Hidden from docs like the `test_*` accessors on
//! `PluginExtractor`, which follow the same precedent.

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rdlp_core::ExtractionContext;

use crate::adapter::{HostResources, PluginExtractor};
use crate::engine::{Engine, EngineConfig};
use crate::loader::{DiscoverOutcome, Loader};
use crate::manifest::{Manifest, Signature, canonical_bytes};
use crate::prompt::{AlwaysApprove, ConfirmRequest, ConfirmResponse, Prompter};
use crate::trust_store::TrustStore;

/// The committed 0.5.0 example component (`tests/fixtures/example-extractor-0.5.0`):
/// WASI-free, no capabilities, implements `extract` and `search`, instantiates
/// on the host world — see its README. The one copy of the fixture's bytes
/// every test in this crate and `rdlp-api` loads.
#[doc(hidden)]
pub const EXAMPLE_0_5_0_WASM: &[u8] =
    include_bytes!("../tests/fixtures/example-extractor-0.5.0/plugin.wasm");

/// The committed 0.5.2 example component (`tests/fixtures/example-extractor-0.5.2`):
/// the same source as the 0.5.0 one, rebuilt against `rdlp:plugin@0.5.2`
/// with `search-filters`, `extract-playlist` and `extract-with-metadata`
/// exported — see its README for what each answers per URL. The one copy
/// of those bytes for the playlist/metadata unit tests and
/// `tests/abi_0_5_2_fixture.rs`.
#[doc(hidden)]
pub const EXAMPLE_0_5_2_WASM: &[u8] =
    include_bytes!("../tests/fixtures/example-extractor-0.5.2/plugin.wasm");

/// The default extraction context a plugin test hands to `extract` /
/// `search`: a stock HTTP client, the boa engine, an empty cookie jar and
/// default config. Formerly six identical copies (the adapter unit tests
/// and five integration binaries).
#[doc(hidden)]
#[must_use]
pub fn extraction_ctx() -> ExtractionContext {
    ExtractionContext::new(
        Arc::new(rdlp_http::HttpClientFactory::default().build()),
        Arc::new(rdlp_jsinterp::BoaJsEngine::new()),
        Arc::new(rdlp_cookies::SimpleCookieJar::new()),
        Arc::new(rdlp_types::Config::default()),
    )
}

/// Every manifest field this crate's (and `rdlp-api`'s) integration tests
/// vary across their signed-plugin fixtures. `dir` and `key` stay as
/// [`write_signed_plugin`]'s own parameters (they answer "where" and "who
/// signs", not "what manifest").
///
/// Start from [`SignedPluginSpec::stub`] / [`SignedPluginSpec::example`]
/// and override the fields a test is about with struct-update syntax, so a
/// call site names only what it varies.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct SignedPluginSpec<'a> {
    /// Plugin name — always varies (one per plugin fixture).
    pub name: &'a str,
    /// Plugin semver version.
    pub version: &'a str,
    /// Target WIT contract version this manifest declares.
    pub wit_version: &'a str,
    /// Chrome-style match patterns.
    pub matches: &'a [&'a str],
    /// Optional fine-grained regex for ID extraction.
    pub url_regex: Option<&'a str>,
    /// Plugin priority within the band 100..=199.
    pub priority: u32,
    /// Hostnames this plugin claims the right to shadow from a built-in.
    pub claims_override: &'a [&'a str],
    /// Host capabilities the plugin requests.
    pub capabilities: &'a [&'a str],
    /// The manifest's `supports_extract` value. This struct field is a
    /// plain mandatory `bool` — every caller states it explicitly; it is
    /// only the TOML *parser*'s `#[serde(default = "default_true")]` that
    /// treats an omitted `supports_extract` key as `true` for manifests
    /// written before the field existed.
    pub supports_extract: bool,
    /// Whether the manifest declares `supports_search`.
    pub supports_search: bool,
    /// The manifest's `search_site` (`None` = the plugin's own name).
    pub search_site: Option<&'a str>,
    /// The manifest's `search_claims_override`.
    pub search_claims_override: &'a [&'a str],
    /// The compiled component bytes to sign and write.
    pub wasm: &'a [u8],
}

impl<'a> SignedPluginSpec<'a> {
    /// A stub plugin: `wasm` is whatever bytes the caller supplies (a
    /// `(component)` WAT stub, or bytes that need not be a component at
    /// all when only signing is under test), one `example.com` match
    /// pattern, priority 150, no capabilities, extract-only.
    #[must_use]
    pub const fn stub(name: &'a str, wasm: &'a [u8]) -> Self {
        Self {
            name,
            version: "1.0.0",
            wit_version: "0.5.0",
            matches: &["https://example.com/*"],
            url_regex: None,
            priority: 150,
            claims_override: &[],
            capabilities: &[],
            supports_extract: true,
            supports_search: false,
            search_site: None,
            search_claims_override: &[],
            wasm,
        }
    }

    /// The committed 0.5.0 example component under its template's name
    /// and match pattern — a pure, deterministic plugin with no
    /// capabilities, as `examples/plugins/example-extractor`'s
    /// `plugin.toml.template` declares it.
    #[must_use]
    pub const fn example() -> Self {
        Self {
            version: "0.1.0",
            ..Self::stub("example", EXAMPLE_0_5_0_WASM)
        }
    }

    /// The committed 0.5.2 example component under the same template:
    /// `wit_version = "0.5.2"` (the contract it was built against) and
    /// `supports_search = true` (it exports `search` + `search-filters`).
    #[must_use]
    pub const fn example_0_5_2() -> Self {
        Self {
            wit_version: "0.5.2",
            supports_search: true,
            wasm: EXAMPLE_0_5_2_WASM,
            ..Self::example()
        }
    }
}

/// Base64 of the key's 32-byte public key, as the manifest carries it.
fn pubkey_b64(key: &SigningKey) -> String {
    base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes())
}

/// Sign `manifest` (whose signature block must already carry `key`'s
/// public key) over `canonical_bytes(manifest) || wasm`, writing the
/// signature into it. The one signing mechanism every plugin test across
/// the workspace shares — [`write_signed_plugin`] here and
/// `tests/signature_ed25519.rs`'s in-memory manifests alike.
///
/// # Panics
///
/// If `manifest.signature` is not `Ed25519`.
#[doc(hidden)]
pub fn sign_manifest(manifest: &mut Manifest, key: &SigningKey, wasm: &[u8]) {
    let mut buf = canonical_bytes(manifest);
    buf.extend_from_slice(wasm);
    let sig = key.sign(&buf);
    match &mut manifest.signature {
        Signature::Ed25519 { signature, .. } => {
            *signature = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
        }
        Signature::Sigstore { .. } => panic!("sign_manifest signs Ed25519 manifests only"),
    }
}

/// Write a signed `plugin.toml` + `plugin.wasm` into `dir` so
/// `Loader::discover` accepts it. Builds the [`Manifest`] directly from
/// `spec`, signs it with [`sign_manifest`], serialises it with `toml`
/// (so any `url_regex` is escaped correctly), and re-parses the written
/// text so the fixture is validated exactly as the loader will validate it.
#[doc(hidden)]
pub fn write_signed_plugin(dir: &Path, key: &SigningKey, spec: &SignedPluginSpec<'_>) {
    write_fixture_file(dir, "plugin.wasm", spec.wasm);

    let owned = |items: &[&str]| items.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    let mut manifest = Manifest {
        name: spec.name.to_string(),
        display_name: None,
        version: spec.version.to_string(),
        wit_version: spec.wit_version.to_string(),
        matches: owned(spec.matches),
        url_regex: spec.url_regex.map(str::to_string),
        priority: spec.priority,
        claims_override: owned(spec.claims_override),
        supports_search: spec.supports_search,
        supports_extract: spec.supports_extract,
        search_site: spec.search_site.map(str::to_string),
        search_claims_override: owned(spec.search_claims_override),
        capabilities: owned(spec.capabilities),
        signature: Signature::Ed25519 {
            pubkey: pubkey_b64(key),
            signature: String::new(),
        },
    };
    sign_manifest(&mut manifest, key, spec.wasm);

    let text = toml::to_string(&manifest).unwrap_or_else(|e| panic!("serialise manifest: {e}"));
    crate::manifest::parse_manifest_str(&text)
        .unwrap_or_else(|e| panic!("the manifest write_signed_plugin built is invalid: {e}"));
    write_fixture_file(dir, "plugin.toml", text.as_bytes());
}

/// Create `dir` and write `contents` to `dir.join(name)`. The one place
/// sync `std::fs` I/O happens in this module — a single scoped allow
/// covers both calls instead of one per call site.
///
/// Test fixture — sync I/O is acceptable per clippy.toml's
/// disallowed-methods carve-out (c); every caller is a
/// `#[test]`/`#[tokio::test]` setup step, never a hot async path.
#[allow(clippy::disallowed_methods)]
fn write_fixture_file(dir: &Path, name: &str, contents: &[u8]) {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
    std::fs::write(dir.join(name), contents)
        .unwrap_or_else(|e| panic!("write {}/{name}: {e}", dir.display()));
}

/// Sign `spec` under a fresh key into `<root>/plugins/<spec.name>`, let
/// `tamper` edit that directory (a no-op closure for the honest case),
/// then run the production [`Loader::discover`] over `<root>/plugins` with
/// an [`AlwaysApprove`] prompter and a trust store at `<root>/trust.toml`.
/// Returns the engine (an adapter built from the outcome must share it)
/// and the one outcome `discover` produced for that directory.
///
/// The one "sign, then discover through the real loader" mechanism for
/// every fixture: the ABI compat tests (`tests/loader.rs`,
/// `tests/abi_0_5_2_fixture.rs`) and the example search e2e. `tamper`
/// runs between the write and the discover so a refusal test can flip a
/// byte of the signed artefact or rewrite the manifest without a second
/// copy of this wiring.
///
/// # Panics
///
/// If `discover` returns anything other than exactly one outcome — the
/// directory holds exactly the one plugin this wrote.
#[doc(hidden)]
pub fn discover_signed_after(
    root: &Path,
    spec: &SignedPluginSpec<'_>,
    tamper: impl FnOnce(&Path),
) -> (Arc<Engine>, DiscoverOutcome) {
    let plugins_dir = root.join("plugins");
    let dir = plugins_dir.join(spec.name);
    let key = SigningKey::generate(&mut rand::rngs::OsRng);
    write_signed_plugin(&dir, &key, spec);
    tamper(&dir);

    let engine =
        Arc::new(Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("engine: {e}")));
    let mut trust =
        TrustStore::open(root.join("trust.toml")).unwrap_or_else(|e| panic!("trust store: {e}"));
    let mut loader = Loader::new(engine.as_ref(), &mut trust, Arc::new(AlwaysApprove));
    let mut outcomes = loader.discover(&plugins_dir);
    assert_eq!(
        outcomes.len(),
        1,
        "discover over a directory holding one plugin yields exactly one outcome"
    );
    (engine, outcomes.remove(0))
}

/// [`discover_signed_after`] with no tampering, unwrapped into a
/// [`PluginExtractor`] with no host resources — the shape every fixture
/// that declares no capabilities loads through.
///
/// # Panics
///
/// If the loader refuses the plugin or the adapter cannot be built.
#[doc(hidden)]
#[must_use]
pub fn load_signed_adapter(root: &Path, spec: &SignedPluginSpec<'_>) -> PluginExtractor {
    let (engine, outcome) = discover_signed_after(root, spec, |_| {});
    let loaded = outcome.unwrap_or_else(|(path, err)| {
        panic!(
            "the signed fixture must load through the production loader: {}: {err:?}",
            path.display()
        )
    });
    PluginExtractor::new(loaded, engine, HostResources::default())
        .unwrap_or_else(|e| panic!("adapter: {e}"))
}

/// The trust-store identity string for a signing key, computed by the
/// production `Signature::identity_string` over the manifest's base64
/// public key. Every test that pre-trusts a freshly generated key (rather
/// than going through the interactive prompter) needs this to populate
/// `Config::plugin_trusted_publishers`.
#[doc(hidden)]
#[must_use]
pub fn trusted_identity_for(key: &SigningKey) -> String {
    Signature::Ed25519 {
        pubkey: pubkey_b64(key),
        signature: String::new(),
    }
    .identity_string()
}

/// Sign `spec` under a fresh key into `dir.join(spec.name)`, and a `Config`
/// that pre-trusts that key with `dir` as its one plugin directory — so a
/// `bootstrap_plugins`/`RdlpClient` test loads the fixture without an
/// interactive prompt. The caller keeps `dir` alive for the test's
/// duration.
#[doc(hidden)]
#[must_use]
pub fn signed_fixture_config(dir: &Path, spec: &SignedPluginSpec<'_>) -> rdlp_types::Config {
    let key = SigningKey::generate(&mut rand::rngs::OsRng);
    write_signed_plugin(&dir.join(spec.name), &key, spec);
    rdlp_types::Config {
        plugin_directories: vec![dir.to_path_buf()],
        plugin_trusted_publishers: vec![trusted_identity_for(&key)],
        ..Default::default()
    }
}

/// A [`Prompter`] that records every request it is shown and answers with
/// a fixed response, so a test can assert what the loader asked.
#[doc(hidden)]
pub struct RecordingPrompter {
    requests: std::sync::Mutex<Vec<ConfirmRequest>>,
    answer: ConfirmResponse,
}

impl RecordingPrompter {
    /// A prompter answering `answer` to everything.
    #[must_use]
    pub const fn answering(answer: ConfirmResponse) -> Self {
        Self {
            requests: std::sync::Mutex::new(Vec::new()),
            answer,
        }
    }

    /// Every request shown so far, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<ConfirmRequest> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Prompter for RecordingPrompter {
    fn confirm(&self, request: ConfirmRequest) -> ConfirmResponse {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        self.answer
    }
}

/// Run `f` with `XDG_CONFIG_HOME` and `HOME` both pointing at a fresh temp
/// directory, so a plugin-bootstrap test never reads or writes the real
/// `~/.config/rdlp` trust store. `bootstrap_plugins` resolves its config
/// directory via `dirs::config_dir()` → `XDG_CONFIG_HOME` → `HOME`, so
/// pointing all of them at the same tempdir guarantees a clean slate.
/// `temp-env`'s internal mutex serialises this against any other test in
/// the same process that reads or writes these vars; the tempdir is
/// cleaned up when `f` returns. `f` receives the tempdir's path so a test
/// that needs to write directly under `$XDG_CONFIG_HOME` (e.g. a
/// `<config_dir>/rdlp/plugin-disabled.toml` fixture) can do so without
/// creating a second, unrelated tempdir.
#[doc(hidden)]
pub fn with_isolated_config_dir<R>(f: impl FnOnce(&Path) -> R) -> R {
    let tempdir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let path_str = tempdir
        .path()
        .to_str()
        .unwrap_or_else(|| panic!("tempdir path is not utf-8: {}", tempdir.path().display()));
    temp_env::with_vars(
        [
            ("XDG_CONFIG_HOME", Some(path_str)),
            ("HOME", Some(path_str)),
        ],
        || f(tempdir.path()),
    )
}

/// Unit-test-only seams shared by more than one `#[cfg(test)]` module in
/// this crate (the adapter, search adapter, conversion, and host-import
/// tests), so no test module has to reach into a sibling's `mod tests`.
#[cfg(test)]
pub(crate) mod unit {
    use std::sync::Arc;

    use crate::adapter::{HostResources, PluginExtractor};
    use crate::convert::PluginOrigin;
    use crate::engine::{Engine, EngineConfig};
    use crate::loader::LoadedPlugin;
    use crate::manifest::parse_manifest_str;

    /// Placeholder signature: `parse_manifest_str` checks shape only;
    /// verification happens in the loader, which these fixtures bypass.
    pub const FIXTURE_MANIFEST: &str = r#"
name = "example"
version = "0.0.1"
wit_version = "0.5.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = []

[signature]
type = "ed25519"
pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
signature = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#;

    /// [`FIXTURE_MANIFEST`] with `extra_top_level_lines` spliced into its
    /// top-level key block, after `capabilities = []`. The fixture ends
    /// with a `[signature]` table, so appending after the text would land
    /// the lines inside that table — invalid TOML; splicing above it keeps
    /// the fixture valid whatever the lines are (`display_name`,
    /// `search_site`, an override claim).
    pub fn fixture_manifest_with(extra_top_level_lines: &str) -> String {
        FIXTURE_MANIFEST.replace(
            "capabilities = []",
            &format!("capabilities = []\n{extra_top_level_lines}"),
        )
    }

    /// [`FIXTURE_MANIFEST`] under another plugin `name` — for a test that
    /// must tell its own log lines from every other fixture-driven test's
    /// in the binary: the plugin's log target is derived from its name
    /// (`PluginStoreData::new`), so a unique name is a unique target.
    pub fn fixture_manifest_named(name: &str) -> String {
        FIXTURE_MANIFEST.replace(r#"name = "example""#, &format!("name = {name:?}"))
    }

    /// The 0.5.0 fixture wrapped in an adapter, on a manifest with no
    /// `search_site` and no override claim of either kind.
    pub fn fixture_extractor() -> PluginExtractor {
        fixture_extractor_with_manifest(FIXTURE_MANIFEST)
    }

    /// The 0.5.0 fixture on `toml` — for tests that need a manifest field
    /// (a `search_site`, an override claim) the default fixture lacks.
    pub fn fixture_extractor_with_manifest(toml: &str) -> PluginExtractor {
        fixture_extractor_from(toml, super::EXAMPLE_0_5_0_WASM)
    }

    /// Build an adapter from `toml` and `wasm` directly — the seam
    /// [`fixture_extractor_with_manifest`] uses for the committed 0.5.0
    /// fixture. A future fixture on a later WIT contract version (e.g. a
    /// 0.5.2 component with an `extract-playlist` export) uses this
    /// directly rather than a second hand-rolled copy of the same
    /// engine/component/loader wiring.
    pub fn fixture_extractor_from(toml: &str, wasm: &[u8]) -> PluginExtractor {
        let engine = Arc::new(Engine::new(EngineConfig::default()).expect("engine"));
        let component =
            wasmtime::component::Component::from_binary(engine.raw(), wasm).expect("component");
        let manifest = parse_manifest_str(toml).expect("manifest");
        let identity = manifest.signature.identity_string();
        let loaded = LoadedPlugin {
            manifest,
            identity,
            component,
            origin_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        };
        PluginExtractor::new(loaded, engine, HostResources::default()).expect("adapter")
    }

    /// The plugin name every unit-test `PluginStoreData` carries, and the
    /// log target derived from it (`PluginStoreData::new`).
    pub const TEST_PLUGIN_NAME: &str = "test";
    pub const TEST_LOG_TARGET: &str = "plugin::test";

    /// The diagnostics origin matching [`TEST_PLUGIN_NAME`].
    pub const fn test_origin() -> PluginOrigin<'static> {
        PluginOrigin {
            plugin_name: TEST_PLUGIN_NAME,
            log_target: TEST_LOG_TARGET,
            display_name: TEST_PLUGIN_NAME,
        }
    }

    /// The `log` capture sink, shared with rdlp-extractor's own tests: see
    /// `rdlp_extractor::log_capture` for the process-global-buffer caveat
    /// every assertion here lives with.
    pub use rdlp_extractor::log_capture::{
        captured_count_containing, captured_count_on_target, captured_entry_containing,
        captured_logs,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `"` in the regex would break a hand-rolled `\`-only escape (it
    /// terminates the TOML string early, producing either a parse error or
    /// a silently truncated `url_regex`); this pins the round-trip through
    /// `toml::Value::String` for a pattern carrying both a literal `"` and
    /// a `\`, S1-style — the manifest that comes back out must equal the
    /// regex that went in, not merely "parse without panicking".
    // Test fixture — sync I/O is acceptable per clippy.toml's
    // disallowed-methods carve-out (c).
    #[allow(clippy::disallowed_methods)]
    #[test]
    fn url_regex_with_quote_and_backslash_round_trips() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let regex_with_quote_and_backslash = r#"^https://example\.com/"(?P<id>\d+)"$"#;
        let key = SigningKey::generate(&mut rand::rngs::OsRng);
        write_signed_plugin(
            dir.path(),
            &key,
            &SignedPluginSpec {
                url_regex: Some(regex_with_quote_and_backslash),
                ..SignedPluginSpec::stub("quote-test", b"not real wasm")
            },
        );

        let written = std::fs::read_to_string(dir.path().join("plugin.toml"))
            .unwrap_or_else(|e| panic!("read back plugin.toml: {e}"));
        let manifest = crate::manifest::parse_manifest_str(&written)
            .unwrap_or_else(|e| panic!("parse the manifest write_signed_plugin just wrote: {e}"));
        assert_eq!(
            manifest.url_regex.as_deref(),
            Some(regex_with_quote_and_backslash),
            "the regex must round-trip through TOML unchanged, quote and all"
        );
    }
}
