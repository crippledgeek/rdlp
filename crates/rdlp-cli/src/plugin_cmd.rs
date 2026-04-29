//! `rdlp plugin <subcommand>` — plugin management commands.

// Explicit `#[path]` because lib.rs loads this file via `#[path = "plugin_cmd.rs"]`,
// which makes implicit submodule lookup resolve from `src/` rather than `src/plugin_cmd/`.
#[path = "plugin_cmd/build_from_ytdlp.rs"]
mod build_from_ytdlp;
pub use build_from_ytdlp::run as run_build_from_ytdlp;

use anyhow::{Context, Result};
use rdlp_plugin::manifest::validate_plugin_name;
use rdlp_plugin::trust_store::TrustStore;
use rdlp_types::Config;
use std::path::PathBuf;

/// Reject path-traversing or otherwise unsafe plugin names BEFORE any
/// `dir.join(name)` / `remove_dir_all` operation. Gives the user a clear
/// error message rather than silently mis-resolving the path.
fn require_valid_name(name: &str) -> Result<()> {
    validate_plugin_name(name).map_err(|e| anyhow::anyhow!("invalid plugin name '{name}': {e}"))
}

/// Return the rdlp config directory (`~/.config/rdlp` on most platforms).
pub fn config_path() -> Result<PathBuf> {
    Ok(dirs::config_dir().context("no config dir")?.join("rdlp"))
}

/// Return the path to the plugin trust store file.
pub fn trust_store_path() -> Result<PathBuf> {
    Ok(config_path()?.join("plugin-trust.toml"))
}

/// Return the path to the plugin disabled-list file.
pub fn disabled_list_path() -> Result<PathBuf> {
    Ok(config_path()?.join("plugin-disabled.toml"))
}

/// `rdlp plugin list` — list all installed plugins with their trust state.
pub async fn run_list(config: &Config) -> Result<()> {
    let trust = TrustStore::open(trust_store_path()?)?;
    if config.plugin_directories.is_empty() {
        println!("(no plugin directories configured; set Config::plugin_directories)");
        return Ok(());
    }
    let mut found = 0usize;
    for dir in &config.plugin_directories {
        #[allow(clippy::disallowed_methods)] // startup/CLI commands — sync I/O is acceptable
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }
            match rdlp_plugin::manifest::parse_manifest_file(&manifest_path) {
                Ok(m) => {
                    found += 1;
                    let trust_state = trust
                        .lookup(&m.name)
                        .map(|e| e.identity.clone())
                        .unwrap_or_else(|| "(untrusted)".into());
                    println!(
                        "{}  v{}  identity={}  caps=[{}]",
                        m.name,
                        m.version,
                        trust_state,
                        m.capabilities.join(", ")
                    );
                }
                Err(e) => {
                    println!("{}  ERROR: {e}", path.display());
                }
            }
        }
    }
    if found == 0 {
        println!("(no plugins installed)");
    }
    Ok(())
}

/// `rdlp plugin info <name>` — show detailed info for a specific plugin.
pub async fn run_info(name: &str, config: &Config) -> Result<()> {
    require_valid_name(name)?;
    for dir in &config.plugin_directories {
        let plugin_dir = dir.join(name);
        let manifest_path = plugin_dir.join("plugin.toml");
        if !manifest_path.exists() {
            continue;
        }
        let m = rdlp_plugin::manifest::parse_manifest_file(&manifest_path)?;
        let trust = TrustStore::open(trust_store_path()?)?;
        let trust_entry = trust.lookup(name);

        println!("Plugin: {}", m.name);
        println!("Version: {}", m.version);
        println!("WIT version: {}", m.wit_version);
        println!("Priority: {}", m.priority);
        println!("Match patterns:");
        for p in &m.matches {
            println!("  - {p}");
        }
        if !m.claims_override.is_empty() {
            println!("Claims override:");
            for h in &m.claims_override {
                println!("  - {h}");
            }
        }
        println!("Capabilities: {}", m.capabilities.join(", "));
        match trust_entry {
            Some(e) => {
                println!("Trust state: TRUSTED ({})", e.identity);
                let caps: Vec<_> = e.approved_capabilities.iter().cloned().collect();
                println!("Approved capabilities: {}", caps.join(", "));
            }
            None => println!("Trust state: UNTRUSTED (will prompt on next load)"),
        }
        println!("Origin: {}", plugin_dir.display());
        return Ok(());
    }
    anyhow::bail!("plugin '{name}' not found in any configured plugin directory")
}

/// `rdlp plugin retrust <name>` — clear the recorded identity so the next load re-prompts.
pub async fn run_retrust(name: &str) -> Result<()> {
    let mut trust = TrustStore::open(trust_store_path()?)?;
    if trust.lookup(name).is_some() {
        trust.forget(name)?;
        println!("Trust forgotten for plugin '{name}'. Next load will prompt for approval.");
    } else {
        println!("Plugin '{name}' was not in the trust store; nothing to forget.");
    }
    Ok(())
}

/// `rdlp plugin disable <name>` — add the plugin to the disabled list.
pub async fn run_disable(name: &str) -> Result<()> {
    require_valid_name(name)?;
    let path = disabled_list_path()?;
    // Fail loudly on a corrupted disabled list. Silently treating it as
    // empty would re-enable a previously-blocked plugin — a security
    // regression we explicitly do not want.
    let mut current = read_disabled(&path)
        .with_context(|| format!("read disabled-plugin list at {}", path.display()))?;
    if current.contains(&name.to_string()) {
        println!("Plugin '{name}' is already disabled.");
        return Ok(());
    }
    current.push(name.to_string());
    current.sort();
    write_disabled(&path, &current)?;
    println!("Plugin '{name}' disabled. It will be skipped on next load.");
    Ok(())
}

/// `rdlp plugin enable <name>` — remove the plugin from the disabled list.
pub async fn run_enable(name: &str) -> Result<()> {
    require_valid_name(name)?;
    let path = disabled_list_path()?;
    let mut current = read_disabled(&path)
        .with_context(|| format!("read disabled-plugin list at {}", path.display()))?;
    let before = current.len();
    current.retain(|n| n != name);
    if current.len() == before {
        println!("Plugin '{name}' was not disabled.");
        return Ok(());
    }
    write_disabled(&path, &current)?;
    println!("Plugin '{name}' re-enabled.");
    Ok(())
}

/// `rdlp plugin uninstall <name>` — delete the plugin directory and forget its trust entry.
pub async fn run_uninstall(name: &str, config: &Config) -> Result<()> {
    require_valid_name(name)?;
    let mut found = false;
    for dir in &config.plugin_directories {
        let plugin_dir = dir.join(name);
        if plugin_dir.exists() {
            #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
            std::fs::remove_dir_all(&plugin_dir)
                .with_context(|| format!("removing {}", plugin_dir.display()))?;
            println!("Removed plugin directory {}", plugin_dir.display());
            found = true;
        }
    }
    let mut trust = TrustStore::open(trust_store_path()?)?;
    if trust.lookup(name).is_some() {
        trust.forget(name)?;
        println!("Forgot trust entry for '{name}'.");
        found = true;
    }
    if !found {
        anyhow::bail!("plugin '{name}' not installed");
    }
    Ok(())
}

// The disabled-list TOML shape and reader live in `rdlp_plugin::disabled_list`
// so that orchestrator bootstrap (in rdlp-api) can read the same file the
// CLI writes without taking a dependency on rdlp-cli.
use rdlp_plugin::disabled_list::{DisabledList, read_disabled_list as read_disabled};

fn write_disabled(path: &PathBuf, list: &[String]) -> Result<()> {
    let dl = DisabledList {
        disabled: list.to_vec(),
    };
    let s = toml::to_string_pretty(&dl)?;
    if let Some(parent) = path.parent() {
        #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
        std::fs::create_dir_all(parent)?;
    }
    // Atomic write: tmp + rename in the same dir, mirroring TrustStore::persist.
    // Plain `std::fs::write` would corrupt the file on crash mid-write.
    let mut tmp_path = path.clone();
    let tmp_name = format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin-disabled")
    );
    tmp_path.set_file_name(tmp_name);
    #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
    std::fs::write(&tmp_path, s)?;
    #[cfg(unix)]
    {
        // Restrict mode to user-only — mirrors the trust store.
        use std::os::unix::fs::PermissionsExt;
        #[allow(clippy::disallowed_methods)]
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }
    #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}
