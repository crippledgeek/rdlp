//! Plugin system bootstrap for the rdlp-api orchestrator.
//!
//! Discovers, validates, and registers WASM plugins from
//! `Config::plugin_directories`. Fail-soft design: returns `Ok(0)` for missing
//! directories, logs warnings for individual plugin failures, and never panics.
//! A broken plugin directory **must never** block rdlp from working with
//! built-in extractors.

use anyhow::Context as _;
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::InfoExtractor;
use rdlp_extractor::ExtractorRegistry;
use rdlp_http::HttpClientFactory;
use rdlp_plugin::{
    adapter::{HostResources, PluginExtractor},
    disabled_list::read_disabled_list,
    engine::{Engine, EngineConfig},
    host::store_kv::open_host_db,
    loader::Loader,
    prompt::{AlwaysDeny, PreTrustedIdentities, Prompter},
    search_adapter::PluginSearchExtractor,
    trust_store::TrustStore,
};
use rdlp_types::Config;
use std::sync::Arc;

/// Build an [`ExtractorRegistry`] populated with built-in extractors and any
/// plugins that load cleanly from `config.plugin_directories`.
///
/// Plugin loading errors are non-fatal: each failed plugin emits a `WARN`-level
/// log message and is skipped. The returned registry always contains the
/// complete set of built-in extractors.
pub fn build_registry_with_plugins(config: &Config) -> ExtractorRegistry {
    let mut registry = ExtractorRegistry::new();

    match bootstrap_plugins(config, &mut registry) {
        Ok(count) if count > 0 => log::info!("plugin bootstrap: loaded {count} plugin(s)"),
        Ok(_) => log::debug!("plugin bootstrap: no plugins discovered"),
        Err(e) => {
            log::warn!("plugin bootstrap failed: {e:#}; continuing with built-in extractors only");
        }
    }

    registry
}

/// Inner function — isolated so the outer wrapper can catch the top-level error.
fn bootstrap_plugins(
    config: &Config,
    registry: &mut ExtractorRegistry,
) -> Result<usize, anyhow::Error> {
    if !config.load_plugins {
        return Ok(0);
    }
    if config.plugin_directories.is_empty() {
        return Ok(0);
    }

    let engine_cfg = EngineConfig {
        max_memory_bytes: config.plugin_memory_limit_mb.unwrap_or(64) as usize * 1024 * 1024,
        max_stack_bytes: config.plugin_stack_limit_mb.unwrap_or(1) as usize * 1024 * 1024,
        ..Default::default()
    };
    let engine = Arc::new(Engine::new(engine_cfg).context("wasmtime engine init")?);

    let rdlp_dir = config_dir()?.join("rdlp");
    let trust_path = rdlp_dir.join("plugin-trust.toml");
    // Single attempt — if the real trust store can't be opened, log loudly
    // (any subsequent first-install confirmations will not persist) and
    // continue with the original path; TrustStore::open returns an empty
    // in-memory store on missing files, so this rarely fails for legitimate
    // I/O reasons. The previous triple-fallback chain was confusing and
    // hid the failure mode behind a tmp file the next process never read.
    let mut trust_store = match TrustStore::open(&trust_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!(
                "plugin trust store at {} failed to open: {e}; \
                 trust decisions made this run WILL NOT PERSIST across restarts",
                trust_path.display()
            );
            return Err(e).context("trust store open");
        }
    };

    // Read the disabled-plugins list once at bootstrap. A corrupted file is
    // a hard failure — silently treating it as empty would re-activate any
    // previously-disabled plugin (security regression).
    let disabled_path = rdlp_dir.join("plugin-disabled.toml");
    let disabled: std::collections::HashSet<String> = read_disabled_list(&disabled_path)
        .with_context(|| format!("read disabled-plugin list at {}", disabled_path.display()))?
        .into_iter()
        .collect();

    // Prompter selection — conservative by default:
    //   - AlwaysDeny  : no pre-trusted publishers configured.
    //   - PreTrustedIdentities: user explicitly listed trusted publishers in
    //                           their config or via `--trust-publisher` flag.
    //
    // AlwaysDeny is the safe default: an unattended CLI run must NOT silently
    // auto-trust unknown publishers.
    let prompter: Arc<dyn Prompter> = if config.plugin_trusted_publishers.is_empty() {
        Arc::new(AlwaysDeny)
    } else {
        Arc::new(PreTrustedIdentities {
            trusted: config.plugin_trusted_publishers.clone(),
        })
    };

    // Build the shared host resources once. Each plugin's adapter
    // populates per-call capability contexts from these.
    let host_resources = build_host_resources(config)?;

    let mut loader = Loader::new(&engine, &mut trust_store, prompter);
    let mut loaded_count = 0usize;

    for dir in &config.plugin_directories {
        for outcome in loader.discover(dir) {
            match outcome {
                Ok(plugin) => {
                    if disabled.contains(&plugin.manifest.name) {
                        log::info!(
                            "plugin '{}' is in the disabled list; skipping load",
                            plugin.manifest.name
                        );
                        continue;
                    }
                    let plugin_name = plugin.manifest.name.clone();
                    match PluginExtractor::new(plugin, Arc::clone(&engine), host_resources.clone())
                    {
                        Ok(extractor) => {
                            // Extract and search are independent capabilities
                            // (D5): a search-only plugin sets
                            // `supports_extract = false` and must not appear
                            // as an `InfoExtractor` at all, so URL routing
                            // never dispatches to it. Both registrations
                            // share one `Arc<PluginExtractor>` rather than
                            // constructing the adapter twice.
                            let supports_extract = extractor.manifest.supports_extract;
                            let supports_search = extractor.manifest.supports_search;
                            let adapter = Arc::new(extractor);
                            if supports_extract {
                                registry.register(Arc::clone(&adapter) as Arc<dyn InfoExtractor>);
                            }
                            if supports_search {
                                registry.register_search(Arc::new(PluginSearchExtractor::new(
                                    Arc::clone(&adapter),
                                )));
                            }
                            log::debug!(
                                "plugin bootstrap: registered plugin '{plugin_name}' \
                                 (extract={supports_extract}, search={supports_search})"
                            );
                            loaded_count += 1;
                        }
                        Err(e) => {
                            log::warn!("plugin '{plugin_name}' adapter init failed: {e}");
                        }
                    }
                }
                Err((plugin_dir, e)) => {
                    log::warn!("plugin {} failed to load: {e}", plugin_dir.display());
                }
            }
        }
    }

    Ok(loaded_count)
}

/// Build the shared per-host resources that the plugin adapters use to
/// populate per-call capability contexts. Failure here is non-fatal at the
/// per-resource level — the corresponding capability is simply not granted.
#[allow(clippy::unnecessary_wraps)] // may return Err in future when resource init can fail
fn build_host_resources(config: &Config) -> anyhow::Result<HostResources> {
    let cookie_jar = Arc::new(SimpleCookieJar::new());
    let raw_jar = cookie_jar.jar();
    let fetch_client =
        Some(HttpClientFactory::from_rdlp_config(config).build_with_cookies(raw_jar));

    // sled DB for host:store-kv. Sited under the rdlp config dir so it's
    // user-private and persists across runs.
    let kv_db = match config_dir() {
        Ok(base) => {
            let kv_path = base.join("rdlp").join("plugin-kv");
            match open_host_db(&kv_path) {
                Ok(db) => Some(Arc::new(db)),
                Err(e) => {
                    log::warn!(
                        "plugin store-kv at {}: {e}; the host:store-kv capability will be denied",
                        kv_path.display()
                    );
                    None
                }
            }
        }
        Err(e) => {
            log::warn!("no config dir for plugin store-kv: {e}");
            None
        }
    };

    Ok(HostResources {
        fetch_client,
        cookie_jar: Some(cookie_jar),
        kv_db,
        // Production never injects fixtures; the field exists for the
        // golden-test harness in `crates/rdlp-plugin/tests/`.
        fetch_fixtures: None,
    })
}

/// Resolve the platform config directory.
///
/// Falls back to `$HOME/.config` when `dirs::config_dir()` returns `None`
/// (unusual on Linux/macOS; possible in minimal container environments).
fn config_dir() -> anyhow::Result<std::path::PathBuf> {
    // `dirs::config_dir()` returns `None` on platforms without a concept of a
    // config directory. We fall back to `$HOME/.config` for UNIX compatibility.
    if let Some(d) = dirs::config_dir() {
        return Ok(d);
    }
    let home = std::env::var("HOME").context("HOME not set and no config dir available")?;
    Ok(std::path::PathBuf::from(home).join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use rdlp_plugin::test_support::{
        SignedPluginSpec, trusted_identity_for, with_isolated_config_dir, write_signed_plugin,
    };

    /// The Task 1 fixture shared with `rdlp-plugin`'s own tests: a real,
    /// WASI-free component implementing `extract` and `search`, so the
    /// manifest's `supports_extract`/`supports_search` flags are the only
    /// thing under test here — the component itself always answers both.
    const FIXTURE_0_5_0: &str = "../rdlp-plugin/tests/fixtures/example-extractor-0.5.0/plugin.wasm";

    /// Which of the plugin's two independent capabilities (D5) a test
    /// manifest declares. A bare `(bool, bool)` positional pair is exactly
    /// the ambiguous-call-site shape `limit-function-arguments` flags —
    /// `config_with_signed_plugin(false, true)` reads no better here than
    /// it would with the arguments swapped.
    struct Capabilities {
        extract: bool,
        search: bool,
    }

    /// Sign the fixture component under the given capability flags into a
    /// fresh temp plugin directory, and a `Config` pre-trusting the signer
    /// so `bootstrap_plugins` loads it without an interactive prompt.
    /// Returns the `TempDir` guard alongside the `Config` so the caller
    /// keeps the plugin directory alive for the duration of the test
    /// instead of it being deleted the moment this function returns.
    fn config_with_signed_plugin(capabilities: &Capabilities) -> (Config, tempfile::TempDir) {
        let wasm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_0_5_0);
        // Test fixture — sync I/O is acceptable per clippy.toml's
        // disallowed-methods carve-out (c); this runs in test setup, never
        // on an async hot path.
        #[allow(clippy::disallowed_methods)]
        let wasm = std::fs::read(&wasm_path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", wasm_path.display()));

        let tempdir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let key = SigningKey::generate(&mut OsRng);
        write_signed_plugin(
            &tempdir.path().join("example"),
            &key,
            &SignedPluginSpec {
                name: "example",
                version: "0.1.0",
                wit_version: "0.5.0",
                matches: &["https://example.com/*"],
                url_regex: None,
                priority: 150,
                claims_override: &[],
                capabilities: &[],
                supports_extract: capabilities.extract,
                supports_search: capabilities.search,
                wasm: &wasm,
            },
        );

        let config = Config {
            plugin_directories: vec![tempdir.path().to_path_buf()],
            plugin_trusted_publishers: vec![trusted_identity_for(&key)],
            ..Default::default()
        };
        (config, tempdir)
    }

    #[test]
    fn search_only_plugin_registers_only_as_a_search_extractor() {
        with_isolated_config_dir(|_config_dir| {
            let (config, _tempdir) = config_with_signed_plugin(&Capabilities {
                extract: false,
                search: true,
            });
            let registry = build_registry_with_plugins(&config);
            assert!(
                !registry.list_extractors().contains(&"example"),
                "supports_extract = false must not register as an InfoExtractor"
            );
            assert!(
                registry.list_search_extractors().contains(&"example"),
                "supports_search = true must register as a SearchExtractor"
            );
        });
    }

    #[test]
    fn plugin_supporting_both_registers_in_both_lists() {
        with_isolated_config_dir(|_config_dir| {
            let (config, _tempdir) = config_with_signed_plugin(&Capabilities {
                extract: true,
                search: true,
            });
            let registry = build_registry_with_plugins(&config);
            assert!(
                registry.list_extractors().contains(&"example"),
                "supports_extract = true must register as an InfoExtractor"
            );
            assert!(
                registry.list_search_extractors().contains(&"example"),
                "supports_search = true must register as a SearchExtractor"
            );
        });
    }

    /// The extract-only negative: a plugin that does not declare
    /// `supports_search` must not appear in `list_search_extractors()`,
    /// mirroring the search-only test's own negative assertion.
    #[test]
    fn extract_only_plugin_is_absent_from_search_extractors() {
        with_isolated_config_dir(|_config_dir| {
            let (config, _tempdir) = config_with_signed_plugin(&Capabilities {
                extract: true,
                search: false,
            });
            let registry = build_registry_with_plugins(&config);
            assert!(
                registry.list_extractors().contains(&"example"),
                "supports_extract = true must register as an InfoExtractor"
            );
            assert!(
                !registry.list_search_extractors().contains(&"example"),
                "supports_search = false must not register as a SearchExtractor"
            );
        });
    }
}
