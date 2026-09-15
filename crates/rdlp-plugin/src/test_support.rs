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
use sha2::{Digest, Sha256};

/// The committed 0.5.0 example component (`tests/fixtures/example-extractor-0.5.0`):
/// WASI-free, no capabilities, implements `extract` and `search`, instantiates
/// on the host world — see its README. The one copy of the fixture's bytes
/// every test in this crate and `rdlp-api` loads.
#[doc(hidden)]
pub const EXAMPLE_0_5_0_WASM: &[u8] =
    include_bytes!("../tests/fixtures/example-extractor-0.5.0/plugin.wasm");

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
#[doc(hidden)]
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
    /// The compiled component bytes to sign and write.
    pub wasm: &'a [u8],
}

/// Write a signed `plugin.toml` + `plugin.wasm` into `dir` so
/// `Loader::discover` accepts it. The one signing mechanism every plugin
/// integration test across the workspace shares.
#[doc(hidden)]
pub fn write_signed_plugin(dir: &Path, key: &SigningKey, spec: &SignedPluginSpec<'_>) {
    write_fixture_file(dir, "plugin.wasm", spec.wasm);

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
    let claims_str = spec
        .claims_override
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    // A hand-rolled `\`-only escape would mishandle a `"` in the regex
    // (producing an invalid manifest); `toml::Value::String`'s `Display`
    // emits a properly TOML-escaped string literal — quotes included — for
    // any Rust `&str`, so the caller never has to pre-escape for TOML.
    let url_regex_line = spec
        .url_regex
        .map(|r| format!("url_regex = {}\n", toml::Value::String(r.to_string())))
        .unwrap_or_default();
    let toml_placeholder = format!(
        r#"
name = "{name}"
version = "{version}"
wit_version = "{wit_version}"
matches = [{match_str}]
{url_regex_line}priority = {priority}
claims_override = [{claims_str}]
supports_search = {supports_search}
supports_extract = {supports_extract}
capabilities = [{cap_str}]

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#,
        name = spec.name,
        version = spec.version,
        wit_version = spec.wit_version,
        priority = spec.priority,
        supports_search = spec.supports_search,
        supports_extract = spec.supports_extract,
    );

    let m = crate::manifest::parse_manifest_str(&toml_placeholder)
        .unwrap_or_else(|e| panic!("parse manifest: {e}"));
    let mut buf = crate::manifest::canonical_bytes(&m);
    buf.extend_from_slice(spec.wasm);
    let sig = key.sign(&buf);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let final_toml = toml_placeholder.replace("PLACEHOLDER", &sig_b64);
    write_fixture_file(dir, "plugin.toml", final_toml.as_bytes());
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

/// The trust-store identity string for a signing key: `ed25519:<hex of the
/// SHA-256 over the base64-encoded public key>`, matching
/// `Signature::identity_string()`'s post-MVP-hardening format. Every test
/// that pre-trusts a freshly generated key (rather than going through the
/// interactive prompter) needs this to populate
/// `Config::plugin_trusted_publishers`.
#[doc(hidden)]
#[must_use]
pub fn trusted_identity_for(key: &SigningKey) -> String {
    let pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    format!(
        "ed25519:{}",
        hex::encode(Sha256::digest(pubkey_b64.as_bytes()))
    )
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

    /// The 0.5.0 fixture wrapped in an adapter, on a manifest with no
    /// `search_site` and no override claim of either kind.
    pub fn fixture_extractor() -> PluginExtractor {
        fixture_extractor_with_manifest(FIXTURE_MANIFEST)
    }

    /// The 0.5.0 fixture on `toml` — for tests that need a manifest field
    /// (a `search_site`, an override claim) the default fixture lacks.
    pub fn fixture_extractor_with_manifest(toml: &str) -> PluginExtractor {
        let engine = Arc::new(Engine::new(EngineConfig::default()).expect("engine"));
        let component =
            wasmtime::component::Component::from_binary(engine.raw(), super::EXAMPLE_0_5_0_WASM)
                .expect("component");
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
        }
    }

    /// `(target, message)` pairs captured from the `log` facade.
    pub type LogEntries = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    /// Minimal `log::Log` sink so a test can assert a refusal was reported
    /// to the plugin's own log target. Mirrors the capturing-logger harness
    /// in `rdlp-cookies`; `log::set_logger` accepts one logger per process,
    /// so the buffer is process-global and never cleared — each assertion
    /// looks for its own distinctive message instead.
    struct CapturingLogger {
        entries: LogEntries,
    }

    impl log::Log for CapturingLogger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            self.entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((record.target().to_string(), record.args().to_string()));
        }
        fn flush(&self) {}
    }

    /// The process-global capture buffer, installing the logger on first use.
    pub fn captured_logs() -> LogEntries {
        static CAPTURED: std::sync::OnceLock<LogEntries> = std::sync::OnceLock::new();
        Arc::clone(CAPTURED.get_or_init(|| {
            let entries: LogEntries = Arc::new(std::sync::Mutex::new(Vec::new()));
            let logger: &'static CapturingLogger = Box::leak(Box::new(CapturingLogger {
                entries: Arc::clone(&entries),
            }));
            log::set_logger(logger).expect("no other logger in the rdlp-plugin lib test binary");
            log::set_max_level(log::LevelFilter::Warn);
            entries
        }))
    }

    /// First captured entry whose message contains `needle`, cloned out so
    /// the lock is released before any assertion panics.
    pub fn captured_entry_containing(logs: &LogEntries, needle: &str) -> (String, String) {
        let entries = logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries
            .iter()
            .find(|(_, m)| m.contains(needle))
            .cloned()
            .unwrap_or_else(|| panic!("no entry containing {needle:?} among {entries:?}"))
    }
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
                name: "quote-test",
                version: "0.1.0",
                wit_version: "0.5.0",
                matches: &["https://example.com/*"],
                url_regex: Some(regex_with_quote_and_backslash),
                priority: 150,
                claims_override: &[],
                capabilities: &[],
                supports_extract: true,
                supports_search: false,
                wasm: b"not real wasm",
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
