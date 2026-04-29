//! Plugin system bootstrap for the rdlp-api orchestrator.
//!
//! Discovers, validates, and registers WASM plugins from
//! `Config::plugin_directories`. Fail-soft design: returns `Ok(0)` for missing
//! directories, logs warnings for individual plugin failures, and never panics.
//! A broken plugin directory **must never** block rdlp from working with
//! built-in extractors.

use anyhow::Context as _;
use rdlp_cookies::SimpleCookieJar;
use rdlp_extractor::ExtractorRegistry;
use rdlp_http::HttpClientFactory;
use rdlp_plugin::{
    adapter::{HostResources, PluginExtractor},
    disabled_list::read_disabled_list,
    engine::{Engine, EngineConfig},
    host::store_kv::open_host_db,
    loader::Loader,
    prompt::{AlwaysDeny, PreTrustedIdentities, Prompter},
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
pub(crate) fn build_registry_with_plugins(config: &Config) -> ExtractorRegistry {
    let mut registry = ExtractorRegistry::new();

    match bootstrap_plugins(config, &mut registry) {
        Ok(count) if count > 0 => log::info!("plugin bootstrap: loaded {count} plugin(s)"),
        Ok(_) => log::debug!("plugin bootstrap: no plugins discovered"),
        Err(e) => {
            log::warn!("plugin bootstrap failed: {e:#}; continuing with built-in extractors only")
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
                "plugin trust store at {trust_path:?} failed to open: {e}; \
                 trust decisions made this run WILL NOT PERSIST across restarts"
            );
            return Err(e).context("trust store open");
        }
    };

    // Read the disabled-plugins list once at bootstrap. A corrupted file is
    // a hard failure — silently treating it as empty would re-activate any
    // previously-disabled plugin (security regression).
    let disabled_path = rdlp_dir.join("plugin-disabled.toml");
    let disabled: std::collections::HashSet<String> = read_disabled_list(&disabled_path)
        .with_context(|| {
            format!("read disabled-plugin list at {}", disabled_path.display())
        })?
        .into_iter()
        .collect();

    // Prompter selection — conservative by default:
    //   - AlwaysDeny  : no pre-trusted publishers configured.
    //   - PreTrustedIdentities: user explicitly listed trusted publishers in
    //                           their config or via `--trust-publisher` flag.
    //
    // AlwaysDeny is the safe default: an unattended CLI run must NOT silently
    // auto-trust unknown publishers.
    let prompter: Arc<dyn Prompter> = if !config.plugin_trusted_publishers.is_empty() {
        Arc::new(PreTrustedIdentities {
            trusted: config.plugin_trusted_publishers.clone(),
        })
    } else {
        Arc::new(AlwaysDeny)
    };

    // Build the shared host resources once. Each plugin's adapter
    // populates per-call capability contexts from these.
    let host_resources = build_host_resources(config)?;

    let mut loader = Loader::new(&engine, &mut trust_store, prompter);
    let mut loaded_count = 0usize;

    for dir in &config.plugin_directories {
        for outcome in loader.discover(dir) {
            match outcome {
                Ok(loaded) => {
                    if disabled.contains(&loaded.manifest.name) {
                        log::info!(
                            "plugin '{}' is in the disabled list; skipping load",
                            loaded.manifest.name
                        );
                        continue;
                    }
                    let plugin_name = loaded.manifest.name.clone();
                    match PluginExtractor::new(
                        loaded,
                        Arc::clone(&engine),
                        host_resources.clone(),
                    ) {
                        Ok(extractor) => {
                            log::debug!(
                                "plugin bootstrap: registered plugin '{plugin_name}'"
                            );
                            registry.register(Arc::new(extractor));
                            loaded_count += 1;
                        }
                        Err(e) => {
                            log::warn!("plugin '{plugin_name}' adapter init failed: {e}");
                        }
                    }
                }
                Err((plugin_dir, e)) => {
                    log::warn!("plugin {plugin_dir:?} failed to load: {e}");
                }
            }
        }
    }

    Ok(loaded_count)
}

/// Build the shared per-host resources that the plugin adapters use to
/// populate per-call capability contexts. Failure here is non-fatal at the
/// per-resource level — the corresponding capability is simply not granted.
fn build_host_resources(config: &Config) -> anyhow::Result<HostResources> {
    let cookie_jar = Arc::new(SimpleCookieJar::new());
    let raw_jar = cookie_jar.jar();
    let fetch_client = Some(
        HttpClientFactory::from_rdlp_config(config).build_with_cookies(raw_jar),
    );

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
