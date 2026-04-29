//! Plugin system bootstrap for the rdlp-api orchestrator.
//!
//! Discovers, validates, and registers WASM plugins from
//! `Config::plugin_directories`. Fail-soft design: returns `Ok(0)` for missing
//! directories, logs warnings for individual plugin failures, and never panics.
//! A broken plugin directory **must never** block rdlp from working with
//! built-in extractors.

use anyhow::Context as _;
use rdlp_extractor::ExtractorRegistry;
use rdlp_plugin::{
    adapter::PluginExtractor,
    engine::{Engine, EngineConfig},
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

    let trust_path = config_dir()?.join("rdlp").join("plugin-trust.toml");
    let mut trust_store = TrustStore::open(&trust_path).unwrap_or_else(|e| {
        log::warn!("plugin trust store at {trust_path:?}: {e}; using in-memory store");
        // Fall back to an in-memory store at a non-existent path — TrustStore
        // will re-open from disk on the next process start.
        TrustStore::open(trust_path).unwrap_or_else(|_| {
            // If the real path fails too, use a temp path that will never be read.
            TrustStore::open(std::env::temp_dir().join("rdlp-plugin-trust-fallback.toml"))
                .expect("tmp trust store always succeeds")
        })
    });

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

    let mut loader = Loader::new(&engine, &mut trust_store, prompter);
    let mut loaded_count = 0usize;

    for dir in &config.plugin_directories {
        for outcome in loader.discover(dir) {
            match outcome {
                Ok(loaded) => match PluginExtractor::new(loaded, Arc::clone(&engine)) {
                    Ok(extractor) => {
                        log::debug!(
                            "plugin bootstrap: registered plugin {:?}",
                            dir.file_name().unwrap_or_default()
                        );
                        registry.register(Arc::new(extractor));
                        loaded_count += 1;
                    }
                    Err(e) => {
                        log::warn!("plugin adapter init for {dir:?}: {e}");
                    }
                },
                Err((plugin_dir, e)) => {
                    log::warn!("plugin {plugin_dir:?} failed to load: {e}");
                }
            }
        }
    }

    Ok(loaded_count)
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
